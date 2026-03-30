# CPersona Memory System — MCP Server Design

> **Status:** Implemented (v2.4 RRF complete, v2.5 Recency Boost planned)
> **Related:** `MCP_PLUGIN_ARCHITECTURE.md`, `ARCHITECTURE.md` Section 3
> **MCP Server ID:** `memory.cpersona`
> **Companion Server:** `tool.embedding` (pluggable vector embedding)

---

## 1. Background

### 1.1 Evolution History

| Version | Project | Storage | Search | Memory Extraction | Status |
|---------|---------|---------|--------|-------------------|--------|
| CPersona 2.0/2.1 | ai_karin | SQLite (WAL, FTS5, vector) | FTS5 + cosine similarity + semantic cache | LLM-powered (DeepSeek Reasoner): profile extraction, episode archival | Reference implementation |
| CPersona 2.2 | ClotoCore | plugin_data (key-value via SAL) | `LIKE '%keyword%'` | None | Deprecated (Rust plugin) |
| CPersona 2.3 | ClotoCore | Dedicated SQLite (`data/cpersona.db`) | FTS5 + vector (pluggable) | LLM-powered (Phase 3) + anti-contamination (Phase 4) + background task queue (Phase 5) | Complete |
| CPersona 2.3.1 | ClotoCore | Same as 2.3 | Same as 2.3 | 2.3 + JSONL export/import, pre-computed summary/keywords in archive_episode, Claude Code integration | Complete |
| CPersona 2.3.2 | ClotoCore | Same as 2.3 | Same as 2.3 | 2.3.1 + Memory Confidence Score (recall output enriched with confidence metadata: cosine, age, geometric mean score) | Complete |
| CPersona 2.3.3–2.3.6 | ClotoCore | Same as 2.3 | Same as 2.3 | Task decay, deep recall, FTS5 trigram, adaptive scan, remote vector search | Complete |
| CPersona 2.3.7 | ClotoCore | Same as 2.3 | Same as 2.3 | Auto-calibration of VECTOR_MIN_SIMILARITY (z-score of null cosine distribution, label-free) | **Complete** |
| CPersona 2.4 | ClotoCore | Dedicated SQLite (`data/cpersona.db`) | FTS5 + vector + RRF reranking | Reciprocal Rank Fusion as alternative recall mode (RECALL_MODE=rrf). Vector and FTS5 run independently, merged by RRF score | **Complete** |
| CPersona 2.5 | ClotoCore | Dedicated SQLite (`data/cpersona.db`) | FTS5 + vector + recency-weighted scoring | LLM-powered + anti-contamination + gated recency boost (reuses v2.3.2 normalization) | Planned |
| CPersona 3.0 | Standalone | SQLite + graph tables (nodes/edges) | Cascade + graph traversal (BFS) + bi-temporal | MIT license, PyPI packaging, memory evolution (full) | Planned |

### 1.2 Capabilities Lost in 2.2 (restored in 2.3)

CPersona 2.2 was a deliberate simplification for the initial ClotoCore port.
The following capabilities were dropped and subsequently restored in 2.3:

| Capability | CPersona 2.1 | CPersona 2.2 | CPersona 2.3 | Impact |
|------------|----------------------|-----------------------|----------|--------|
| **LLM Profile Extraction** | DeepSeek Reasoner | None | Cerebras (Phase 3) | Restored |
| **Episode Archival** | Summarization + keywords | None | LLM-powered (Phase 3) | Restored |
| **FTS5 Full-Text Search** | FTS5 AND matching | `LIKE '%keyword%'` | FTS5 (Phase 1) | Restored |
| **Vector Search** | MiniLM, cosine similarity | None | Pluggable embedding (Phase 2) | Restored |
| **Anti-Contamination** | None | None | Memory boundary markers, timestamp annotations, anti-hallucination guardrails (Phase 4) | **New** |
| **Semantic Cache** | ≥0.95 similarity cache | None | None | Not yet restored |
| **Background Task Queue** | DB-persisted with crash recovery | None | DB-persisted, crash-recoverable (Phase 5) | Restored |
| **Cross-Scope Sharing** | Per-user, per-guild | Per-agent flat store | Per-agent flat store | Not yet restored |

### 1.3 Design Goals

1. **Restore 2.1 search quality** — FTS5 + vector search (pluggable embedding provider)
2. **Prepare for 2.1 memory extraction** — Schema supports profiles and episodes from day one
3. **Dedicated storage** — Independent SQLite file, no dependency on kernel's plugin_data table
4. **Pluggable embedding** — Decoupled from any specific model; provider selected via configuration
5. **Lightweight footprint** — CPersona MCP server itself stays ~40MB; heavy models live elsewhere
6. **Anti-contamination** — Prevent memory-induced hallucination via temporal annotations and boundary markers

---

## 2. Architecture

### 2.1 Two-Server Design

```
┌──────────────────────────────────────────────────────────────┐
│  Kernel (Rust)                                                │
│                                                                │
│  system.rs: run_agentic_loop()                                │
│    ├── Memory Resolver → find MCP server with store/recall    │
│    ├── recall(agent_id, query, limit)                         │
│    │     └─ MCP call_tool("recall", {...})                    │
│    ├── [agentic loop / consensus]                             │
│    └── store(agent_id, message)                               │
│          └─ MCP call_tool("store", {...})                     │
└──────────────┬────────────────────────┬──────────────────────┘
               │ stdio                  │ stdio
               ▼                        ▼
┌──────────────────────┐  ┌────────────────────────────────────┐
│  memory.cpersona           │  │  tool.embedding                      │
│  (~40MB)             │  │  (~40-490MB depending on provider) │
│                      │  │                                    │
│  Tools (13):         │  │  Tools (5):                        │
│  - store             │  │  - embed (batch, max 100)          │
│  - recall            │  │  - index (namespace + vectors)     │
│  - update_profile    │  │  - search (similarity query)       │
│  - archive_episode   │  │  - remove (by ID)                  │
│  - list_memories     │  │  - purge (by namespace)            │
│  - list_episodes     │  │                                    │
│  - delete_memory     │  │  Providers:                        │
│  - delete_episode    │  │  - onnx_miniml (local, ~490MB)     │
│  - delete_agent_data │  │  - api_openai  (remote, ~40MB)    │
│  - calibrate_threshold│ │                                    │
│  - get_queue_status  │  │  HTTP: localhost:PORT              │
│  - export_memories   │  │  /embed /index /search /remove     │
│  - import_memories   │  │  /purge                            │
│                      │  │  Embedding Cache:                  │
│  DB: cpersona.db     │  │  LRU (256 entries) + TTL (300s)   │
│  (SQLite, FTS5)      │  │                                    │
│                      │  │                                    │
│  Task Queue ─────────┤  │                                    │
│  (DB-persisted FIFO) │  │                                    │
│                      │  │                                    │
│  Embedding Client ───┼──┤                                    │
│  (http/api/none)     │  │                                    │
│                      │  │                                    │
│  LLM Proxy ──────────┼──→  Kernel LLM Proxy (port 8082)     │
│  (profile/episode)   │  │                                    │
└──────────────────────┘  └────────────────────────────────────┘
```

### 2.2 Embedding Communication

MCP servers cannot call each other directly (stdio is kernel-only). The embedding
server exposes a **lightweight HTTP endpoint** alongside MCP stdio for internal use.

| CPersona Embedding Mode | How it works | CPersona Memory | Embedding Server |
|---------------------|-------------|-------------|-----------------|
| `http` | CPersona calls embedding server's HTTP endpoint | ~40MB | Required (~40-490MB) |
| `api` | CPersona calls external API directly (OpenAI-compatible) | ~40MB | Not required |
| `none` | Vector search disabled (FTS5 + keyword only) | ~40MB | Not required |

### 2.3 Embedding Cache

CPersona maintains an in-process **LRU cache with TTL** for embedding queries to
avoid redundant HTTP/API calls.

| Parameter | Default | Env Variable |
|-----------|---------|--------------|
| Capacity | 256 entries | `CPERSONA_EMBEDDING_CACHE_SIZE` |
| TTL | 300 seconds | `CPERSONA_EMBEDDING_CACHE_TTL` |

