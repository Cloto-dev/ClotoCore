# CPersona Memory System — MCP Server Design

> **Status:** Implemented (Phase 1-4 complete as of v0.5.9)
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
| CPersona 2.3 | ClotoCore | Dedicated SQLite (`data/cpersona.db`) | FTS5 + vector (pluggable) | LLM-powered (Phase 3) + anti-contamination (Phase 4) | **Current** |
| CPersona 2.4 | ClotoCore | Dedicated SQLite (`data/cpersona.db`) | FTS5 + vector + recency-weighted scoring | LLM-powered + anti-contamination + gated recency boost | **Planned** |
| CPersona 2.5 | ClotoCore | Dedicated SQLite (`data/cpersona.db`) | FTS5 + vector + recency + RRF reranking | LLM-powered + anti-contamination + profile enrichment | **Planned** |
| CPersona 3.0 | ClotoCore | SQLite + graph tables (nodes/edges) | Cascade + graph traversal (BFS) + bi-temporal | LLM-powered + anti-contamination + memory evolution (full) | **Planned** |

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
| **Background Task Queue** | DB-persisted with crash recovery | None | None | Not yet restored |
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
│  Tools:              │  │  Tools:                            │
│  - store             │  │  - embed (batch, max 100)          │
│  - recall            │  │                                    │
│  - update_profile    │  │  Providers:                        │
│  - archive_episode   │  │  - onnx_miniml (local, ~490MB)     │
│  - list_memories     │  │  - api_openai  (remote, ~40MB)    │
│  - list_episodes     │  │  - api_deepseek (remote, ~40MB)   │
│  - delete_memory     │  │                                    │
│  - delete_episode    │  │  HTTP: localhost:PORT/embed        │
│  - delete_agent_data │  │  (lightweight internal endpoint)   │
│  - get_queue_status  │  │                                    │
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
| `api` | CPersona calls external API directly (OpenAI/DeepSeek) | ~40MB | Not required |
| `local` | CPersona loads ONNX model in-process | ~490MB | Not required |
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

**Response:** `{"messages": [{"id": "...", "content": "[Memory from 2026-03-13 14:30 JST] ...", "source": {...}, "timestamp": "..."}]}`

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
  └─ 4. Keyword Fallback (2.2-compatible)
        Condition: remaining slots available after strategies 1-3
        → Keyword match on memories.content (LIKE)
        → Chronological ordering (newest first)
        → Max rows scanned: CPERSONA_MAX_MEMORIES (OOM guard)

  → Merge all results, deduplicate by seen_ids set, sort by relevance
  → Truncate to limit, reverse to chronological order for LLM context
  → Apply Phase 4 timestamp annotations: [Memory from YYYY-MM-DD HH:MM TZ]
```

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
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_episodes_agent
    ON episodes(agent_id, created_at DESC);

-- FTS5 full-text search index
CREATE VIRTUAL TABLE episodes_fts USING fts5(
    summary,
    keywords,
    content=episodes,
    content_rowid=id
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
| `CPERSONA_EMBEDDING_MODE` | `none` | Embedding strategy: `http`, `api`, `local`, `none` |
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

### CPersona 2.4+ Roadmap

#### Recency-Weighted Vector Search (Planned — v2.4 scope)

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

##### 1. Reciprocal Rank Fusion (RRF) Reranking

Current recall merges results from 4 strategies by deduplication + relevance sort.
RRF provides a principled fusion method that is backend-agnostic and parameter-free
(no training required).

```python
# Applied after all 4 cascade strategies produce their ranked lists
RRF_K = 60  # Standard constant (Cormack et al., 2009)

def rrf_score(rank: int) -> float:
    return 1.0 / (RRF_K + rank)

# Each strategy produces a ranked list; RRF merges them:
# final_score(doc) = Σ rrf_score(rank_in_strategy_i) for each strategy that returned doc
```

**Why RRF over alternatives:**
- Cross-encoder reranking requires an additional model (latency + memory cost)
- MMR requires tuning λ diversity parameter per domain
- RRF is zero-parameter, works across heterogeneous score scales (cosine vs BM25 vs recency)
- Proven effective: used by Zep/Graphiti as one of their 5 reranking options

**Environment variables:**

| Variable | Default | Description |
|----------|---------|-------------|
| `CPERSONA_RRF_ENABLED` | `false` | Enable RRF reranking (opt-in) |
| `CPERSONA_RRF_K` | `60` | RRF constant (higher = more weight to lower-ranked results) |

**Implementation scope:** `cloto-mcp-servers` repo (`servers/cpersona/server.py`,
`recall()` function). Insert RRF merge step between strategy collection and final
truncation. No schema changes.

##### 2. Profile Enrichment (Limited Memory Evolution)

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
- Baseline scores for v2.3, re-measured after v2.4 and v2.5 changes
- Results documented in this file (new Section 11: Benchmark Results)

---

#### CPersona 3.0 Roadmap — Graph Memory Paradigm

**Theme:** Architectural shift from flat memory to structured knowledge graph.
This is the pathway to closing the gap with Zep/Graphiti, Mem0, and Cognee.

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

**Integration with anti-contamination:** Temporal annotations extended to include
validity period: `[Memory from 2024-01, valid until 2025-06]`.

##### 3. Full Memory Evolution

Extension of v2.5's profile enrichment to the graph level:
- New information triggers **retroactive edge updates** (A-MEM style)
- LLM evaluates whether new facts modify existing entity attributes or create new edges
- Cognee's memify concept: periodically prune stale nodes, strengthen frequent connections

**Dependency:** Requires graph memory (item 1) to be implemented first.

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
| `local` ONNX (no server) | ~490MB | — | **~490MB** | Excellent (vector + FTS5) |
