# ClotoCore Architecture

ClotoCore is an AI agent platform designed with a high degree of flexibility, safety, and extensibility. This document provides an integrated definition of design principles, the security framework, plugin communication mechanisms, and sub-project specifications.

---

## 0. System Overview

### 0.1 High-Level Architecture

```mermaid
graph TB
    Client[HTTP Client / Dashboard] --> Router[Axum Router]
    Router --> Auth[API Key Auth]
    Auth --> Handlers[Handler Functions]

    Handlers --> DB[(SQLite Database)]
    Handlers --> EventTx[Event Channel]

    EventTx --> Processor[Event Processor]
    Processor --> DepthCheck{Depth Check}
    DepthCheck -->|Valid| Manager[Plugin Manager]
    DepthCheck -->|Too Deep| Drop[Drop Event]

    Manager --> Timeout[Timeout Guard]
    Timeout --> Dispatch[Dispatch to Plugins]

    Dispatch --> MCP1[MCP Server]
    Dispatch --> MCPN[MCP Server N]

    MCP1 --> Cascade[Cascade Events]
    MCPN --> Cascade

    Cascade --> EventTx

    SSE[SSE Stream] --> Client

    style DB fill:#f0e6ff,stroke:#333
    style Processor fill:#e6f0ff,stroke:#333
    style Manager fill:#e6ffe6,stroke:#333
```

### 0.2 Request Flow

```
Client Request
  │
  ├─ GET /api/agents ──────────────────► get_agents() ──► DB Query ──► JSON Response
  ├─ POST /api/chat ────► check_auth() ──► chat_handler() ──► Event Bus ──► Plugin Processing
  ├─ GET /api/events/stream ───────────► sse_handler() ──► Broadcast Subscribe ──► SSE Stream
  └─ POST /api/plugins/:id/permissions ► check_auth() ──► grant_permission() ──► Event + Audit Log
```

### 0.3 Event Processing Pipeline

```
1. Event Injected (HTTP handler / plugin cascade / system)
       │
2. EnvelopedEvent wraps event with source_plugin_id + trace_id
       │
3. Event Channel (mpsc) buffers event
       │
4. Event Processor loop:
   ├── Depth check (configurable, default 10 levels, prevents infinite cascade)
   ├── Broadcast to SSE subscribers
   ├── Save to event history ring buffer
   └── Dispatch to Plugin Manager
           │
5. Plugin Manager:
   ├── Filter active plugins
   ├── Apply timeout guard (per-plugin)
   └── Call plugin.on_event() concurrently
           │
6. Plugin responses may generate cascade events → back to step 1
```

#### Consensus mode (multi-engine deliberation)

A message starting with the consensus prefix (default `consensus:`) takes a
dedicated in-kernel path instead of the single-engine loop
(`handlers/system.rs::run_consensus`): the kernel fans the task out
concurrently to the originating agent's granted engine servers (the global
`CONSENSUS_ENGINES` env only narrows this per-agent set), collects the
independent proposals, then runs a synthesizer engine over the combined views
and delivers exactly one response attributed to the originating agent.
Orchestration is synchronous inside one task — proposals are never carried by
events, so identity and ordering are structural (no aggregation races). Engine
failures degrade gracefully: working engines are re-sampled through varied
reasoning framings to reach quorum (`CONSENSUS_ENGINE_REUSE`), and every
terminal branch emits a definite response. Per-step `ConsensusProgress` events
feed the dashboard's Consensus tab. `consensus.rs` retains only the config and
prompt builders; see `docs/CONSENSUS_REVIVAL_DESIGN.md`.

### 0.4 Crate Structure

