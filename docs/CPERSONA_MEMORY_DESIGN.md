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
│  - store             │  │  - embed                           │
│  - recall            │  │  - embed_batch                     │
│  - update_profile    │  │                                    │
│  - archive_episode   │  │  Providers:                        │
│                      │  │  - onnx_miniml (local, ~490MB)     │
│  DB: cpersona.db  │  │  - api_openai  (remote, ~40MB)    │
│  (SQLite, FTS5)      │  │  - api_deepseek (remote, ~40MB)   │
│                      │  │                                    │
│  Embedding Client ───┼──┤  HTTP: localhost:PORT/embed        │
│  (http/api/none)     │  │  (lightweight internal endpoint)   │
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

**Response:** `{"ok": true}` or `{"error": "..."}`

**Behavior:**
1. Insert message into `memories` table
2. If embedding provider is available, compute embedding and store it
3. Return immediately (no LLM call)

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

**Response:** `{"messages": [{"id": "...", "content": "...", "source": {...}, "timestamp": "..."}]}`

**Search Strategy (cascading):**

```
recall(agent_id, query, limit)
  │
  ├─ 1. Vector Search (if embedding available)
  │     → Compute query embedding
  │     → Cosine similarity on memories.embedding + episodes.embedding
  │     → Return top-K candidates with scores
  │
  ├─ 2. FTS5 Full-Text Search
  │     → Query episodes_fts with AND-matched keywords
  │     → Return ranked results
  │
  ├─ 3. Profile Lookup
  │     → Fetch profiles for this agent_id
  │     → Include as contextual information
  │
  └─ 4. Recent Memory Fallback (2.2-compatible)
        → Keyword match on memories.content (LIKE)
        → Chronological ordering (newest first)

  → Merge all results, deduplicate, sort by relevance
  → Truncate to limit, reverse to chronological order for LLM context
```

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

**Behavior (ported from 2.1):**
1. Extract user facts from conversation using LLM (requires external reasoning engine)
2. Merge with existing profile in `profiles` table (UPSERT)
3. Runs as foreground operation (caller may fire-and-forget)

> **Phase 1:** Stub — stores raw history summary without LLM extraction.
> **Phase 2:** Full 2.1 port — LLM-powered extraction via external API.

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

**Behavior (ported from 2.1):**
1. Concatenate history into text
2. Generate summary + keywords (requires external reasoning engine or simple heuristic)
3. Compute embedding if provider available
4. Insert into `episodes` table + FTS5 index

> **Phase 1:** Simple concatenation summary + keyword extraction (no LLM).
> **Phase 2:** LLM-powered summarization.

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
        "description": "Texts to embed (batch)"
      }
    },
    "required": ["texts"]
  }
}
```

**Response:** `{"embeddings": [[0.012, -0.034, ...], ...], "dimensions": 384}`

### 5.3 HTTP Endpoint

For CPersona direct access (bypasses kernel MCP routing):

```
POST http://127.0.0.1:{HTTP_PORT}/embed
Content-Type: application/json

{"texts": ["hello world", "test query"]}

→ {"embeddings": [[...], [...]], "dimensions": 384}
```

### 5.4 Providers

| Provider | Model | Dimensions | Memory | Latency | Cost |
|----------|-------|-----------|--------|---------|------|
| `onnx_miniml` | all-MiniLM-L6-v2 (ONNX) | 384 | ~490MB | <10ms/text | Free |
| `api_openai` | text-embedding-3-small | 1536 | ~40MB | ~100ms/text | $0.02/1M tokens |
| `api_deepseek` | (if available) | TBD | ~40MB | ~100ms/text | TBD |

Configured via environment variable:

```
EMBEDDING_PROVIDER=onnx_miniml    # or api_openai, api_deepseek
EMBEDDING_MODEL=all-MiniLM-L6-v2  # provider-specific model name
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
| `CPERSONA_EMBEDDING_API_URL` | — | API endpoint for `api` mode |
| `CPERSONA_EMBEDDING_MODEL` | `all-MiniLM-L6-v2` | Model name for `local` mode |
| `CPERSONA_MAX_MEMORIES` | `500` | Max memories loaded per recall (OOM guard) |
| `CPERSONA_FTS_ENABLED` | `true` | Enable FTS5 episode search |

---

## 7. Kernel Integration (Memory Resolver)

### 7.1 Current Flow (Rust Plugin)

```rust
// system.rs — current implementation
let memory_plugin = registry.find_memory().await;  // Arc<dyn Plugin>
if let Some(mem) = plugin.as_memory() {
    let context = mem.recall(agent_id, query, limit).await?;
    // ... agentic loop ...
    mem.store(agent_id, message).await?;
}
```

### 7.2 New Flow (MCP Dual Dispatch)

```rust
// system.rs — after CPersona MCP migration
// 1. Try Rust plugin first (backward compatible)
let memory_plugin = registry.find_memory().await;

// 2. Fallback: find MCP server with store+recall tools
let mcp_memory = if memory_plugin.is_none() {
    mcp_manager.find_memory_server().await  // checks for store+recall tools
} else {
    None
};

// 3. recall
let context = if let Some(ref plugin) = memory_plugin {
    plugin.as_memory().unwrap().recall(...).await?
} else if let Some(ref mcp) = mcp_memory {
    let result = mcp.call_tool("memory.cpersona", "recall", args).await?;
    parse_recall_result(&result)?  // JSON → Vec<ClotoMessage>
} else {
    vec![]
};

// 4. store (same pattern)
```

### 7.3 McpClientManager Extension

New method on `McpClientManager`:

```rust
/// Find an MCP server that provides memory capabilities (has both store and recall tools).
pub async fn find_memory_server(&self) -> Option<String> {
    let index = self.tool_index.read().await;
    let has_store = index.get("store").cloned();
    let has_recall = index.get("recall").cloned();
    match (has_store, has_recall) {
        (Some(s1), Some(s2)) if s1 == s2 => Some(s1),
        _ => None,
    }
}
```

---

## 8. Data Migration

### 8.1 From CPersona 2.2 (plugin_data)

Existing 2.2 data lives in `cloto_memories.db` → `plugin_data` table:

```
plugin_id = 'memory.cpersona'
key = 'mem:{agent_id}:{timestamp}:{hash}'
value = JSON(ClotoMessage)
```

Migration script (`mcp-servers/cpersona/migrate.py`):

1. Connect to source: `data/cloto_memories.db` → `plugin_data WHERE plugin_id='memory.cpersona'`
2. For each row, parse `key` to extract `agent_id` and `timestamp`
3. Deserialize `value` as ClotoMessage JSON
4. Insert into destination: `data/cpersona.db` → `memories` table
5. Optionally compute embeddings for migrated memories

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

---

## 10. Memory Footprint Summary

| Configuration | CPersona | Embedding | Total | Search Quality |
|--------------|------|-----------|-------|---------------|
| `none` (FTS5 only) | ~40MB | — | **~40MB** | Good (FTS5 + keyword) |
| `http` + ONNX MiniLM | ~40MB | ~490MB | **~530MB** | Excellent (vector + FTS5) |
| `http` + API provider | ~40MB | ~40MB | **~80MB** | Excellent (vector + FTS5) |
| `api` (no embedding server) | ~40MB | — | **~40MB** | Excellent (vector + FTS5) |
| `local` ONNX (no server) | ~490MB | — | **~490MB** | Excellent (vector + FTS5) |