**Behavior:**
- **Single-text queries** (`len(texts) == 1`): cache lookup → on hit, return immediately; on miss, fetch + cache result
- **Batch queries** (`len(texts) > 1`): bypass cache entirely (avoids partial-cache complexity)
- Cache key: SHA-256 hash of input text (first 16 hex chars)
- Eviction: LRU order when capacity exceeded; expired entries removed on access

### 2.4 LLM Proxy Integration

CPersona uses the kernel's LLM proxy for memory extraction tasks (`update_profile`,
`archive_episode`). This avoids embedding LLM client logic in the MCP server.

| Parameter | Default | Env Variable |
|-----------|---------|--------------|
| Proxy URL | `http://127.0.0.1:8082/v1/chat/completions` | `CPERSONA_LLM_PROXY_URL` |
| Provider | `cerebras` | `CPERSONA_LLM_PROVIDER` |
| Model | `gpt-oss-120b` | `CPERSONA_LLM_MODEL` |

**Protocol:** OpenAI-compatible chat completion with custom `X-LLM-Provider` header.
Timeout: 60 seconds. On failure, returns `None` — callers implement fallback logic
(simple concatenation for profiles, word-frequency keywords for episodes).

---

## 3. MCP Tool Definitions

### 3.1 store

Store a message in agent memory.

```json
{
  "name": "store",
  "description": "Store a message in agent memory for future recall.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": {
        "type": "string",
        "description": "Agent identifier"
      },
      "message": {
        "type": "object",
        "description": "ClotoMessage to store (id, content, source, timestamp, metadata)",
        "properties": {
          "id": { "type": "string" },
          "content": { "type": "string" },
          "source": { "type": "object" },
          "timestamp": { "type": "string" },
          "metadata": { "type": "object" }
        },
        "required": ["content"]
      }
    },
    "required": ["agent_id", "message"]
  }
}
```

**Response:** `{"ok": true}` or `{"ok": true, "skipped": true, "reason": "..."}` or `{"error": "..."}`

**Behavior:**
1. If `content` is empty, skip and return `{"ok": true, "skipped": true, "reason": "empty content"}`
2. If `message.id` (msg_id) is provided, check for duplicate `(agent_id, msg_id)` pair — if found, return `{"ok": true, "skipped": true, "reason": "duplicate msg_id"}`
3. If embedding provider is available, compute embedding and store it
4. Insert message into `memories` table
5. Return immediately (no LLM call)

### 3.2 recall

Recall relevant memories for a query.

```json
{
  "name": "recall",
  "description": "Recall relevant memories for a query using multi-strategy search.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": {
        "type": "string",
        "description": "Agent identifier"
      },
      "query": {
        "type": "string",
        "description": "Search query (empty string returns recent memories)"
      },
      "limit": {
        "type": "integer",
        "description": "Maximum number of memories to return",
        "default": 10
      }
    },
    "required": ["agent_id", "query"]
  }
}
```

**Response:**
```json
{
  "messages": [
    {
      "id": "...",
      "content": "[Memory from 2026-03-13 14:30 JST] ...",
      "source": {"User": "..."},
      "timestamp": "2026-03-13T14:30:00+09:00",
      "confidence": {
        "cosine": 0.55,
        "age_hours": 120.0,
        "score": 0.59
      }
    }
  ]
}
```

The `confidence` field is included when `CPERSONA_CONFIDENCE_ENABLED=true` (v2.3.2+).
When disabled, the field is omitted and the response format is identical to v2.3.1.

- `cosine`: Raw cosine similarity from vector search (0.0–1.0). Omitted for non-vector results (FTS5, keyword, profile).
- `age_hours`: Hours elapsed since memory's `timestamp` (falls back to `created_at` if `timestamp` is absent).
- `score`: Unified confidence score (0.0–1.0). See v2.3.2 section for computation details.

**Search Strategy (cascading):**

```
recall(agent_id, query, limit)
  │
  ├─ 1. Vector Search
  │     Condition: _embedding_client AND query.strip() non-empty
  │     → Compute query embedding (via EmbeddingClient)
  │     → Cosine similarity on memories.embedding + episodes.embedding
  │     → Filter by CPERSONA_VECTOR_MIN_SIMILARITY (default 0.3)
  │     → Return top-K candidates with scores
  │     → Max rows scanned: CPERSONA_MAX_MEMORIES (default 500, OOM guard)
  │
  ├─ 2. FTS5 Full-Text Search
  │     Condition: CPERSONA_FTS_ENABLED AND query.strip() non-empty
  │     → Sanitize query: strip non-alphanumeric/CJK chars (regex [^\w\s])
  │     → Quote each word for phrase matching (FTS5 injection prevention)
  │     → Query episodes_fts → ranked results with [Episode] prefix
  │
  ├─ 3. Profile Lookup
  │     Condition: always executed
  │     → Fetch profiles for this agent_id
  │     → Include as [Profile] prefixed contextual information
  │
  └─ 4. Keyword Fallback (v2.3.4+: FTS5 on memories_fts)
        Condition: remaining slots available after strategies 1-3
        → FTS5 match on memories_fts (primary); LIKE fallback when FTS disabled
        → Chronological ordering (newest first)
        → Max rows scanned: CPERSONA_MAX_MEMORIES (OOM guard)

  → Merge all results, deduplicate by seen_ids set, sort by relevance
  → Truncate to limit, reverse to chronological order for LLM context
  → Apply Phase 4 timestamp annotations: [Memory from YYYY-MM-DD HH:MM TZ]
```

**Alternative: RRF mode (v2.4)**

When `CPERSONA_RECALL_MODE=rrf`, the cascade is replaced by Reciprocal Rank Fusion:

```
recall(agent_id, query, limit)  [RRF mode]
  │
  ├─ 1. Vector Search (independent, threshold relaxed by RRF_THRESHOLD_FACTOR)
  ├─ 2. FTS5 Episode Search (independent)
  ├─ 3. FTS5 Memory Search (independent)
  │
  ├─ Merge: RRF score = Σ 1/(k + rank) for each retriever that found the doc
  ├─ Profile Injection (unchanged, always executed)
  ├─ Confidence Re-rank (if CPERSONA_CONFIDENCE_ENABLED=true)
  └─ Autocut (if CPERSONA_AUTOCUT_ENABLED=true, largest score gap detection)
```

RRF avoids cascade's positional bias where later stages are disadvantaged by earlier stages filling slots. Each retriever runs independently and contributes equally to the final ranking.

**FTS5 Injection Prevention:**
- Input sanitized with `re.sub(r'[^\w\s]', "", query, flags=re.UNICODE)` — strips all FTS5 operators (`AND`, `OR`, `NOT`, `NEAR`, `*`, `^`, `-`)
- Each remaining word is individually quoted for exact matching
- Prevents operator reconstruction from word boundaries

### 3.3 update_profile

Extract and update user profile from conversation history.

```json
{
  "name": "update_profile",
  "description": "Extract user facts from conversation and merge with existing profile.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": {
        "type": "string",
        "description": "Agent identifier"
      },
      "history": {
        "type": "array",
        "description": "Recent conversation messages",
        "items": { "type": "object" }
      }
    },
    "required": ["agent_id", "history"]
  }
}
```

**Response:** `{"ok": true, "profiles_updated": 1}` or `{"error": "..."}`

**Behavior:**
1. Format conversation history into `[User] ... / [Agent] ...` text
2. Fetch existing profile from `profiles` table (`WHERE agent_id = ? AND user_id = ''`)
3. Call LLM proxy with prompt: "Extract facts about the user... MERGE with existing facts — keep all existing information unless explicitly contradicted"
4. UPSERT result into `profiles` table (`ON CONFLICT(agent_id, user_id) DO UPDATE`)
5. **Fallback:** If LLM proxy is unavailable, concatenate user lines from history as a simple profile summary

### 3.4 archive_episode

Summarize and archive a conversation episode.

```json
{
  "name": "archive_episode",
  "description": "Summarize a conversation episode and store as searchable archive.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": {
        "type": "string",
        "description": "Agent identifier"
      },
      "history": {
        "type": "array",
        "description": "Conversation messages to archive",
        "items": { "type": "object" }
      }
    },
    "required": ["agent_id", "history"]
  }
}
```

**Response:** `{"ok": true, "episode_id": 42}` or `{"error": "..."}`