```
ClotoCore/
├── crates/
│   ├── core/          # Kernel: HTTP server, handlers, event loop, database
│   │   └── src/
│   │       ├── handlers.rs      # Handler routing module (re-exports sub-handlers)
│   │       ├── handlers/        # HTTP API sub-handlers
│   │       │   ├── system.rs    #   Agentic loop, chat pipeline, delegation
│   │       │   ├── mcp.rs       #   MCP server management endpoints
│   │       │   ├── agents.rs    #   Agent CRUD, avatar, power toggle
│   │       │   ├── chat.rs      #   Chat message persistence
│   │       │   ├── cron.rs      #   CRON job management
│   │       │   ├── commands.rs  #   Command approval (HITL)
│   │       │   ├── permissions.rs # Permission approval/denial
│   │       │   ├── events.rs    #   SSE event stream
│   │       │   ├── llm.rs       #   LLM proxy endpoints
│   │       │   ├── assets.rs    #   Static asset serving
│   │       │   ├── response.rs  #   ok_data() / json_data() helpers
│   │       │   └── utils.rs     #   Validation, password verification
│   │       ├── db/              # SQLite persistence layer
│   │       │   ├── mod.rs       #   Schema, migrations, core queries
│   │       │   ├── mcp.rs       #   MCP server state, access control
│   │       │   ├── chat.rs      #   Chat message storage
│   │       │   ├── permissions.rs # Permission requests
│   │       │   ├── cron.rs      #   CRON job persistence
│   │       │   ├── audit.rs     #   Audit log queries
│   │       │   ├── api_keys.rs  #   API key management
│   │       │   ├── llm.rs       #   LLM routing rules
│   │       │   └── trusted_commands.rs # Command trust store
│   │       ├── managers/        # Runtime managers
│   │       │   ├── mod.rs       #   Module declarations
│   │       │   ├── mcp.rs       #   McpClientManager (server orchestration)
│   │       │   ├── mcp_client.rs #  JSON-RPC client per MCP server
│   │       │   ├── mcp_types.rs #   Shared MCP types (McpServerHandle, etc.)
│   │       │   ├── mcp_protocol.rs # MCP JSON-RPC message types
│   │       │   ├── mcp_transport.rs # stdio transport layer
│   │       │   ├── mcp_mgp.rs  #   MGP capability negotiation & types
│   │       │   ├── mcp_kernel_tool.rs # MGP kernel tool registry (mgp.* namespace, 18 tools)
│   │       │   ├── mcp_tool_validator.rs # Code security validation
│   │       │   ├── mcp_lifecycle.rs # Server lifecycle state machine
│   │       │   ├── mcp_streaming.rs # Streaming & flow control
│   │       │   ├── mcp_events.rs #  Event subscriptions & callbacks
│   │       │   ├── mcp_discovery.rs # Runtime server discovery
│   │       │   ├── mcp_tool_discovery.rs # Dynamic tool discovery & session cache
│   │       │   ├── mcp_isolation.rs # OS isolation profile enforcement
│   │       │   ├── mcp_seal.rs  #   Magic Seal binary verification
│   │       │   ├── mcp_health.rs #  Server health monitor + auto-restart
│   │       │   ├── mcp_venv.rs  #   Python venv auto-setup
│   │       │   ├── capability_dispatcher.rs # Capability-based event dispatch
│   │       │   ├── registry.rs  #   PluginRegistry (event dispatch)
│   │       │   ├── plugin.rs    #   PluginManager (lifecycle)
│   │       │   ├── agents.rs    #   AgentManager
│   │       │   ├── llm_proxy.rs #   LLM API proxy
│   │       │   └── scheduler.rs #   CRON scheduler
│   │       ├── config.rs        # AppConfig from environment variables
│   │       ├── events.rs        # Event processor (cascade, broadcast, dispatch)
│   │       ├── middleware.rs    # Rate limiter, request tracking
│   │       ├── consensus.rs    # Consensus config + prompt builders (orchestration: handlers/system.rs)
│   │       ├── capabilities.rs # Capability types and permission model
│   │       │   │       └── lib.rs           # AppState, router setup, server bootstrap
│   └── shared/        # Shared trait definitions
│       └── src/lib.rs           # Plugin, ReasoningEngine, Tool traits
│
├── dashboard/          # React/TypeScript web UI (Tauri desktop app)
├── infra/              # Docker Compose, SearXNG config
├── scripts/            # Build tools, verification scripts
├── qa/                 # Issue registry (version-controlled bug tracking)
└── docs/               # Architecture, changelog, development guides, archive
```

### 0.5 API Endpoint Summary

**Admin Endpoints** (requires `X-API-Key` header, rate-limited):

| Method | Route | Description |
|--------|-------|-------------|
| POST | `/api/system/shutdown` | Graceful shutdown |
| POST | `/api/plugins/apply` | Bulk enable/disable plugins |
| POST | `/api/plugins/:id/config` | Update plugin config |
| POST | `/api/plugins/:id/permissions/grant` | Grant permission to plugin |
| POST | `/api/agents` | Create agent |
| POST | `/api/agents/:id` | Update agent |
| POST | `/api/agents/:id/power` | Toggle agent power state |
| POST | `/api/events/publish` | Publish event to bus |
| POST | `/api/permissions/:id/approve` | Approve permission request |
| POST | `/api/permissions/:id/deny` | Deny permission request |
| POST | `/api/chat` | Send message to agent |
| GET/POST/DELETE | `/api/chat/:agent_id/messages` | Chat message persistence |
| GET | `/api/chat/attachments/:attachment_id` | Retrieve chat attachment |
| POST/GET | `/api/mcp/servers` | MCP server management |
| DELETE | `/api/mcp/servers/:name` | Delete MCP server |
| GET/PUT | `/api/mcp/servers/:name/settings` | Server settings |
| GET/PUT | `/api/mcp/servers/:name/access` | Access control |
| POST | `/api/mcp/servers/:name/start\|stop\|restart` | Lifecycle |

