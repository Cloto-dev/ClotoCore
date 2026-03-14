# MCP/MGP Plugin Architecture (v2)

> **Status:** Implemented (v0.5.3)
> **Supersedes:** Three-Tier Plugin Model (Rust/Python Bridge/WASM) → Two-Layer Model (Rust Core + MCP/MGP)
> **Related:** `ARCHITECTURE.md` Section 3, [MGP_SPEC.md](MGP_SPEC.md)
>
> **Note:** ClotoCore implements **MGP (Model General Protocol)**, a strict superset of MCP.
> All MCP servers are MGP-compliant. MGP adds trust levels, isolation policies, handshake
> extensions, and event forwarding on top of the standard MCP protocol.
> See [MGP_SPEC.md](MGP_SPEC.md) for the full specification and
> [cloto-mcp-servers](https://github.com/Cloto-dev/cloto-mcp-servers) for detailed MGP documentation.

---

## 1. Motivation

### 1.1 Current Challenges

ClotoCore's plugin system was originally designed as a Three-Tier Model:

| Tier | Status | Maintenance Cost |
|------|--------|-----------------|
| Tier 1: Rust Native | Running (6 plugins) | SDK, macros, inventory, registry, factory, cast |
| Tier 2: Python Bridge | Removed | - |
| Tier 3: WASM | Not implemented (design docs only) | - |

For a solo developer, the maintenance burden of the Rust Plugin SDK is significant:

- `crates/shared/` — 5 plugin trait definitions
- `crates/macros/` — `#[cloto_plugin]` procedural macro
- `plugins/` — 6 Rust plugin implementations
- `managers/plugin.rs` — PluginManager (factory, bootstrap, capability injection)
- `managers/registry.rs` — PluginRegistry (dispatch, timeout, semaphore)
- `inventory` crate — compile-time auto-registration
- Magic Seal verification (`0x56455253`)
- Capability Injection (SafeHttpClient, FileCapability, ProcessCapability)

### 1.2 Design Decision

**Abolish the Rust Plugin SDK entirely and adopt MCP (Model Context Protocol) as the sole plugin standard.**

- **The core sells "trust" in Rust, while plugins open the "gateway" via MCP**
- The Kernel specializes as an MCP client and orchestrator
- The ultimate realization of Design Principle 1.1 (Core Minimalism)

### 1.3 Selection Rationale

Results of evaluating alternatives:

| Option | Configuration | Verdict | Reason |
|--------|---------------|---------|--------|
| A | Rust + MCP | **Adopted** | Maintainability, dynamic generation, ecosystem, implementation cost |
| B | Rust + WASM (MCP interface) | Rejected | High initial implementation cost, difficult dynamic generation |
| C | Rust + WASM + MCP (hybrid) | Rejected | Reverts to 3 layers, degraded maintainability |
| D | Go + MCP | Rejected | Loss of Tauri, rewrite cost |
| E | Rust + Go + MCP | Rejected | Two-language maintenance, excessive for ~350 lines |

---

## 2. Architecture

### 2.1 Two-Layer Model

```
┌──────────────────────────────────────────────────────┐
│  Layer 1: Rust Core (Kernel)                          │
│                                                        │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────┐ │
│  │ Axum     │ │ SQLite   │ │ Event    │ │ Tauri   │ │
│  │ HTTP     │ │ Database │ │ Bus      │ │ Desktop │ │
│  └──────────┘ └──────────┘ └──────────┘ └─────────┘ │
│  ┌──────────────────────────────────────────────────┐ │
│  │ MCP Client Manager                               │ │
│  │  - Server lifecycle (spawn / stop / restart)      │ │
│  │  - Tool routing & dispatch                        │ │
│  │  - Manifest management                            │ │
│  │  - Magic Seal verification (HMAC)                 │ │
│  │  - Event → MCP Notification forwarding            │ │
│  └──────────────────────────────────────────────────┘ │
│  ┌──────────────────────────────────────────────────┐ │
│  │ Chat Pipeline                                     │ │
│  │  - MCP Tool "think" invocation (formerly ReasoningEngine) │
│  │  - MCP Tool "store" / "recall" (formerly MemoryProvider)  │
│  └──────────────────────────────────────────────────┘ │
│  ┌──────────────────────────────────────────────────┐ │
│  │ Evolution Engine (archived)                         │ │
│  └──────────────────────────────────────────────────┘ │
└───────────────────────┬──────────────────────────────┘
                        │
                        │ MCP (JSON-RPC 2.0 over stdio)
                        │
┌───────────────────────▼──────────────────────────────┐
│  Layer 2: MCP Servers (any language)                   │
│                                                        │
│  Repository: cloto-mcp-servers (github.com/Cloto-dev/cloto-mcp-servers) │
│                                                        │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────────┐ │
│  │ mind.*      │ │ memory.*    │ │ tool.*          │ │
│  │ (Reasoning) │ │ (Memory)    │ │ (Execution)     │ │
│  │ deepseek    │ │ cpersona    │ │ terminal        │ │
│  │ cerebras    │ │             │ │ websearch       │ │
│  │ claude      │ │             │ │ research        │ │
│  │ ollama      │ │             │ │ cron, embedding │ │
│  └─────────────┘ └─────────────┘ └─────────────────┘ │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────────┐ │
│  │ vision.*    │ │ voice.*     │ │ output.*        │ │
│  │ capture     │ │ stt         │ │ avatar          │ │
│  │ gaze_webcam │ │             │ │                 │ │
│  └─────────────┘ └─────────────┘ └─────────────────┘ │
└──────────────────────────────────────────────────────┘
```

### 2.2 Request Flow

```
User Message
  │
  ├─ POST /api/chat
  │    │
  │    ├─ Kernel: Chat Pipeline
  │    │    │
  │    │    ├─ MCP Client Manager → invoke mind.deepseek "think" Tool
  │    │    │    └─ JSON-RPC: {"method": "tools/call", "params": {"name": "think", ...}}
  │    │    │    └─ Response: {"result": {"content": [{"type": "text", "text": "..."}]}}
  │    │    │
  │    │    ├─ MCP Client Manager → invoke memory.cpersona "store" Tool
  │    │    │
  │    │    └─ Event Bus → SSE broadcast
  │    │
  │    └─ JSON Response → Client
```

### 2.3 Event Flow

> For the full event processing pipeline, see `ARCHITECTURE.md` § 0.3.

The MCP Client Manager receives Kernel Events and forwards them to all MCP Servers as `notifications/cloto.event`.

---

## 3. MCP Protocol Usage

### 3.1 Leveraging Standard MCP Features

| MCP Primitive | Usage in ClotoCore |
|---------------|--------------|
| **Tools** | Primary representation of plugin functionality (think, store, recall, execute_command) |
| **Resources** | Read-only data exposure (metrics, status) |
| **Prompts** | Template prompts (future extension) |
| **Notifications** | Kernel → Server event forwarding |

### 3.2 ClotoCore-Specific Extensions (Custom Methods)

While maximizing the use of the MCP standard, the following ClotoCore-specific methods are defined:

| Method | Direction | Purpose |
|--------|-----------|---------|
| `cloto/handshake` | Client → Server | Manifest exchange + Magic Seal verification |
| `cloto/shutdown` | Client → Server | Graceful shutdown request |

**Notification (Server → Client):**

| Notification | Purpose |
|-------------|---------|
| `notifications/cloto.event` | Forwarding of Kernel events |
| `notifications/cloto.config_updated` | Notification of plugin configuration changes |

### 3.3 Legacy Trait to MCP Tool Mapping

#### ReasoningEngine → MCP Tools

```json
{
  "name": "think",
  "description": "Process a message and generate a response",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": { "type": "string" },
      "message": { "type": "string" },
      "context": {
        "type": "array",
        "items": { "type": "object" }
      }
    },
    "required": ["agent_id", "message"]
  }
}
```

```json
{
  "name": "think_with_tools",
  "description": "Process a message with available tool schemas",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": { "type": "string" },
      "message": { "type": "string" },
      "context": { "type": "array" },
      "tools": { "type": "array" },
      "tool_history": { "type": "array" }
    },
    "required": ["agent_id", "message"]
  }
}
```

#### MemoryProvider → MCP Tools

```json
{
  "name": "store",
  "description": "Store a message in agent memory",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": { "type": "string" },
      "message": { "type": "object" }
    },
    "required": ["agent_id", "message"]
  }
}
```

```json
{
  "name": "recall",
  "description": "Recall relevant memories for a query",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": { "type": "string" },
      "query": { "type": "string" },
      "limit": { "type": "integer", "default": 10 }
    },
    "required": ["agent_id", "query"]
  }
}
```

#### Tool → MCP Tools

```json
{
  "name": "execute_command",
  "description": "Execute a shell command in sandboxed directory",
  "inputSchema": {
    "type": "object",
    "properties": {
      "command": { "type": "string" },
      "args": { "type": "array", "items": { "type": "string" } },
      "timeout_secs": { "type": "integer", "default": 120 }
    },
    "required": ["command"]
  }
}
```

---

## 4. MCP Server Manifest

### 4.1 Manifest Structure

Each MCP Server returns the following manifest via `cloto/handshake`:

```json
{
  "id": "mind.deepseek",
  "name": "DeepSeek Reasoning Engine",
  "description": "DeepSeek API reasoning engine with R1 support",
  "version": "0.1.0",
  "sdk_version": "0.1.0",
  "category": "Agent",
  "service_type": "Reasoning",
  "tags": ["#MIND", "#LLM"],
  "required_permissions": ["NetworkAccess"],
  "provided_capabilities": ["Reasoning"],
  "provided_tools": ["think", "think_with_tools"],
  "seal": "<HMAC-SHA256 signature>"
}
```

### 4.2 Naming Convention (Retained)

| Namespace | Purpose | Examples |
|-----------|---------|---------|
| `mind.*` | Reasoning engines (LLM) | `mind.deepseek`, `mind.cerebras` |
| `memory.*` | Memory and knowledge management | `memory.cpersona` |
| `tool.*` | Tool execution | `tool.terminal`, `tool.embedding`, `tool.web-search` |
| `adapter.*` | External protocol bridges | `adapter.discord`, `adapter.slack` |
| `vision.*` | Vision / perception | `vision.capture`, `vision.gaze_webcam` |
| `voice.*` | Voice I/O | `voice.stt` |
| `output.*` | Output rendering | `output.avatar` |
| `hal.*` | Hardware abstraction | `hal.audio`, `hal.gpio` |

---

## 5. Magic Seal (MCP)

### 5.1 Legacy Method (Deprecated)

```rust
// Rust compile-time constant — scheduled for deprecation
magic_seal: 0x56455253  // ASCII: "VERS"
```

### 5.2 New Method: HMAC-Signed Manifest

```
MCP Server starts → Kernel invokes cloto/handshake
                  → Server returns manifest + HMAC signature
                  → Kernel verifies the HMAC
                  → Verification succeeds → Connection established
                  → Verification fails → Connection refused
```

**Signature Generation:**

```
seal = HMAC-SHA256(
  key  = CLOTO_SDK_SECRET,
  data = canonical_json(manifest without "seal" field)
)
```

**CLOTO_SDK_SECRET:**

- Embedded in the ClotoCore MCP SDK package
- Lightweight proof that the official SDK was used
- Not cryptographic tamper prevention, but a "declaration of trust" (equivalent to the legacy Magic Seal)

### 5.3 Unsigned Mode

During development, signature verification can be skipped with `CLOTO_ALLOW_UNSIGNED=true`.
In production, signatures are required by default.

---

## 6. Dynamic Plugin Creation (L5)

### 6.1 Autonomous Generation by Agents

```
Agent (L5 Autonomy)
  │
  ├─ 1. Generate MCP Server code
  │      Using Python + cloto-mcp-sdk
  │      Tool definitions + business logic
  │
  ├─ 2. Kernel validates the code
  │      - AST security inspection
  │      - Manifest validity check
  │      - Permission request validation
  │
  ├─ 3. Kernel spawns the MCP Server as a subprocess
  │      - stdio transport (process isolation)
  │      - MCP handshake + Magic Seal verification
  │
  ├─ 4. Tools are registered with the Kernel
  │      - Available in the Chat Pipeline
  │      - Displayed on the Dashboard
  │
  └─ 5. Disposed of when no longer needed
        - Process kill + deregistration
```

### 6.2 MCP Server Management API

| Method | Route | Description |
|--------|-------|-------------|
| GET | `/api/mcp/servers` | List registered MCP Servers |
| POST | `/api/mcp/servers` | Register an MCP Server (manual / dynamic) |
| DELETE | `/api/mcp/servers/:id` | Stop and deregister an MCP Server |
| POST | `/api/mcp/servers/:id/restart` | Restart an MCP Server |

---

## 7. MCP Client Manager

### 7.1 Overview

Promote the current `adapter.mcp` plugin to a Kernel core feature.

**Responsibilities:**

1. MCP Server lifecycle management (spawn / monitor / restart / stop)
2. MCP JSON-RPC client (Tool invocation, Notification sending)
3. Tool routing (Tool name → dispatch to the appropriate MCP Server)
4. Manifest management + Magic Seal verification
5. Kernel Event → MCP Notification conversion and forwarding
6. Server health monitoring + automatic restart

### 7.2 Configuration

MCP/MGP servers are configured in `mcp.toml` at the project root. The `[paths]` section
defines named path variables that are expanded as `${var}` in `command` and `args` fields.

```toml
# Path variables — expanded in command/args before relative path resolution.
# Values may reference environment variables: servers = "${CLOTO_MCP_SERVERS}"
[paths]
servers = "C:/Users/Cycia/source/repos/cloto-mcp-servers/servers"

[[servers]]
id = "mind.deepseek"
display_name = "DeepSeek"
command = "python"
args = ["${servers}/deepseek/server.py"]
transport = "stdio"
auto_restart = true
[servers.env]
DEEPSEEK_API_URL = "http://127.0.0.1:8082/v1/chat/completions"
DEEPSEEK_PROVIDER = "deepseek"

[[servers]]
id = "tool.terminal"
command = "python"
args = ["${servers}/terminal/server.py"]
transport = "stdio"
auto_restart = true
[servers.tool_validators]
execute_command = "sandbox"
```

**MGP isolation fields** (optional per-server, see MGP §8-10):

```toml
[[servers]]
id = "example.server"
seal = "sha256:..."                   # HMAC-SHA256 integrity seal (L0 Magic Seal)
[servers.mgp]
trust_level = "standard"             # core | standard | experimental | untrusted
[servers.isolation]                  # Override defaults derived from trust_level
memory_limit_mb = 512
filesystem_scope = "sandbox"         # unrestricted | sandbox | readonly | none
network_scope = "proxyonly"          # unrestricted | proxyonly | none
```

---

## 8. Security Model

### 8.1 Changes

| Feature | Rust Native (Old) | MCP (New) | Impact |
|---------|-------------------|-----------|--------|
| SafeHttpClient | Injected by Kernel | Self-managed by MCP Server | Security reduction |
| FileCapability | Sandboxed | OS-level restrictions | Equivalent (implementation-dependent) |
| ProcessCapability | Allowlist enforced | Restricted within MCP Server | Security reduction |
| Memory isolation | Rust type safety | Process isolation | Equivalent |
| Magic Seal | Compile-time constant | HMAC signature | Equivalent |

### 8.2 Mitigations

- **Solo development assumption**: All MCP Servers are self-authored → trust assumption holds
- **Third-party support**: Consider introducing OS-level sandboxing (seccomp, AppArmor)
- **Magic Seal**: Can reject connections from MCP Servers not using the official SDK

### 8.3 Preserved Safety Mechanisms

- API Key authentication (Dashboard <-> Kernel)
- Event depth check (cascade prevention, max 5)
- Rate limiter (HTTP request throttling)
- CORS origin restrictions
- MCP Server process isolation (OS level)

---

## 9. Deprecated Components

Components removed or archived as part of the MCP migration:

| Component | Status | Notes |
|-----------|--------|-------|
| Plugin SDK (traits) | Trait definitions remain in `crates/shared/` | Replaced by MCP |
| Plugin Macros (`crates/macros/`) | **Completed** — Removed | Replaced by MCP manifests |
| Plugin Implementations (`plugins/`) | **Completed** — Removed | Reimplemented as MCP Servers |
| inventory crate | **Completed** — Removed | No longer needed |
| WASM Plugin Design | **Archived** — `docs/archive/` | Historical reference material |
| PluginManager / PluginRegistry | Migrating to MCP Client Manager | Remains in `managers/` |
| Capability Injection | Migrating to MCP Server self-management | Remains in `capabilities.rs` |
| Magic Seal 0x56455253 | Migrating to HMAC signatures | Legacy constant scheduled for removal |

---

## 10. Migration Plan

### Phase 1: tool.terminal → MCP Server — **Completed**

- [x] Reimplemented `tool.terminal` as a Python MCP Server
- [x] Basic implementation of MCP Client Manager
- [x] Kernel can invoke MCP Tool `execute_command`

### Phase 2: mind.deepseek → MCP Server — **Completed**

- [x] Reimplemented `mind.deepseek` as a Python MCP Server
- [x] Changed Chat Pipeline to use MCP Tool `think` invocation
- [x] Verified `think_with_tools` operation as an MCP Tool

### Phase 3: Remaining Plugin Migration — **Completed**

- [x] `mind.cerebras` → MCP Server
- [x] `memory.cpersona` → MCP Server (store/recall Tools)
- [x] `tool.embedding` → MCP Server

### Phase 4: Rust Plugin SDK Removal — **Partial**

- [x] Removed `crates/macros/`
- [x] Removed `plugins/` directory → migrated to MCP servers
- [x] Removed `inventory` crate dependency
- [ ] Remove plugin traits from `crates/shared/` (trait definitions still remain)
- [ ] Complete removal of PluginManager and PluginRegistry

### Phase 5: Dynamic Plugin Generation — **Pending**

1. Magic Seal (HMAC) implementation
2. Implementation of autonomous MCP Server generation by Agent L5
3. Dashboard MCP Server management UI — **Completed** (v0.5.3)

---

## 11. Trade-offs (Accepted)

| Loss | Impact | Reason for Acceptance |
|------|--------|----------------------|
| Capability Injection | Medium | Solo development — all Servers are self-authored → trust assumption holds |
| Compile-time type safety | Low | Type validation at JSON-RPC boundaries is ensured by the MCP SDK |
| JSON-RPC overhead | Negligible | LLM API calls take hundreds of ms; a few ms of IPC is within margin of error |
| Zero-copy event dispatch | Low | Event frequency is low (a few per second or less) |

---

## 12. Server Repository

As of v0.5.4, all MCP/MGP server implementations are maintained in a separate repository:

- **Repository:** [cloto-mcp-servers](https://github.com/Cloto-dev/cloto-mcp-servers)
- **Contents:** 16 Python MCP servers + common library + tests + MGP documentation
- **Integration:** `mcp.toml` `[paths].servers` points to the local clone
- **License:** BSL 1.1 (CPersona and MGP Protocol individually MIT-licensed)

The Kernel references servers via path variables in `mcp.toml`. There is zero compile-time
coupling — communication is exclusively via JSON-RPC over stdio.

**Migration plan (D → C):** The current approach (D) uses file-path-based server
resolution. The future approach (C) will use Python package-based invocation
(`python -m cloto_mcp_servers.terminal`), eliminating path configuration entirely.

---

## 13. Future Considerations

- **MGP Evolution**: MGP spec continues to evolve; see [MGP_SPEC.md](MGP_SPEC.md) for current state
- **ClotoCore MCP SDK**: Official SDK packages for Python / Node / Rust
- **MCP Server Marketplace**: Distribution platform for community-built MCP Servers
- **MCP Sampling**: Standardize Kernel-side reasoning invocation after the MCP Sampling feature matures
- **Third-party Support**: Introduce OS-level sandboxing (seccomp / AppArmor) per MGP isolation design