**Behavior:**
1. Format conversation history into text
2. Call LLM proxy for summarization: "Summarize the following conversation concisely (800-1200 characters). Preserve proper nouns, dates, decisions, and key technical details."
3. Call LLM proxy for keyword extraction: "Extract 5-10 search keywords... suitable for full-text search (FTS5). Output space-separated keywords only."
4. Extract `start_time` / `end_time` from message timestamps
5. Compute embedding on summary if provider available
6. Insert into `episodes` table (FTS5 triggers auto-index)
7. **Fallback (LLM unavailable):** Summary = concatenation of first/last messages with ellipsis; Keywords = word-frequency analysis with stopword removal

### 3.5 list_memories

List recent memories for an agent (dashboard/management use).

```json
{
  "name": "list_memories",
  "description": "List recent memories for an agent.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": {
        "type": "string",
        "description": "Agent identifier (empty for all agents)",
        "default": ""
      },
      "limit": {
        "type": "integer",
        "description": "Maximum number of memories to return",
        "default": 100
      }
    }
  }
}
```

**Response:**
```json
{
  "memories": [
    {
      "id": 1,
      "agent_id": "agent-xyz",
      "content": "...",
      "source": {},
      "timestamp": "2026-03-13T14:30:00+09:00",
      "created_at": "2026-03-13T14:30:00"
    }
  ],
  "count": 1
}
```

**Behavior:**
- `limit` is clamped to max 500
- If `agent_id` is empty, returns memories across all agents
- Ordered by `created_at DESC` (newest first)

### 3.6 list_episodes

List archived episodes for an agent (dashboard/management use).

```json
{
  "name": "list_episodes",
  "description": "List archived episodes for an agent.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": {
        "type": "string",
        "description": "Agent identifier (empty for all agents)",
        "default": ""
      },
      "limit": {
        "type": "integer",
        "description": "Maximum number of episodes to return",
        "default": 50
      }
    }
  }
}
```

**Response:**
```json
{
  "episodes": [
    {
      "id": 1,
      "agent_id": "agent-xyz",
      "summary": "...",
      "keywords": "word1 word2 ...",
      "start_time": "2026-03-13T14:00:00+09:00",
      "end_time": "2026-03-13T14:30:00+09:00",
      "created_at": "2026-03-13T14:30:00"
    }
  ],
  "count": 1
}
```

**Behavior:**
- `limit` is clamped to max 200
- If `agent_id` is empty, returns episodes across all agents
- Ordered by `created_at DESC` (newest first)

### 3.7 delete_memory

Delete a single memory entry.

```json
{
  "name": "delete_memory",
  "description": "Delete a single memory by ID.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "memory_id": {
        "type": "integer",
        "description": "Memory ID to delete"
      },
      "agent_id": {
        "type": "string",
        "description": "Agent identifier (if provided, enforces ownership check)",
        "default": ""
      }
    },
    "required": ["memory_id"]
  }
}
```

**Response:** `{"ok": true, "deleted_id": 123}` or `{"error": "..."}`

**Behavior:**
- If `agent_id` is provided: `DELETE FROM memories WHERE id = ? AND agent_id = ?` (ownership enforcement)
- If `agent_id` is empty: `DELETE FROM memories WHERE id = ?` (admin mode)
- Returns error if no matching row found

### 3.8 delete_episode

Delete a single episode entry.

```json
{
  "name": "delete_episode",
  "description": "Delete a single episode by ID.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "episode_id": {
        "type": "integer",
        "description": "Episode ID to delete"
      },
      "agent_id": {
        "type": "string",
        "description": "Agent identifier (if provided, enforces ownership check)",
        "default": ""
      }
    },
    "required": ["episode_id"]
  }
}
```

**Response:** `{"ok": true, "deleted_id": 42}` or `{"error": "..."}`

**Behavior:**
- Same ownership check pattern as `delete_memory`
- FTS5 triggers automatically clean up the full-text search index on deletion

### 3.9 delete_agent_data

Delete ALL data for an agent (bulk cleanup).

```json
{
  "name": "delete_agent_data",
  "description": "Delete all memories, profiles, and episodes for an agent.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": {
        "type": "string",
        "description": "Agent identifier (required, non-empty)"
      }
    },
    "required": ["agent_id"]
  }
}
```

**Response:**
```json
{
  "ok": true,
  "agent_id": "agent-xyz",
  "deleted_memories": 42,
  "deleted_profiles": 1,
  "deleted_episodes": 3
}
```

**Behavior:**
1. `agent_id` must be non-empty (returns error otherwise)
2. Deletes from `memories`, `profiles`, and `episodes` tables atomically
3. FTS5 triggers handle episode index cleanup
4. Called automatically by kernel when an agent is deleted (best-effort cleanup)

---

## 4. Database Schema

