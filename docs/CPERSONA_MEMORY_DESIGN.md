# CPersona Memory System — MCP Server Design

> **Status:** Implemented (Phase 1-4 complete as of v0.5.9)
> **Related:** `MCP_PLUGIN_ARCHITECTURE.md`, `ARCHITECTURE.md` Section 3
> **MCP Server ID:** `memory.cpersona`
> **Companion Server:** `memory.embedding` (pluggable vector embedding)

---

## 1. Background

### 1.1 Evolution History

| Version | Project | Storage | Search | Memory Extraction | Status |
|---------|---------|---------|--------|-------------------|--------|
| CPersona 2.0/2.1 | ai_karin | SQLite (WAL, FTS5, vector) | FTS5 + cosine similarity + semantic cache | LLM-powered (DeepSeek Reasoner): profile extraction, episode archival | Reference implementation |
| CPersona 2.2 | ClotoCore | plugin_data (key-value via SAL) | `LIKE '%keyword%'` | None | Deprecated (Rust plugin) |
| CPersona 2.3 | ClotoCore | Dedicated SQLite (`data/cpersona.db`) | FTS5 + vector (pluggable) | LLM-powered (Phase 3) + anti-contamination (Phase 4) | **Current** |

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
│  memory.cpersona           │  │  memory.embedding                    │
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
│                      │  │                                    │
│  DB: cpersona.db  │  │  Embedding Cache:                  │
│  (SQLite, FTS5)      │  │  LRU (256 entries) + TTL (300s)   │
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

### 4.4 Schema Versioning

The CPersona MCP server manages its own schema migrations at startup using a simple
version table:

```sql
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

---

## 5. Embedding Server (`memory.embedding`)

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
[[servers]]
id = "memory.embedding"
command = "python"
args = ["-m", "cloto_mcp_embedding"]
transport = "stdio"
auto_restart = true
env = { EMBEDDING_PROVIDER = "onnx_miniml", EMBEDDING_HTTP_PORT = "8401" }

[[servers]]
id = "memory.cpersona"
command = "python"
args = ["-m", "cloto_mcp_cpersona"]
transport = "stdio"
auto_restart = true
env = {
    CPERSONA_DB_PATH = "data/cpersona.db",
    CPERSONA_EMBEDDING_MODE = "http",
    CPERSONA_EMBEDDING_URL = "http://127.0.0.1:8401/embed"
}
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

- [x] `mcp-servers/cpersona/server.py` with `store` and `recall` tools
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
- [ ] Background task queue (DB-persisted, crash-recoverable) — deferred
- [ ] Semantic cache (high-confidence recall caching) — deferred

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
