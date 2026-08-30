# Claude Code Integration: CPersona Memory

Connect CPersona (persistent memory) and the CEmbedding server to Claude Code
so that Claude can store, recall, and manage memories across sessions.

## Architecture

```
Claude Code
  ├─ cpersona MCP server    (store, recall, export, import, ...)
  │    └─ HTTP → CEmbedding server (port 8401)
  └─ cembedding MCP server  (local ONNX inference, HTTP endpoint)
```

Both servers run as stdio MCP processes. CPersona calls CEmbedding's
HTTP endpoint for vector operations — Claude Code launches both, so the
dependency resolves automatically.

> **Note:** Both servers now live in their own repositories —
> [Cloto-dev/CPersona](https://github.com/Cloto-dev/CPersona) (MIT, PyPI:
> [`cpersona`](https://pypi.org/project/cpersona/)) and
> [Cloto-dev/CEmbedding](https://github.com/Cloto-dev/CEmbedding) (MIT, PyPI:
> [`cembedding`](https://pypi.org/project/cembedding/)). The former
> `servers/embedding` copy in clotohub-servers was retired at 0.4.0 (2026-07-09).
> Canonical setup instructions are in each repository's README; this document
> is kept as the ClotoCore-side integration reference.

## Prerequisites

- Python 3.10+ and [`uv`](https://docs.astral.sh/uv/) (both servers install
  from PyPI via `uvx` — no repository clone or virtual environment needed)

## Claude Code Configuration

Register both MCP servers using `claude mcp add-json` (user scope):

```bash
# CEmbedding server (must start before CPersona for vector operations)
claude mcp add-json cembedding '{
  "type": "stdio",
  "command": "uvx",
  "args": ["--from", "cembedding[onnx]", "cembedding"],
  "env": {
    "EMBEDDING_PROVIDER": "onnx_miniml",
    "EMBEDDING_HTTP_PORT": "8401"
  }
}' -s user

# CPersona memory server
claude mcp add-json cpersona '{
  "type": "stdio",
  "command": "uvx",
  "args": ["cpersona"],
  "env": {
    "CPERSONA_DB_PATH": "~/.claude/cpersona.db",
    "CPERSONA_EMBEDDING_MODE": "http",
    "CPERSONA_EMBEDDING_URL": "http://127.0.0.1:8401/embed",
    "CPERSONA_TASK_QUEUE_ENABLED": "false"
  }
}' -s user
```

The first `uvx` launch downloads the packages; the first CEmbedding start also
downloads the ONNX model (`cembedding-download-model` pre-fetches it if you
prefer). Pin versions with `cembedding==0.6.0` / `cpersona==X.Y.Z` when you
need reproducible setups.

Verify with `claude mcp list` — both should show "Connected".

> **Note:** The `CPERSONA_DB_PATH` is intentionally separate from ClotoCore's
> database. Each environment (ClotoCore, Claude Code, mobile) maintains its own
> DB. Use `export_memories` / `import_memories` for portability.

## Available Tools

Once configured, Claude Code gains access to:

| Tool | Description |
|------|-------------|
| `store` | Store a message in agent memory |
| `recall` | Search memories (vector + FTS5 + keyword) |
| `update_profile` | Update user/agent profile |
| `get_profile` | Retrieve existing profile |
| `archive_episode` | Summarize and archive a conversation |
| `list_memories` | List recent memories |
| `list_episodes` | List archived episodes |
| `delete_memory` | Delete a single memory |
| `delete_episode` | Delete a single episode |
| `delete_agent_data` | Purge all data for an agent |
| `export_memories` | Export to JSONL file |
| `import_memories` | Import from JSONL file |
| `check_health` | Database health check (15 checks) with optional auto-repair |
| `get_queue_status` | Background task queue status |

## Environment Variables

### CPersona

| Variable | Default | Description |
|----------|---------|-------------|
| `CPERSONA_DB_PATH` | `data/cpersona.db` | SQLite database path |
| `CPERSONA_MAX_MEMORIES` | `500` | Max memories returned per search |
| `CPERSONA_FTS_ENABLED` | `true` | Enable FTS5 full-text search |
| `CPERSONA_EMBEDDING_MODE` | `none` | `none`, `http`, or `api` |
| `CPERSONA_EMBEDDING_URL` | _(empty)_ | HTTP embedding endpoint URL |
| `CPERSONA_EMBEDDING_API_KEY` | _(empty)_ | API key for `api` mode |
| `CPERSONA_EMBEDDING_API_URL` | `https://api.openai.com/v1/embeddings` | API endpoint for `api` mode |
| `CPERSONA_EMBEDDING_MODEL` | `text-embedding-3-small` | Model for `api` mode |
| `CPERSONA_VECTOR_MIN_SIMILARITY` | `0.3` | Cosine similarity threshold |
| `CPERSONA_TASK_QUEUE_ENABLED` | `true` | Enable background task queue |
| `CPERSONA_LLM_PROXY_URL` | `http://127.0.0.1:8082/v1/chat/completions` | LLM proxy for extraction |

### CEmbedding

| Variable | Default | Description |
|----------|---------|-------------|
| `EMBEDDING_PROVIDER` | `api_openai` | `onnx_miniml` / `onnx_jina_v5_nano` / `onnx_bge_m3` / `mlx_bge_m3` (local) or `api_openai` |
| `EMBEDDING_HTTP_PORT` | `8401` | HTTP server port |

The full variable list (vector index, `EMBEDDING_SEARCH_BACKEND`, ONNX tuning)
is in the [CEmbedding README](https://github.com/Cloto-dev/CEmbedding#configuration).

## Memory Portability (Export / Import)

### Export from ClotoCore → Claude Code

1. In the ClotoCore dashboard, go to **Memory Core**
2. Select an agent (or "All") and click the **Download** button
3. A `.jsonl` file is saved locally

Then import into Claude Code's CPersona:

```
Use import_memories with input_path "path/to/export.jsonl"
```

### Export from Claude Code → ClotoCore

```
Use export_memories with agent_id "" and output_path "claude_memories.jsonl"
```

Then import via the ClotoCore dashboard **Upload** button, or the REST API:

```bash
curl -X POST http://127.0.0.1:8081/api/memories/import \
  -H "X-API-Key: YOUR_KEY" \
  -H "Content-Type: application/json" \
  -d '{"data": "...jsonl content...", "agent_id": ""}'
```

### JSONL Format

Each line is a JSON object with a `_type` field:

```jsonl
{"_type":"header","version":"cpersona-export/1.0","agent_id":"","exported_at":"2026-03-24T...","memory_count":10,"episode_count":3,"has_profile":true}
{"_type":"memory","id":1,"agent_id":"agent.sapphy","content":"...","source":{},"timestamp":"...","created_at":"..."}
{"_type":"episode","id":1,"agent_id":"agent.sapphy","summary":"...","keywords":"...","start_time":"...","end_time":"..."}
{"_type":"profile","agent_id":"agent.sapphy","user_id":"","content":"...","updated_at":"..."}
```

- Embeddings are excluded by default (set `include_embeddings: true` to include as base64)
- Import is idempotent: memories with duplicate `msg_id` are skipped
- `target_agent_id` parameter remaps all records to a different agent

## Standalone Mode (No Embedding)

CPersona works without the CEmbedding server — set `CPERSONA_EMBEDDING_MODE=none`
(the default). Memory search falls back to FTS5 + keyword matching.

## Troubleshooting

- **"Connection refused" on port 8401**: Ensure the CEmbedding server is
  running. Claude Code must start it before CPersona attempts embedding calls.
  The very first `uvx` launch downloads the package and the ONNX model, so
  allow extra time (or pre-fetch with `uvx --from "cembedding[onnx]"
  cembedding-download-model`); later starts take a few seconds to load the
  model.

- **Task queue errors**: Set `CPERSONA_TASK_QUEUE_ENABLED=false` for Claude Code
  usage. The task queue is designed for the ClotoCore kernel's LLM proxy, which
  is not available in standalone mode.

- **LLM extraction not working**: `update_profile` and `archive_episode` use
  the kernel LLM proxy by default. In Claude Code standalone mode, they fall
  back to simpler heuristics (keyword extraction, concatenation).