**File:** `data/cpersona.db` (independent from kernel's `cloto_memories.db`)

### 4.1 memories

Raw message storage (2.2-compatible).

```sql
CREATE TABLE memories (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id  TEXT    NOT NULL,
    msg_id    TEXT    NOT NULL DEFAULT '',
    content   TEXT    NOT NULL,
    source    TEXT    NOT NULL DEFAULT '{}',
    timestamp TEXT    NOT NULL,
    metadata  TEXT    NOT NULL DEFAULT '{}',
    embedding BLOB,
    created_at TEXT   NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_memories_agent
    ON memories(agent_id, created_at DESC);

CREATE INDEX idx_memories_msg_id
    ON memories(agent_id, msg_id);
```

### 4.2 profiles

Per-agent user profiles (restored from 2.1).

```sql
CREATE TABLE profiles (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id   TEXT NOT NULL,
    user_id    TEXT NOT NULL DEFAULT '',
    content    TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(agent_id, user_id)
);
```

### 4.3 episodes

Conversation episode archives with FTS5 (restored from 2.1).

```sql
CREATE TABLE episodes (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id   TEXT NOT NULL,
    summary    TEXT NOT NULL,
    keywords   TEXT NOT NULL DEFAULT '',
    embedding  BLOB,
    start_time TEXT,
    end_time   TEXT,
    resolved   INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_episodes_agent
    ON episodes(agent_id, created_at DESC);

-- FTS5 full-text search index
CREATE VIRTUAL TABLE episodes_fts USING fts5(
    summary,
    keywords,
    content=episodes,
    content_rowid=id,
    tokenize='trigram'
);

-- FTS5 full-text search index for memories (v2.3.4+)
CREATE VIRTUAL TABLE memories_fts USING fts5(
    content,
    content=memories,
    content_rowid=id,
    tokenize='trigram'
);

-- Triggers to keep FTS5 in sync
CREATE TRIGGER episodes_ai AFTER INSERT ON episodes BEGIN
    INSERT INTO episodes_fts(rowid, summary, keywords)
    VALUES (new.id, new.summary, new.keywords);
END;

CREATE TRIGGER episodes_ad AFTER DELETE ON episodes BEGIN
    INSERT INTO episodes_fts(episodes_fts, rowid, summary, keywords)
    VALUES ('delete', old.id, old.summary, old.keywords);
END;

CREATE TRIGGER episodes_au AFTER UPDATE ON episodes BEGIN
    INSERT INTO episodes_fts(episodes_fts, rowid, summary, keywords)
    VALUES ('delete', old.id, old.summary, old.keywords);
    INSERT INTO episodes_fts(rowid, summary, keywords)
    VALUES (new.id, new.summary, new.keywords);
END;
```

### 4.4 pending_memory_tasks

Background task queue persistence (Phase 5, restored from KS2.1).

```sql
CREATE TABLE pending_memory_tasks (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    task_type  TEXT NOT NULL,
    agent_id   TEXT NOT NULL,
    payload    TEXT NOT NULL,
    retries    INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

Tasks are inserted on `update_profile` / `archive_episode` tool calls and processed
asynchronously by the `MemoryTaskQueue` background loop. On process restart, any
surviving rows are automatically recovered and reprocessed (crash recovery).

### 4.5 Schema Versioning

The CPersona MCP server manages its own schema migrations at startup using a simple
version table:

```sql
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

---

## 5. Embedding Server (`tool.embedding`)

### 5.1 Overview

Dedicated MCP server for vector embedding generation. Decoupled from CPersona so that:
- The embedding model can be swapped without modifying CPersona
- Other MCP servers can reuse embeddings in the future
- Heavy models (~490MB) don't inflate CPersona's memory footprint

### 5.2 MCP Tools

#### embed

Single tool that handles both individual and batch embedding requests (max 100 texts per call).

```json
{
  "name": "embed",
  "description": "Generate vector embeddings for input texts.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "texts": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Texts to embed (batch, max 100)"
      }
    },
    "required": ["texts"]
  }
}
```

**Response:** `{"embeddings": [[0.012, -0.034, ...], ...], "dimensions": 384}`

**Limits:** Batch size exceeds 100 → `{"error": "Batch size exceeds limit (max 100)"}`

### 5.3 HTTP Endpoint

For CPersona direct access (bypasses kernel MCP routing):

```
POST http://127.0.0.1:{HTTP_PORT}/embed
Content-Type: application/json

{"texts": ["hello world", "test query"]}

→ {"embeddings": [[...], [...]], "dimensions": 384}
```

Batch size limit: 100 texts (same as MCP tool).

### 5.4 Providers

| Provider | Model | Dimensions | Memory | Latency | Cost |
|----------|-------|-----------|--------|---------|------|
| `onnx_miniml` | all-MiniLM-L6-v2 (ONNX) | 384 | ~490MB | <10ms/text | Free |
| `api_openai` | text-embedding-3-small | 1536 | ~40MB | ~100ms/text | $0.02/1M tokens |
| `api_deepseek` | (if available) | TBD | ~40MB | ~100ms/text | TBD |

Configured via environment variable:

```
EMBEDDING_PROVIDER=onnx_miniml    # or api_openai, api_deepseek
EMBEDDING_MODEL=                  # provider-specific (empty = provider default)
EMBEDDING_HTTP_PORT=8401           # HTTP endpoint port
EMBEDDING_API_KEY=sk-...           # for API providers only
EMBEDDING_API_URL=https://...      # for API providers only
```

---

## 6. Configuration

### 6.1 MCP Server Registration (data/mcp.toml)

```toml
[paths]
servers = "C:/path/to/cloto-mcp-servers/servers"

[[servers]]
id = "tool.embedding"
command = "python"
args = ["${servers}/embedding/server.py"]
transport = "stdio"
auto_restart = true
[servers.env]
EMBEDDING_PROVIDER = "onnx_miniml"
EMBEDDING_HTTP_PORT = "8401"

[[servers]]
id = "memory.cpersona"
command = "python"
args = ["${servers}/cpersona/server.py"]
transport = "stdio"
auto_restart = true
[servers.env]
CPERSONA_EMBEDDING_MODE = "http"
CPERSONA_EMBEDDING_URL = "http://127.0.0.1:8401/embed"
```

### 6.2 CPersona Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CPERSONA_DB_PATH` | `data/cpersona.db` | Path to CPersona's dedicated SQLite database |
| `CPERSONA_EMBEDDING_MODE` | `none` | Embedding strategy: `http`, `api`, `none` |
| `CPERSONA_EMBEDDING_URL` | — | URL for `http` mode (embedding server endpoint) |
| `CPERSONA_EMBEDDING_API_KEY` | — | API key for `api` mode |
| `CPERSONA_EMBEDDING_API_URL` | `https://api.openai.com/v1/embeddings` | API endpoint for `api` mode |
| `CPERSONA_EMBEDDING_MODEL` | `text-embedding-3-small` | Model name (provider-specific) |
| `CPERSONA_MAX_MEMORIES` | `500` | Max memories loaded per recall (OOM guard) |
| `CPERSONA_FTS_ENABLED` | `true` | Enable FTS5 episode search |
| `CPERSONA_VECTOR_MIN_SIMILARITY` | `0.3` | Cosine similarity threshold for vector search (0.0–1.0) |
| `CPERSONA_EMBEDDING_CACHE_SIZE` | `256` | LRU embedding cache capacity (entries) |
| `CPERSONA_EMBEDDING_CACHE_TTL` | `300` | Embedding cache entry lifetime (seconds) |
| `CPERSONA_LLM_PROXY_URL` | `http://127.0.0.1:8082/v1/chat/completions` | Kernel LLM proxy endpoint for memory extraction |
| `CPERSONA_LLM_PROVIDER` | `cerebras` | LLM provider name (sent via `X-LLM-Provider` header) |
| `CPERSONA_LLM_MODEL` | `gpt-oss-120b` | LLM model name for extraction tasks |
| `CPERSONA_TASK_QUEUE_ENABLED` | `true` | Enable background task queue for `update_profile`/`archive_episode` |
| `CPERSONA_TASK_MAX_RETRIES` | `3` | Max retry attempts before discarding a failed task |
| `CPERSONA_TASK_RETRY_DELAY` | `30` | Seconds to wait between retries |
| `CPERSONA_CONFIDENCE_ENABLED` | `false` | Enable confidence metadata in recall output (v2.3.2) |
| `CPERSONA_COSINE_FLOOR` | `0.20` | Cosine normalization lower bound (model-dependent; v2.3.2) |
| `CPERSONA_COSINE_CEIL` | `0.75` | Cosine normalization upper bound (model-dependent; v2.3.2) |
| `CPERSONA_DECAY_RATE` | `0.005` | Time decay rate per hour for confidence score (v2.3.2) |
| `CPERSONA_RESOLVED_DECAY_FACTOR` | `0.3` | Additional decay factor for resolved episodes (v2.3.2) |
| `CPERSONA_VECTOR_SEARCH_MODE` | `local` | `local` (BLOB+NumPy) / `remote` (embedding server delegation) |
| `CPERSONA_STORE_BLOB` | `true` | Store embedding BLOBs even in remote mode (fallback) |
| `CPERSONA_AUTO_CALIBRATE` | `false` | Auto-calibrate VECTOR_MIN_SIMILARITY on startup (v2.3.7) |
| `CPERSONA_CALIBRATE_SAMPLE_SIZE` | `200` | Number of embeddings to sample for calibration |
| `CPERSONA_CALIBRATE_Z_FACTOR` | `1.0` | Z-score multiplier (higher = more permissive) |
| `CPERSONA_CALIBRATE_FLOOR` | `0.05` | Minimum threshold floor for calibration |
| `CPERSONA_RECALL_MODE` | `cascade` | `cascade` (sequential 4-stage) / `rrf` (Reciprocal Rank Fusion) |
| `CPERSONA_RRF_K` | `60` | RRF smoothing parameter K |
| `CPERSONA_RRF_THRESHOLD_FACTOR` | `0.5` | RRF mode vector similarity threshold multiplier |
| `CPERSONA_AUTOCUT_ENABLED` | `false` | Enable score-gap detection noise cutoff (v2.4) |
| `CPERSONA_TRANSPORT` | `stdio` | `stdio` / `streamable-http` |
| `CPERSONA_AUTH_TOKEN` | (empty) | Bearer token for HTTP transport authentication |
| `CPERSONA_HTTP_HOST` | `0.0.0.0` | HTTP server bind address |
| `CPERSONA_HTTP_PORT` | `8402` | HTTP server port |

---

## 7. Kernel Integration (Memory Resolver)

### 7.1 Memory Resolution (Dual Dispatch)

The kernel resolves the memory server through a three-step fallback chain:

```rust
// system.rs — Memory Resolution Chain

// Step 1: Check agent's preferred_memory metadata
let memory_plugin = if let Some(preferred_id) = agent.metadata.get("preferred_memory") {
    self.registry.get_engine(preferred_id).await
} else {
    self.registry.find_memory().await  // legacy Rust plugin search
};

// Step 2: MCP fallback via CapabilityType::Memory
let mcp_memory: Option<(Arc<McpClientManager>, String)> = if memory_plugin.is_none() {
    if let Some(ref mcp) = self.registry.mcp_manager {
        mcp.resolve_capability_server(CapabilityType::Memory)
            .await
            .and_then(|server_id| {
                // 🔐 Access control: agent must have grant to the memory server
                if granted_server_ids.contains(&server_id) {
                    Some((mcp.clone(), server_id))
                } else {
                    None  // agent lacks access — memory skipped
                }
            })
    } else { None }
} else { None };
```

**Capability Classification** (`capability_dispatcher.rs`):

Servers are classified as `CapabilityType::Memory` by:
1. **Server prefix** (primary): `server_id.starts_with("memory.")`
2. **Tool name fallback**: `store`, `recall`, `list_memories`, `delete_memory`, `list_episodes`, `delete_episode`, `archive_episode`, `delete_agent_data`, `update_profile`

### 7.2 REST Endpoint Integration

Memory operations are called via `call_server_tool()` with timeout:

```rust
// Recall (with timeout)
let recall_args = serde_json::json!({
    "agent_id": agent.id,
    "query": msg.content,
    "limit": self.memory_context_limit,
});
match tokio::time::timeout(
    Duration::from_secs(self.memory_timeout_secs),
    mcp.call_server_tool(server_id, "recall", recall_args),
).await {
    Ok(Ok(result)) => Self::parse_mcp_recall_result(&result),
    Ok(Err(e)) => vec![],   // MCP error → empty context
    Err(_) => vec![],        // timeout → empty context
};

// Store (same timeout pattern)
mcp.call_server_tool(&server_id, "store", store_args).await;
```

**Agent deletion** triggers automatic cleanup:
```rust
// agents.rs — on DELETE /api/agents/:id
mcp.call_server_tool(mem_server, "delete_agent_data", json!({"agent_id": id})).await;
```

### 7.3 Auto-Archive (`maybe_archive_episode`)

The kernel automatically triggers episode archival when enough unarchived memories accumulate.

| Parameter | Value |
|-----------|-------|
| `TOOL_USAGE_THRESHOLD` | 10 (constant in `system.rs`) |

**Flow:**
1. After each `store()`, call `maybe_archive_episode()`
2. Fetch recent memories via `list_memories(agent_id, limit=15)`
3. Fetch last episode via `list_episodes(agent_id, limit=1)` to get `created_at` timestamp
4. Count memories newer than last episode's `created_at`
5. If count ≥ `TOOL_USAGE_THRESHOLD`:
   - Call `archive_episode(agent_id, history)` with unarchived messages
   - On success, call `update_profile(agent_id, history)` to refresh user profile

---

## 8. Data Migration

### 8.1 From CPersona 2.2 (plugin_data)

Existing 2.2 data lives in `cloto_memories.db` → `plugin_data` table:

```
plugin_id = 'memory.cpersona'
key = 'mem:{agent_id}:{timestamp}:{hash}'
value = JSON(ClotoMessage)
```

**Manual SQL migration procedure:**

1. Open source database: `sqlite3 data/cloto_memories.db`
2. Query plugin_data: `SELECT key, value FROM plugin_data WHERE plugin_id = 'memory.cpersona';`
3. For each row, parse `key` to extract `agent_id` and `timestamp`
4. Deserialize `value` as ClotoMessage JSON
5. Insert into destination: `sqlite3 data/cpersona.db` → `memories` table
6. Optionally compute embeddings for migrated memories via the embedding server HTTP endpoint

### 8.2 From CPersona 2.1 (ai_karin)

Not automated. CPersona 2.1 used Discord-specific schemas (user_id, guild_id)
that don't map to ClotoCore's agent_id model. Manual migration may be performed if needed.

---

## 9. Implementation Phases

### Phase 1: MCP Pipeline — **Completed**

- [x] `servers/cpersona/server.py` with `store` and `recall` tools
- [x] `recall`: FTS5 + keyword fallback (no vector search)
- [x] `update_profile`: Stub (no LLM)
- [x] `archive_episode`: Simple concatenation (no LLM)
- [x] Dedicated SQLite database (`data/cpersona.db`)
- [x] Kernel Memory Resolver update (`system.rs`, `registry.rs`, `mcp.rs`)
- [x] Remove Rust `plugins/cpersona/` dependency

### Phase 2: Embedding Integration — **Completed** (v0.4.15)

- [x] `mcp-servers/embedding/server.py` with `embed` tool + HTTP endpoint
- [x] CPersona `EmbeddingClient` interface with provider abstraction
- [x] Vector columns populated on `store` and `archive_episode`
- [x] `recall` vector search path activated (ONNX MiniLM, cosine similarity)
- [x] Auto-download ONNX model on first startup

### Phase 3: LLM Memory Extraction (2.1 Restoration) — **Completed** (v0.4.15)

- [x] `update_profile`: LLM-powered fact extraction via Cerebras
- [x] `archive_episode`: LLM-powered summarization with keywords via Cerebras
- [x] Auto `update_profile` trigger after episode archival
- [ ] Semantic cache (high-confidence recall caching) — deferred

### Phase 5: Background Task Queue (KS2.1 Restoration) — **Completed**

- [x] `pending_memory_tasks` table in cpersona.db (DB-persisted FIFO queue)
- [x] `MemoryTaskQueue` class: asyncio background loop with crash recovery
- [x] `update_profile` / `archive_episode` tools enqueue and return immediately
- [x] FIFO processing with configurable retry (`CPERSONA_TASK_MAX_RETRIES`, `CPERSONA_TASK_RETRY_DELAY`)
- [x] Startup recovery: pending tasks from previous crash automatically reprocessed
- [x] New tool: `get_queue_status` for monitoring
- [x] Disable via `CPERSONA_TASK_QUEUE_ENABLED=false` (falls back to synchronous execution)

### CPersona 2.3.1: Memory Portability & Claude Code Integration — **Complete**

- [x] `export_memories` tool: JSONL output (header/memory/episode/profile records, optional base64 embeddings)
- [x] `import_memories` tool: JSONL input with msg_id deduplication (idempotent), agent_id remapping, dry_run preview, profile UPSERT
- [x] `archive_episode` pre-computed summary/keywords: skip LLM proxy when caller provides summary (enables sub-agent summarization via Sonnet/Haiku)
- [x] MemoryCore dashboard Export/Import UI (client-side JSONL + POST /api/memories/import Rust endpoint)
- [x] Claude Code integration documentation (`docs/CLAUDE_CODE_INTEGRATION.md`)
- [ ] Dashboard Export/Import UI verification (`npx tauri dev`)
- [ ] Claude Code `recall` tool verification
- [ ] Embedding server port conflict resolution (multiple Claude Code sessions share port 8401 — second instance fails to bind, blocking cpersona startup)

### CPersona 2.3.2: Memory Confidence Score — **Complete**

Enrich recall output with machine-computed confidence metadata so that consuming
agents can modulate their response certainty without hardcoded behavioral rules.

**Design philosophy:** CPersona provides **raw numerical data only**. It does NOT
prescribe behavioral labels (e.g., "assertive", "uncertain"). The consuming agent's
persona layer interprets the numbers and decides how to express confidence — a formal
agent might say "records indicate…" while a casual agent might say "たぶんね".

```
CPersona (memory layer)     → confidence: { cosine: 0.55, age_hours: 120, score: 0.59 }
  ↓
Persona / System Prompt     → interprets score according to personality
  ↓
Agent output                → personality-appropriate certainty expression
```

**Confidence score computation:**

```python
# Step 1: Cosine normalization (compress practical range to 0.0–1.0)
COSINE_FLOOR = 0.20   # Env: CPERSONA_COSINE_FLOOR
COSINE_CEIL  = 0.75   # Env: CPERSONA_COSINE_CEIL
norm_cos = clamp((raw_cosine - COSINE_FLOOR) / (COSINE_CEIL - COSINE_FLOOR), 0.0, 1.0)

# Step 2: Time decay (hyperbolic, approaches 0 but never reaches it)
DECAY_RATE = 0.005    # Env: CPERSONA_DECAY_RATE
age_hours = (now - timestamp).total_seconds() / 3600
time_decay = 1.0 / (1.0 + age_hours * DECAY_RATE)

# Step 3: Geometric mean (preserves 2D quadrant separation)
score = math.sqrt(norm_cos * time_decay)
```

**Why geometric mean over alternatives:**

| Formula | High cos + old (168h) | Low cos + new (1h) | Problem |
|---------|----------------------|-------------------|---------|
| `cos * decay` (product) | 0.35 | 0.09 | Scores compress toward zero — old relevant memories die |
| `cos + decay` (sum) | 1.18 | 1.08 | Exceeds 1.0, low-cos memories score too high |
| `α·cos + (1-α)·decay` (weighted) | 0.52–0.61 | 0.36–0.42 | Requires tuning α; low-cos new memories rank too high |
| **`√(cos·decay)`** (geometric mean) | **0.59** | **0.30** | **Clean quadrant separation, 0.0–1.0 range, no extra parameters** |

**Score examples (DECAY_RATE=0.005):**

| Memory scenario | raw cos | norm_cos | age | time_decay | **score** |
|----------------|---------|----------|-----|------------|-----------|
| High relevance + just now | 0.65 | 0.82 | 1h | 0.995 | **0.90** |
| High relevance + 1 week | 0.55 | 0.64 | 168h | 0.54 | **0.59** |
| High relevance + 6 months | 0.60 | 0.73 | 4380h | 0.04 | **0.17** |
| Low relevance + just now | 0.25 | 0.09 | 1h | 0.995 | **0.30** |
| Low relevance + 6 months | 0.25 | 0.09 | 4380h | 0.04 | **0.06** |

**Quadrant interpretation (for persona designers, NOT enforced by CPersona):**

```
                    cosine (semantic relevance)
                    Low ←————————→ High
              ┌─────────────┬─────────────┐
  New (age)   │  ~0.30      │  ~0.90      │
              │  (uncertain)│  (confident) │
              ├─────────────┼─────────────┤
  Old (age)   │  ~0.06      │  ~0.17–0.59 │
              │  (no basis) │  (fading)    │
              └─────────────┴─────────────┘
```

**Scope:**

- [x] Extend `_search_vector()` to compute and attach `confidence` dict to each result
- [x] Extend `do_recall()` to include `confidence` in output messages (opt-in via `CPERSONA_CONFIDENCE_ENABLED`)
- [x] Timestamp selection: prefer `memories.timestamp`, fall back to `memories.created_at`
- [x] Non-vector results (FTS5, keyword, profile): `confidence.cosine` omitted, `confidence.score` based on `age_hours` only (`score = sqrt(time_decay)`)
- [x] Environment variables: `CPERSONA_CONFIDENCE_ENABLED`, `CPERSONA_COSINE_FLOOR`, `CPERSONA_COSINE_CEIL`, `CPERSONA_DECAY_RATE`
- [x] Unit tests: score computation, edge cases (NULL timestamp, zero age, cosine outside floor/ceil range) — 14 tests
- [x] `DECAY_RATE` default (0.005) — implemented, empirical validation ongoing
- [x] Confidence-based re-ranking: results re-sorted by unified score before truncation

**Implementation scope:** `cloto-mcp-servers` repo only (`servers/cpersona/server.py`).
No ClotoCore kernel changes required. No schema changes.

### CPersona 2.3.7: Auto-Calibration — **Complete**

`VECTOR_MIN_SIMILARITY` is embedding-model-dependent. Different models produce
different cosine similarity distributions — MiniLM clusters around 0.2–0.7, while
OpenAI text-embedding-3-small may cluster around 0.3–0.8. Manual tuning per model
is fragile and error-prone.

**Solution: z-score based null distribution calibration**

Implemented as `calibrate_threshold` MCP tool and optional startup auto-calibration.
Samples random embedding pairs to build a null distribution (mostly unrelated pairs),
then sets the threshold at `mean - z × std` to filter the noise floor.

```python
# Actual implementation (do_calibrate_threshold in server.py)
rows = await db.execute_fetchall(
    "SELECT embedding FROM memories WHERE agent_id = ? AND embedding IS NOT NULL "
    "ORDER BY RANDOM() LIMIT ?", (agent_id, sample_n))
vecs = np.array([np.frombuffer(blob, dtype=np.float32) for blob in rows])
sim_matrix = vecs @ vecs.T
pairwise_sims = sim_matrix[np.triu_indices(len(vecs), k=1)]

# z-score threshold: mean - z * std (lower tail filtering)
threshold = max(mean - z_factor * std, CALIBRATE_FLOOR)
VECTOR_MIN_SIMILARITY = threshold
```

Initially designed with percentile (p5) approach, but this produced destructively
high thresholds on homogeneous corpora. z-score adapts to corpus diversity:
- Diverse corpus (mean=0.15, std=0.10): threshold ≈ 0.05
- Homogeneous corpus (mean=0.55, std=0.10): threshold ≈ 0.45

**Scope:**

- [x] `calibrate_threshold` MCP tool (manual invocation)
- [x] `CPERSONA_AUTO_CALIBRATE` env var (`true`/`false`, default `false`)
- [x] `CPERSONA_CALIBRATE_SAMPLE_SIZE` env var (default `200`)
- [x] `CPERSONA_CALIBRATE_Z_FACTOR` env var (default `1.0`)
- [x] `CPERSONA_CALIBRATE_FLOOR` env var (default `0.05`)
- [x] Log calibrated values at INFO level
- [ ] Persistent model-specific calibration (detect embedding dimension change → re-calibrate → store in DB) — deferred to future version

**Implementation scope:** `cloto-mcp-servers` repo only. No schema changes.

#### Cross-Agent Memory Merge (Planned — v2.3.8 scope)

When the same user interacts with cpersona from multiple clients (e.g., Claude Code as
`claude-code` and Claude web as `claude-web`), identical memories must be stored under
each agent_id separately. This creates duplication and maintenance burden.

**Solution: `merge_memories` tool**

```python
merge_memories(
    source_agent_id: str,   # Agent to merge FROM
    target_agent_id: str,   # Agent to merge INTO
    mode: str = "copy",     # "copy" (preserve source) or "move" (delete source after merge)
    dry_run: bool = False,  # Preview without writing
)
```

**Merge behavior by record type:**

| Record Type | Merge Strategy | Dedup |
|-------------|---------------|-------|
| memories | Copy to target | msg_id dedup (skip duplicates) |
| episodes | Copy to target | summary hash dedup |
| profiles | LLM merge | Reuse existing `_run_update_profile()` logic |

**Anti-Contamination compatibility:** Merge is an explicit, user-initiated operation
(like export/import). It does not weaken agent_id isolation — the user consciously
decides to merge agent A's memories into agent B. Implicit cross-agent access remains
prohibited.

**Relationship with export/import:** `merge_memories` is conceptually equivalent to
`export_memories(source) → import_memories(target)`, but executed as a single atomic
operation without intermediate files. It reuses the same dedup and UPSERT logic.

**Environment variables:** None. Merge is a tool invocation, not a background behavior.

**Implementation scope:** `cloto-mcp-servers` repo only (`servers/cpersona/server.py`).
No schema changes. New MCP tool registration only.

### CPersona 2.4+ Roadmap

#### Design Philosophy: 3-Layer Hybrid Evolution

CPersona v2.3.x established a **3-layer hybrid architecture** (Agent Tools / RAG System / Filter).
The v2.4+ roadmap is positioned as progressive deepening and expansion of these 3 layers.

- **v2.4 (Complete)**: Refining the 3 layers — Reciprocal Rank Fusion (RRF) as alternative recall mode.
  Vector and FTS5 run independently and merge by RRF score, eliminating cascade's positional bias.
  Autocut (Weaviate-style score gap detection) for noise filtering. RRF threshold relaxation for broader coverage.
- **v2.5 (Planned)**: Deepening the 3 layers — Internalizes temporal awareness into the RAG layer (Layer 2).
  By making search time-aware, the system can prioritize "slightly less similar but recent" memories
  over "semantically similar but stale" ones. The dual structure of v2.3.2 Confidence Score
  (post-recall output metadata) and v2.5 Recency Boost (search-time ranking) ensures that
  temporal relevance is firmly embedded in the RAG layer.
- **v3.0**: 3-layer → 4-layer expansion — Graph Memory (entities/edges) is added as Layer 4.
  Bi-Temporal Model adds multi-dimensional time (valid time + record time).

#### RRF (Reciprocal Rank Fusion) — **Complete** (v2.4)

Implemented as `CPERSONA_RECALL_MODE=rrf`. Three retrievers (vector, FTS5 episodes, FTS5 memories) run independently and merge by RRF score `1/(k + rank)`. Profile is injected outside RRF. Autocut (`CPERSONA_AUTOCUT_ENABLED`) detects the largest score gap and cuts noise results.

**Environment variables:**

| Variable | Default | Description |
|----------|---------|-------------|
| `CPERSONA_RECALL_MODE` | `cascade` | `cascade` / `rrf` |
| `CPERSONA_RRF_K` | `60` | RRF smoothing constant |
| `CPERSONA_RRF_THRESHOLD_FACTOR` | `0.5` | Vector threshold multiplier in RRF mode |
| `CPERSONA_AUTOCUT_ENABLED` | `false` | Score gap noise cutoff |

**Implementation scope:** `cloto-mcp-servers` repo (`servers/cpersona/server.py`, `_recall_rrf()` and `_autocut()` functions). No schema changes.

#### Recency-Weighted Vector Search (Planned — v2.5 scope)

Current vector search treats all memories equally regardless of age. A fixed cosine
similarity threshold (`CPERSONA_VECTOR_MIN_SIMILARITY = 0.3`) means a relevant
recent memory at 0.28 is discarded while an older, less contextually relevant memory
at 0.31 is returned.

**Solution: Gated recency boost**

```python
COSINE_GATE = 0.20          # Minimum cosine similarity (hard floor)
RECENCY_MAX_BOOST = 0.10    # Maximum score bonus from recency
RECENCY_DECAY = 0.05        # Decay rate (higher = faster decay)

if cosine_sim < COSINE_GATE:
    continue  # Block semantically irrelevant memories regardless of recency

age_hours = (now - timestamp).total_seconds() / 3600
recency_boost = min(RECENCY_MAX_BOOST, RECENCY_MAX_BOOST / (1.0 + age_hours * RECENCY_DECAY))
final_score = cosine_sim + recency_boost
```

**Score examples:**

| Memory | cosine | age | boost | final | outcome |
|--------|--------|-----|-------|-------|---------|
| Unrelated chat (5 min ago) | 0.12 | 0.08h | — | — | Blocked by gate (< 0.20) |
| Related recent talk (10 min ago) | 0.28 | 0.17h | +0.099 | 0.379 | Boosted above old threshold |
| Birthday fact (6 months ago) | 0.35 | 4380h | +0.000 | 0.350 | Survives on cosine alone |
| High-relevance old memory (1 week) | 0.55 | 168h | +0.011 | 0.561 | Still top-ranked |

**Anti-contamination guarantees:**
- `COSINE_GATE` mechanically blocks semantically irrelevant memories (prevents noise injection)
- `RECENCY_MAX_BOOST` caps recency contribution (cosine similarity remains dominant signal)
- Recency acts as a **tiebreaker** between memories of similar relevance, not a primary ranking factor
- Profile memories are unaffected (retrieved via Strategy 2, not vector search)
- Episode memories are unaffected (retrieved via Strategy 1 FTS5, not vector search)

**Environment variables (all optional, defaults preserve current behavior):**

| Variable | Default | Description |
|----------|---------|-------------|
| `CPERSONA_COSINE_GATE` | 0.20 | Hard minimum cosine similarity |
| `CPERSONA_RECENCY_MAX_BOOST` | 0.10 | Maximum recency score bonus |
| `CPERSONA_RECENCY_DECAY` | 0.05 | Recency decay rate per hour |
| `CPERSONA_RECENCY_ENABLED` | `false` | Set to `true` to enable (opt-in) |

When `CPERSONA_RECENCY_ENABLED=false` (default), the system uses the existing fixed
threshold behavior (`CPERSONA_VECTOR_MIN_SIMILARITY`). This ensures full backward
compatibility.

**Relationship with v2.3.2 (Confidence Score):**
v2.3.2 and v2.4 address different concerns using overlapping concepts:
- **v2.3.2** = output metadata (annotates returned memories with confidence scores for the agent to interpret)
- **v2.4** = search ranking (changes *which* memories are returned by boosting recent relevant results)

When both are enabled, v2.4's recency-boosted `final_score` is used for ranking, and
v2.3.2's geometric mean `confidence.score` is attached to each result independently.
The two scores serve different purposes and are computed separately — v2.4 uses additive
boost for ranking, v2.3.2 uses geometric mean for certainty communication.

v2.4 can reuse v2.3.2's cosine normalization utilities (`COSINE_FLOOR`/`COSINE_CEIL`)
and timestamp parsing logic. `COSINE_GATE` may be expressed as `COSINE_FLOOR` (the
normalized equivalent of gate=0.0) when v2.3.2 normalization is available.

**Implementation scope:** `cloto-mcp-servers` repo only (`servers/cpersona/server.py`,
`_search_vector()` function). No ClotoCore kernel changes required.

#### Enhanced `get_queue_status` (Planned)

Current `get_queue_status` returns basic `enabled` / `pending` counts. The following
enhancements are planned for CPersona 2.4+:

- [ ] **Task breakdown by type** — Return per-type pending counts (`update_profile: N, archive_episode: M`) instead of a single aggregate number
- [ ] **Retry-in-progress count** — Report how many tasks have `retries > 0` (currently failing and being retried), separate from fresh tasks
- [ ] **Oldest pending task age** — Expose `created_at` of the oldest pending task to detect queue staleness or stuck tasks
- [ ] **Cumulative session statistics** — Track `processed_count`, `failed_count`, and `uptime` across the queue's lifetime for throughput monitoring
- [ ] **Liveness indicator** — Report whether the background loop is actively running (`is_alive: true/false`), distinguishing "queue enabled but loop crashed" from normal idle

#### CPersona 2.5 Roadmap — Search Precision & Profile Evolution

**Theme:** Sharpen existing strengths without architectural changes.

**Note:** RRF was originally planned as v2.5 but was implemented ahead of schedule as v2.4.
The remaining v2.5 items are Profile Enrichment and Benchmark Framework.

##### 1. Profile Enrichment (Limited Memory Evolution)

Current `update_profile` only updates on contradiction ("keep all existing information
unless explicitly contradicted"). v2.5 extends this to **contextual enrichment** —
new information that adds context to existing facts without contradicting them.

**Example:**
- Existing profile: `"User is a software engineer"`
- New conversation: User mentions working on distributed systems at a startup
- Current behavior (v2.3): No update (no contradiction detected)
- v2.5 behavior: `"User is a software engineer specializing in distributed systems, works at a startup"`

**LLM prompt change** (in `_run_update_profile()`):
```
Current: "MERGE with existing facts — keep all existing information unless explicitly contradicted"
v2.5:    "MERGE with existing facts — keep all existing information unless explicitly contradicted.
          ENRICH existing facts with new contextual details when the conversation provides
          additional specificity (e.g., job title → job title + specialization + company).
          Do NOT infer or speculate — only add details explicitly stated in the conversation."
```

**Anti-contamination safeguard:** The "Do NOT infer or speculate" instruction prevents
hallucinated enrichment. Only explicitly stated details are merged.

**Quality risk:** Enrichment quality depends heavily on the LLM's instruction-following
precision. The boundary between "enrich with stated details" and "infer unstated details"
is subtle, especially for smaller/faster models (e.g., Cerebras gpt-oss-120b). Mitigation:
use the Benchmark Verification Framework (item 3) to measure profile quality before/after
enrichment — specifically, a **profile diff audit** that compares enriched profiles against
source conversations to detect hallucinated additions.

**Implementation scope:** `cloto-mcp-servers` repo (`servers/cpersona/server.py`,
`_run_update_profile()` LLM prompt only). No schema changes, no new tools.

##### 3. Benchmark Verification Framework (Parallel Track)

Establish baseline metrics on standard memory benchmarks to objectively measure
improvements across versions.

**Target benchmarks:**
- **LOCOMO** — Multi-session conversation memory (Mem0's primary benchmark)
- **LongMemEval** — Long-term memory evaluation (Zep/Graphiti benchmark)
- **HaluMem** — Memory-induced hallucination detection (validates anti-contamination)

**Deliverables:**
- `benchmarks/` directory in `cloto-mcp-servers` repo
- Evaluation scripts per benchmark (Python, dataset-agnostic runner)
- `benchmarks/results/` with JSON score history per version (machine-readable for CI/regression detection)
- Baseline scores for v2.3, re-measured after v2.4 and v2.5 changes
- Results documented in this file (new Section 11: Benchmark Results)
- **Profile enrichment audit**: side-channel benchmark comparing enriched profiles against source conversations to detect hallucinated additions (validates item 2 quality)

**CI integration:** Runner scripts should be executable as `python benchmarks/run.py --suite locomo --output benchmarks/results/v2.5-locomo.json`, enabling future automation in CI pipelines. Score history accumulation in `benchmarks/results/` enables regression detection when v3.0 graph memory is introduced.

---

#### CPersona 3.0 Roadmap — Graph Memory Paradigm

**Theme:** Architectural shift from flat memory to structured knowledge graph.
This is the pathway to closing the gap with Zep/Graphiti, Mem0, and Cognee.

**Scope boundary:** CPersona v3.0 is strictly about **knowledge representation**
(what the AI knows/remembers). Real-time emotional state simulation is out of scope
and belongs to `persona.emotion` v1.0 — a separate MCP server (see PROJECT_VISION.md
Layer 4). The relationship is one-directional: `persona.emotion` may read personality
traits from CPersona's profile/graph, but CPersona does not depend on the emotion
engine. Emotional events *can* be stored as graph edges (e.g., `User --[felt]--> joy`)
once both systems are available, but this is an integration point, not a v3.0 deliverable.

##### 1. Graph Memory (Core Feature)

Introduce entity-relationship graph stored in SQLite (no external graph DB dependency).

**New tables:**

```sql
CREATE TABLE entities (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id   TEXT NOT NULL,
    name       TEXT NOT NULL,
    entity_type TEXT NOT NULL DEFAULT 'unknown',  -- person, place, concept, etc.
    attributes TEXT NOT NULL DEFAULT '{}',          -- JSON key-value pairs
    embedding  BLOB,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(agent_id, name, entity_type)
);

CREATE TABLE edges (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id    TEXT NOT NULL,
    source_id   INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    target_id   INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    relation    TEXT NOT NULL,            -- "works_at", "friend_of", "likes", etc.
    attributes  TEXT NOT NULL DEFAULT '{}',
    valid_from  TEXT,                      -- bi-temporal: when the fact became true
    valid_to    TEXT,                      -- bi-temporal: when the fact ceased to be true (NULL = current)
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(agent_id, source_id, target_id, relation)
);

CREATE TABLE entity_mentions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_id   INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    memory_id   INTEGER REFERENCES memories(id) ON DELETE SET NULL,
    episode_id  INTEGER REFERENCES episodes(id) ON DELETE SET NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
```

**Entity resolution challenge:** The `UNIQUE(agent_id, name, entity_type)` constraint
handles "Tokyo (place)" vs "Tokyo (company)" via different `entity_type`, but cannot
distinguish same-name, same-type entities (e.g., two different people named "田中").
This ambiguity resolution is delegated to the LLM extraction pipeline, which must use
conversational context to disambiguate (e.g., "田中 from work" vs "田中 from school").
Strategies under consideration:

- **Contextual suffix**: LLM appends disambiguator to name (`"田中 (colleague)"`,
  `"田中 (school friend)"`) — simple, human-readable, but fragile
- **Canonical ID**: LLM assigns a stable identifier; `name` becomes display label,
  `attributes.canonical_id` becomes the unique key — robust, but requires LLM consistency
- **Merge-on-conflict**: Allow duplicate names, use embedding similarity to detect
  and merge duplicates periodically — tolerant, but requires cleanup pipeline

Decision deferred to v3.0 implementation phase; the schema supports all three approaches
(the UNIQUE constraint can be relaxed to `UNIQUE(agent_id, name, entity_type, canonical_id)`
if needed).

**New MCP tools:**
- `store_entity(agent_id, name, entity_type, attributes)` — Create/update entity node
- `store_relation(agent_id, source, target, relation, valid_from?, valid_to?)` — Create edge
- `query_graph(agent_id, entity_name, depth?, relation_filter?)` — BFS traversal from entity

**Recall integration:** Graph traversal becomes Strategy 5 in the cascade, executed
when vector/FTS5 results contain recognized entity names.

##### 2. Bi-Temporal Model

Every edge (relationship) carries `valid_from` / `valid_to` timestamps.
Old facts are **invalidated**, not deleted — enabling temporal queries.

**Example:**
```
Edge: User --[lives_in]--> Tokyo    valid_from: 2024-01, valid_to: 2025-06
Edge: User --[lives_in]--> Osaka    valid_from: 2025-06, valid_to: NULL (current)
```

**Temporal query:** "Where did the user live last year?" → traverse edges where
`valid_from <= 2025-03 AND (valid_to IS NULL OR valid_to > 2025-03)` → Tokyo.

**Temporal extraction strategy:** Determining `valid_from`/`valid_to` is non-trivial.
Three approaches, in order of preference:

1. **LLM-extracted with conversation anchor** (primary): LLM extracts temporal
   references from conversation ("I moved to Osaka last June") and resolves them
   against the conversation timestamp to produce absolute dates. The conversation
   timestamp serves as an anchor for relative references.
2. **Conversation timestamp fallback**: When the LLM cannot extract an explicit
   temporal reference, `valid_from` defaults to the conversation timestamp (the
   moment the fact was first mentioned). This is imprecise but safe — it records
   "when we learned this" rather than "when it became true".
3. **NULL (unknown)**: When neither extraction nor timestamp is meaningful (e.g.,
   timeless facts like "User likes cats"), `valid_from` remains NULL, meaning
   "always valid".

**Hallucination risk:** LLM temporal extraction (approach 1) carries higher
hallucination risk than factual extraction. Mitigation: extracted dates are
cross-validated against conversation timestamp — if the LLM claims a `valid_from`
in the future or implausibly distant past, fall back to approach 2.

**Integration with anti-contamination:** Temporal annotations extended to include
validity period: `[Memory from 2024-01, valid until 2025-06]`.

##### 3. Full Memory Evolution

Extension of v2.5's profile enrichment to the graph level:
- New information triggers **retroactive edge updates** (A-MEM style)
- LLM evaluates whether new facts modify existing entity attributes or create new edges
- Cognee's memify concept: periodically prune stale nodes, strengthen frequent connections

**Dependency:** Requires graph memory (item 1) to be implemented first.

##### Sub-Phasing (Risk Management)

v3.0's three features have sequential dependencies. Explicit sub-phases prevent
scope creep and enable incremental validation against v2.5 benchmark baselines.

| Sub-Phase | Scope | Deliverable | Validation |
|-----------|-------|-------------|------------|
| **v3.0-alpha** | Graph tables + `store_entity` / `store_relation` | Schema migration, entity/edge CRUD, LLM entity extraction pipeline | Entities and edges populate correctly from conversations |
| **v3.0-beta** | Bi-temporal + `query_graph` (BFS) | `valid_from`/`valid_to` extraction, temporal queries, Strategy 5 in recall cascade | Temporal queries return correct results; LOCOMO/LongMemEval scores measured |
| **v3.0** | Full memory evolution (memify) | Retroactive edge updates, stale node pruning, connection strengthening | Benchmark regression test vs v3.0-beta; no precision loss from pruning |

Each sub-phase is independently deployable. v3.0-alpha can ship without temporal
queries; v3.0-beta can ship without memory evolution. This ensures the system remains
functional at every checkpoint and allows course correction based on benchmark results.

---

#### ~~Semantic Cache~~ — **Cancelled**

Previously planned as recall-level caching (cosine ≥ 0.95 → return cached result set).
Cancelled because: (1) `tool.embedding` LRU cache already eliminates the main bottleneck
(embedding generation), (2) DB scan + 500× dot product is sub-millisecond with NumPy,
(3) cache invalidation on every `store`/`delete` adds complexity with minimal gain,
(4) incompatible with v2.4 recency-weighted scoring (cached results become stale as time passes).

### Phase 4: Anti-Contamination — **Partially Completed** (v0.5.9)

- [x] **Timestamp annotations** — `_format_memory_timestamp()` converts ISO-8601 timestamps to human-readable local time; recall prepends `[Memory from YYYY-MM-DD HH:MM TZ]` to each memory
- [x] **Boundary markers** — `[Episode]` prefix on episode summaries, `[Profile]` prefix on profile entries in recall results; `source: {"System": "episode"}` / `{"System": "profile"}` for semantic distinction
- [ ] **Explicit anti-hallucination guardrails** — not yet implemented (no system-prompt-level instructions to LLM about memory provenance)

---

## 10. Memory Footprint Summary

| Configuration | CPersona | Embedding | Total | Search Quality |
|--------------|------|-----------|-------|---------------|
| `none` (FTS5 only) | ~40MB | — | **~40MB** | Good (FTS5 + keyword) |
| `http` + ONNX MiniLM | ~40MB | ~490MB | **~530MB** | Excellent (vector + FTS5) |
| `http` + API provider | ~40MB | ~40MB | **~80MB** | Excellent (vector + FTS5) |
| `api` (no embedding server) | ~40MB | — | **~40MB** | Excellent (vector + FTS5) |