**Public Endpoints** (no authentication required):

| Method | Route | Description |
|--------|-------|-------------|
| GET | `/api/system/version` | Current version info |
| GET | `/api/events` | SSE event stream |
| GET | `/api/history` | Recent event history |
| GET | `/api/metrics` | System metrics |
| GET | `/api/memories` | Memory entries |
| GET | `/api/plugins` | Plugin list with manifests |
| GET | `/api/plugins/:id/config` | Plugin configuration |
| GET | `/api/agents` | Agent configurations |
| GET | `/api/permissions/pending` | Pending permission requests |
| GET | `/api/mcp/access/by-agent/:agent_id` | Agent MCP access |
| ANY | `/api/plugin/*path` | Dynamic plugin route proxy |

### 0.6 API Response Format Convention

All REST API endpoints MUST follow this response envelope convention.
This provides a consistent, extensible contract between backend and frontend.

#### Success Response

All successful responses use the `data` key as the top-level envelope:

```jsonc
// Mutation with no return data (e.g., DELETE, toggle)
{ "data": {} }

// Mutation with return data (e.g., POST creating a resource)
{ "data": { "id": "agent.xxx" } }

// Query returning a single resource
{ "data": { "id": "agent.xxx", "name": "Assistant", ... } }

// Query returning a collection
{ "data": { "messages": [...], "has_more": true } }
```

The `data` key is ALWAYS present on success. Additional top-level keys may be
added in the future without breaking changes:

```jsonc
{
  "data": { ... },
  "request_id": "req-abc123",           // Future: request tracing (M-16)
  "meta": { "page": 1, "total": 42 }   // Future: pagination (L-09)
}
```

#### Error Response

All error responses use the `error` key as the top-level envelope:

```jsonc
{
  "error": {
    "type": "ValidationError",
    "message": "Agent name must be between 1 and 100 characters"
  }
}
```

HTTP status codes carry the primary success/failure signal.
The envelope key (`data` vs `error`) provides a secondary, parseable indicator.

#### Implementation

Handlers MUST use the response helper functions defined in `handlers.rs`:

```rust
// No return data
ok_data(())

// With return data
ok_data(json!({ "id": agent_id }))

// The helper guarantees: { "data": { "id": "..." } }
```

Direct construction of `serde_json::json!({ "status": "..." })` is prohibited.
All response formatting is centralized in the helper to prevent format drift.

#### SSE Events

SSE event payloads (`/api/events`) are NOT wrapped in this envelope.
SSE events use their own format defined by the `ClotoEvent` type system.

---

## 1. Design Principles (Manifesto)

All development within this system must adhere to the following nine design principles.

### 1.1 Core Minimalism
**"The Kernel is the stage, not the actor."**
- **Kernel responsibilities**: Plugin lifecycle management, event mediation, data persistence interface, and web server foundation.
- **Prohibited**: Hard-coding functionality into the Kernel, such as logic dependent on specific AI models (LLMs) or processing logic for specific memory formats.
- **Goal**: Enable all functionality through plugin addition and replacement alone, without modifying the Kernel.

### 1.2 Capability over Concrete Type
**"Not who it is, but what it can do."**
- **Design guideline**: Do not branch logic by directly referencing plugin IDs or names. Instead, invoke plugins through `CapabilityType` (Reasoning, Memory, HAL, etc.).
- **Benefit**: Enables deep-level plugin swapping and improves overall system portability.

### 1.3 Event-First Communication
**"Don't talk directly -- announce it in the plaza."**
- **Design guideline**: Communication between plugins, and between the Kernel and plugins, should be asynchronous and loosely coupled via the event bus (`ClotoEvent`) whenever possible.
- **Goal**: Achieve a state where features integrate by simply reacting to specific events, even if Plugin A is unaware of Plugin B's existence.

