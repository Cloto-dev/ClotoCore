# MCP Server Startup Performance Analysis

**Analysis date**: 2026-03-17
**Subject**: MCP server connection flow during kernel startup

## Server lineup (mcp.toml)

| Server ID | Category | auto_restart | Startup tier |
|-----------|---------|-------------|---------|
| tool.terminal | Tool | true | Priority |
| tool.agent_utils | Tool | true | Priority |
| tool.cron | Tool | true | Priority |
| tool.embedding | Tool | true | Priority |
| tool.websearch | Tool | true | Priority |
| tool.research | Tool | true | Priority |
| mind.deepseek | Engine | true | Priority |
| mind.cerebras | Engine | true | Priority |
| mind.claude | Engine | true | Priority |
| mind.ollama | Engine | true | Priority |
| memory.cpersona | Memory | true | Priority |
| vision.gaze_webcam | Vision | false | Deferred |
| vision.capture | Vision | false | Deferred |
| tool.imagegen | Tool | false | Deferred |
| voice.stt | Voice | false | Deferred |

**Total**: 15 servers (11 Priority / 4 Deferred)
**All servers**: Python stdio

## Connection lifecycle (per server)

Steps inside `connect_server()` (`mcp.rs:462-1065`):

| # | Step | Duration | Timeout | Blocking |
|---|---------|---------|------------|-------------|
| 1 | Command validation | <1ms | — | No |
| 2 | Permission Gate D | 0-500ms (YOLO) | DB 10s | Yes |
| 3 | Seal verification | 0-500ms | — | Conditional |
| 4 | Isolation Profile | <5ms | — | No |
| 5 | **Process spawn** | **500ms-2s** | — | **Yes** |
| 6 | **Initialize RPC** | **1-5s** | **120s** | **Yes** |
| 7 | MGP Negotiation | <1ms | — | No |
| 8 | MGP Permission Flow | 0-500ms (YOLO) | 120s | Conditional |
| 9 | Initialized notification | <1ms | — | No |
| 10 | **tools/list RPC** | **100-500ms** | **120s** | **Yes** |
| 11 | Cloto Handshake | 100-500ms | 120s | Optional |
| 12 | Registration + indexing | <50ms | — | No |
| 13 | Capability Dispatcher | 10-50ms | — | No |
| 14 | Audit/Lifecycle | <1ms | — | No (spawn) |

**Per server, normal case**: 2-8 seconds
**Per server, worst case**: 360 seconds (3 retries × 120 s timeout)

## Startup timeline

### Scenario A: Cold start (venv missing)

```
  0s ─── Kernel start, Config/DB initialization
  2s ─── Plugin Manager initialization
  5s ─── ensure_mcp_venv() begins
         ├── python -m venv create: 1-3s
         ├── pip upgrade: 2-10s
         └── 15-server pip install (sequential): 30-75s ← ★ biggest bottleneck
 80s ─── Priority MCP connect (11 servers in parallel) ← already join_all parallelized
         ├── Process spawn: ~2s (slowest one)
         ├── Initialize RPC: ~2s
         └── tools/list: ~1s
 86s ─── HTTP server up → dashboard reachable
 86s ─── Deferred MCP connect (4 in background)
 92s ─── All servers connected
```

**Total**: **80-95 seconds** (bottleneck: venv dependency install)

### Scenario B: Normal start (venv present)

```
  0s ─── Kernel start, Config/DB initialization
  2s ─── Plugin Manager initialization
  5s ─── ensure_mcp_venv() — venv detected + dependency sync
         └── pip install --quiet × 15 (no-op): 15-30s ← ★ bottleneck
 35s ─── Priority MCP connect (11 in parallel)
         └── Parallel connect: ~5s
 40s ─── HTTP server up
 46s ─── All servers connected
```

**Total**: **35-46 seconds** (bottleneck: pip no-op check)

### Scenario C: Ideal start (after venv optimization)

```
  0s ─── Kernel start
  2s ─── Config/DB
  5s ─── venv check only (skip pip install)
  6s ─── Priority MCP connect (11 in parallel): ~5s
 11s ─── HTTP server up
 17s ─── All servers connected
```

**Total**: **11-17 seconds**

## Bottleneck analysis

### CRITICAL: venv dependency sync (`mcp_venv.rs:116-165`)

```rust
// install_server_deps() — sequentially pip-install every server
for entry in entries.flatten() {
    pip install <server_path> --quiet  // 2-5 s/server × 15 = 30-75 s
}
```

- **Runs on every startup** (even no-op pip dependency resolution takes 2-3 s/server)
- **Sequential** — not parallelized
- **All servers** — including the auto_restart=false ones

### HIGH: 120 s request timeout (`config.rs:302-310`)

```rust
CLOTO_MCP_REQUEST_TIMEOUT_SECS = 120  // default
```

- Applied to Initialize RPC, tools/list, permission RPC, all of them
- A single unresponsive server blocks for 120 s
- With 3 retries, up to 360 s

### HIGH: Python interpreter spawn (per server)

- Python venv `python.exe` spawn: 500ms-2s
- Import resolution (including shared libs): additional 500ms-1s
- Even with 11 parallel spawns, the slowest one paces the whole batch

### MODERATE: tools/list RPC

- Depends on the server's number of registered tools
- Usually 100-500ms; large tool sets may take 1-5s

## Recommended optimizations

### Priority HIGH

| # | Action | Expected savings | Implementation cost |
|---|------|---------|-----------|
| 1 | **Limit venv dependency sync to auto_restart servers** | -15-30s | low |
| 2 | **Parallelize pip install** (tokio::spawn × N) | -20-50s | low |
| 3 | **Move venv sync to background** (after HTTP up) | perceived -30-75s | medium |
| 4 | **Skip dependency sync after first install** (hash-based cache) | -15-30s | medium |

### Priority MEDIUM

| # | Action | Expected savings | Implementation cost |
|---|------|---------|-----------|
| 5 | **Shorten connection timeout to 30s** (startup only) | worst-case -270s | low |
| 6 | **Tool schema cache** (DB-backed, refetch only on change) | -1-5s | medium |
| 7 | **Python process pool** (pre-spawned) | -5-10s | high |

### Priority LOW

| # | Action | Expected savings | Implementation cost |
|---|------|---------|-----------|
| 8 | Parallelize seal verification | <1s | low |
| 9 | Batch Permission Gate D | <1s | medium |

## Timeout settings reference

| Setting | Default | Purpose | File |
|------|-----------|------|---------|
| `CLOTO_MCP_REQUEST_TIMEOUT_SECS` | 120s | All RPCs (initialize, tools/list, etc.) | config.rs:302 |
| `CLOTO_DB_TIMEOUT_SECS` | 10s | Permission DB check | config.rs:269 |
| `CLOTO_MEMORY_TIMEOUT_SECS` | 5s | Memory plugins | config.rs |
| `CLOTO_TOOL_TIMEOUT_SECS` | 30s | Tool execution | config.rs |
| Retry backoff | 1s, 2s | connect_server retries | mcp.rs:681 |

## Related files

- Startup orchestration: `crates/core/src/lib.rs:400-590`
- Server connect: `crates/core/src/managers/mcp.rs:311-356, 462-1065`
- MCP client: `crates/core/src/managers/mcp_client.rs:56-270`
- Transport: `crates/core/src/managers/mcp_transport.rs:111-249`
- Venv management: `crates/core/src/managers/mcp_venv.rs:116-267`
- Timeout configuration: `crates/core/src/config.rs:269-310`