### 1.4 Data Sovereignty
**"The Kernel holds the data, but does not interpret its contents."**
- **Design guideline**: Plugin-specific settings and agent attributes are treated as opaque `metadata` (JSON) rather than extending Kernel tables.
- **Goal**: Allow plugins to freely define and persist their own internal data structures without modifying the database schema.

### 1.5 Strict Permission Isolation
**"Capability comes with responsibility and authorization."**
- **Design guideline**: Separate capability provision from resource access (permissions).
- **Goal**: Achieve secure execution environments through metadata definitions alone -- such as "an agent that can write to files but cannot access the network."

### 1.6 Seamless Integration & DevEx
**"Principles are guardrails, not walls."**
- **Design guideline**: Use the SDK (macros, utilities) to abstract away architectural complexity so that strict principles do not hinder development.
- **Approach**:
    - **Macro-driven Compliance**: Minimize human error and boilerplate by converting common trait implementations and manifest definitions into macros.
    - **Encapsulated Complexity**: Abstract common patterns such as event filtering and storage initialization at the SDK level.

### 1.7 Polyglot Extension
**"Guard the core with Rust, spread the wings with Python."**
- **Design guideline**: When advanced numerical computation or access to extensive library ecosystems is needed, integrate other language environments (primarily Python) through an "AI Container (Bridge Plugin)" rather than adding functionality to the Core.

### 1.8 Dynamic Intelligence Orchestration
**"Capabilities are not given -- they are earned."**
- **Design guideline**: AI agent permissions should not be fixed at startup, but rather dynamically requested and granted at runtime based on "intent" (Human-in-the-loop).
- **Goal**: Realize a dynamic ecosystem where humans can instantly unlock AI capabilities as needed while maintaining the principle of least privilege.

### 1.9 Self-Healing AI Containerization
**"Even if it dies, it resurrects and keeps moving forward."**
- **Design guideline**: External runtimes (AI Containers) must not only be physically isolated from the Kernel but also possess the ability to autonomously reset and recover upon anomalies.

### Operating Core Minimalism: When Kernel Extension Is Justified

§1.1 is a guardrail, not a wall (§1.6). It biases toward a minimal kernel and pushing functionality into plugins, but it does not forbid kernel extension outright — periodically a concern genuinely belongs in the kernel rather than a plugin. A change earns a place in the kernel when it satisfies these five tests; the more it satisfies, the stronger the case:

1. **Cross-cutting reach** — the concern applies uniformly across multiple plugins, all agents, or multiple backends, so the kernel is the only natural convergence point. Distributing it across plugins would duplicate it or let independent copies drift out of sync. (§1.3)
2. **Binding to kernel-owned machinery** — it is inseparable from data or a lifecycle the kernel already owns (e.g. `SessionManager` / the short-term transcript, the event bus, persistence). Deriving that data is the owner's responsibility. (§1.1 kernel responsibilities)
3. **Format-agnostic abstraction** — *this is the boundary line.* §1.1 forbids hard-coding *a specific plugin's internal format*. A capability-level, generic intent may live in the kernel; a plugin's implementation detail may not. The kernel may hold the abstraction; the plugin keeps the interpretation.
4. **Universality and stability** — a stable, universal abstraction is worth lifting into the kernel; a fluid experiment stays in a plugin until it settles. (§1.6)
5. **Replacement cost** — if distributing the concern across plugins produces duplication, wiring complexity, or incoherence, kernel consolidation wins on simplicity. (§1.1 goal)

**The boundary line (test 3) in practice.** The kernel may resolve an opaque `metadata` setting and derive a generic, capability-level value from it; it must not encode how a specific plugin interprets that value. The canonical pattern is `engine_routing`: the routing plugin owns the schema, and the kernel only forwards the opaque metadata and consumes the result (`EngineSelection`). Equivalently, the kernel may resolve an agent's *session scope* into a generic `(session_key, source-identity)` pair and hand it to the Memory capability — while *how* a memory plugin interprets that identity (for example, a prefix match over stored sources) stays inside the plugin.

When a change fails these tests — it is one plugin's concern, it depends on a plugin's internal format, or it is still experimental — keep it in the plugin.

---

## 2. Security and Governance Framework

### 2.1 Three-Layer Security Structure

1. **Physical Isolation (Containerization)**: Each agent (especially those with external connectivity) runs as an independent OS process, separate from the Kernel.
2. **Authorization Gate**: All action intents (`ActionRequested`, `PermissionRequested`) pass through the Kernel's event processor and are filtered based on a whitelist.
3. **Capability Injection**: Plugins cannot instantiate communication libraries on their own. They interact with the outside world only through "authorized tools" provided by the Kernel.
4. **Event Enveloping**: All plugin events are wrapped in an `EnvelopedEvent`, physically preventing source ID spoofing.

### 2.2 Dynamic Permission Escalation (Human-in-the-loop)

1. The AI Container detects an operation that cannot be performed with current permissions
2. Issues a `ClotoEvent::PermissionRequested`
3. The Dashboard's Security Guard UI prompts the user for approval
4. The user approves
5. The Kernel updates the DB and live-injects the capability into the running container

### 2.3 SafeHttpClient

- **Host restriction**: Blocks access to `localhost` and private IPs by default
- **Domain control**: Domain restriction via whitelist (O(1) lookup using HashSet)
- **DNS rebinding protection**: Restriction checks are also applied to IP addresses after name resolution

### 2.4 Operational Guidelines

- **Least privilege**: Only list the truly necessary minimum items in the manifest's `required_permissions`
- **Explicit reasoning**: Include a human-readable explanation in the `reason` field when requesting permissions

---

## 3. Plugin Interaction Mechanisms

### 3.1 MCP Server Architecture

All plugin functionality is delivered via **Model Context Protocol (MCP)** servers.
MCP servers are independent processes that communicate with the Kernel via JSON-RPC over stdio.

For full architecture details, see [MCP Plugin Architecture](https://github.com/Cloto-dev/ClotoCore/blob/master/docs/MCP_PLUGIN_ARCHITECTURE.md).

**Key characteristics:**
- Servers maintained in the private clotohub-servers repository (formerly cloto-mcp-servers)
- Configured via `mcp.toml` with `[paths]` section for external server resolution
- Language-agnostic: any language implementing MCP protocol
- Process isolation: each server runs as a separate OS process
- Dispatch: PluginRegistry routes events to MCP servers via MCP client manager
- Access control: 3-level RBAC (capability → server_grant → tool_grant)

### 3.1.1 Server Path Resolution

`mcp.toml` supports a `[paths]` section for named path variables, expanded as
`${var}` in `command` and `args` fields before relative path resolution:

```toml
[paths]
servers = "C:/path/to/clotohub-servers/servers"

[[servers]]
command = "python"
args = ["${servers}/terminal/server.py"]
```

Path variable values may themselves reference environment variables (`${ENV_VAR}`).
If `[paths]` is absent, relative paths are resolved against the project root (backward compatible).

### 3.1.2 Server Restart Policy

When an MCP server exits or errors, the lifecycle manager (`mcp_lifecycle.rs`)
restarts it according to a `RestartPolicy`. Unless overridden per-server in
`mcp.toml`, the following defaults apply. These values are defined once as
constants in `crates/core/src/managers/mcp_protocol.rs` (`DEFAULT_MAX_RESTARTS`
et al.) and are the single source of truth for both the runtime and tests:

| Field | Default | Meaning |
|---|---|---|
| `strategy` | `OnFailure` | Restart only on error/unexpected exit (not on clean stop) |
| `max_restarts` | `5` | Maximum restarts allowed within the window before giving up |
| `restart_window_secs` | `300` | Rolling window (seconds) over which `max_restarts` is counted |
| `backoff_base_ms` | `1000` | Base delay for exponential backoff (`base * 2^(n-1)`) |
| `backoff_max_ms` | `30000` | Upper cap on the backoff delay |

**Migration plan (D → C):** The current approach (D) uses file-path-based server
resolution. The future approach (C) will use Python package-based invocation
(`python -m cloto_mcp_servers.terminal`), eliminating path configuration entirely.
The root `pyproject.toml` in clotohub-servers is prepared for this migration.

### 3.1.3 Protocol Era Negotiation

MCP 2026-07-28 ("stateless core") removed the `initialize` / `notifications/initialized`
handshake and protocol-level sessions: a request now carries its own context in
`params._meta`. The kernel's MCP client is therefore **dual-era** — it decides at
connect time which era a server speaks, and the two are never mixed on one connection.

| | Legacy (≤ 2025-11-25) | Modern (2026-07-28+) |
|---|---|---|
| Connect exchange | `initialize` + `notifications/initialized` | `server/discover` probe only |
| Per-request context | none (session-scoped) | `_meta` `protocolVersion` / `clientInfo` / `clientCapabilities` |
| Log severity | one `logging/setLevel` after initialize | `_meta` `logLevel` per request (method removed from the protocol) |
| HTTP session | `Mcp-Session-Id` sent and tracked | not sent; `mcp-protocol-version` / `mcp-method` routing headers instead |
| MGP activation | `initialize.capabilities.mgp` piggyback | `capabilities.extensions["dev.cloto/mgp"]` |
| MGP grants | `mgp/permission/grant` RPC | same RPC, plus per-request `_meta` `dev.cloto/mgp/grants` |

**Era decision** (`McpClient::negotiate`, mirroring the reference SDK's denylist probe):
anything that is not positive evidence of a modern server falls back to the handshake.
A `server/discover` probe is sent at the newest modern version; `UNSUPPORTED_PROTOCOL_VERSION`
(`-32022`) naming a mutual modern version triggers one downgrade re-probe, while the same
code naming neither a known modern nor any handshake version fails the connect as a genuine
incompatibility. Every other JSON-RPC error — including `METHOD_NOT_FOUND`, an unparseable
result, a reply advertising no modern version, and probe silence — falls back to `initialize`.
Transport and process failures propagate instead: an outage must never be read as an era
verdict. The era is settled once per connection (a stdio server's process lifetime).

Consequences for the legacy path are deliberately nil — its wire traffic, ordering and
session handling are unchanged, which the pre-existing connection tests pin (their
assertions were not touched). In the modern era, a `resultType` of `input_required` (MRTR, a multi-round
continuation the kernel host has no flow for) surfaces as an explicit error rather than
parsing as a final result, and `DiscoverResult.instructions` is captured on the server
handle for the agent-prompt consumer.

**Escape hatch:** `protocol_era = "legacy"` on a server entry in `mcp.toml` skips the probe
entirely, for a server that mishandles unknown methods; an unrecognized value warns and
behaves as `auto`. The probe costs one bounded window (≤ 10 s) against a server that
answers unknown methods with silence — a conformant server replies immediately.

Protocol versions, `_meta` keys, routing header names and the error code are defined once in
`crates/core/src/managers/mcp_protocol.rs` and are the single source of truth for the client
(`mcp_client.rs`), the HTTP transport (`mcp_transport.rs`) and the tests.

### 3.2 Architecture History

> **Note:** The original Three-Tier Plugin Model (Rust compiled, Python Bridge, WASM)
> has been fully superseded by the MCP-only architecture as of v0.4.x.
> Archived design documents are in `docs/archive/`.
> See [MCP Plugin Architecture](https://github.com/Cloto-dev/ClotoCore/blob/master/docs/MCP_PLUGIN_ARCHITECTURE.md) for current details.

---

## 4. Project Oculi: Eye-Tracking & Visual Symbiosis (Paused)

> **Status:** Paused. Sensor layer archived. Future implementation would use MCP server-based approach.

A sub-project that shares human gaze data with the AI in real time, enabling low-latency, low-cost inference through Foveated Vision.

**Original Roadmap:**
- [x] Phase 1: Communication Infrastructure (GazeUpdated event, streaming support)
- [x] Phase 2: Neural Synchronization (MediaPipe gaze estimation, dashboard synchronization)
- [ ] Phase 3: Foveated Vision -- paused
- [ ] Phase 4: Intent Discovery -- paused

---

## 5. Self-Evolution Engine (Archived)

> **Status:** Archived for Phase A. Code has been removed.
>
> The evolution engine implements 5-axis fitness scoring (cognitive, behavioral,
> safety, autonomy, meta_learning), generation management with debounce,
> rollback with grace periods, and safety breach detection.
>
> See the archived source code for full design specifications.

---

## Operations

These principles define the "correct approach" within ClotoCore. The following criteria are used for code audits:
1. **Consistency**: Whether each component adheres to the principles described above
2. **Sustainability**: Whether compliance with the principles is kept "easy" through the SDK and macros

### Related Design Documents

- [mgp-spec](https://github.com/Cloto-dev/mgp-spec) — Multi-Agent Gateway Protocol specification (its own repository)
- [MGP Isolation Design](https://github.com/Cloto-dev/mgp-spec/blob/main/docs/MGP_ISOLATION_DESIGN.md) — OS-Level isolation design (in the mgp-spec repo)
- [MGP Modernization Design](https://github.com/Cloto-dev/mgp-spec/blob/main/docs/MGP_MODERNIZATION_DESIGN.md) — MGP's adaptation to MCP 2026-07-28; the specification counterpart to §3.1.3 (in the mgp-spec repo)
