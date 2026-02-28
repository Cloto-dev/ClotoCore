# MGP — Model General Protocol

**Version:** 0.5.2-draft
**Status:** Draft
**Authors:** ClotoCore Project
**Date:** 2026-02-28

---

## 1. Overview

### 1.1 What is MGP?

MGP (Model General Protocol) is a **strict superset of MCP** (Model Context Protocol) that adds
protocol-level security, access control, and observability while maintaining full backward
compatibility.

Any valid MCP message is a valid MGP message. Any MGP server can operate as a standard MCP
server when connected to a client that does not support MGP extensions.

### 1.2 Design Principles

1. **Backward Compatible** — MGP extends MCP; it never modifies or removes MCP behavior
2. **Graceful Degradation** — MGP features activate only when both sides negotiate support
3. **Security by Default** — Dangerous operations require explicit permission grants
4. **Defense in Depth** — Multiple independent validation layers (server, kernel, protocol)
5. **Auditable** — All security-relevant actions produce structured audit events

### 1.3 Compatibility Matrix

| Client | Server | Behavior |
|--------|--------|----------|
| Standard MCP | Standard MCP | Standard MCP operation |
| Standard MCP | MGP Server | MCP operation (MGP extensions silent) |
| MGP Client | Standard MCP | MCP operation (client uses fallback behavior) |
| MGP Client | MGP Server | Full MGP operation |

All four patterns are functional. No configuration changes are required.

### 1.4 Transport

MGP inherits MCP's transport layer. All transports supported by MCP (stdio, HTTP+SSE) are
supported by MGP. MGP does not define new transports.

### 1.5 Message Format

MGP uses JSON-RPC 2.0, identical to MCP:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "method/name",
  "params": {}
}
```

MGP-specific methods use the `mgp/` prefix. MGP-specific notifications use the
`notifications/mgp.` prefix. Standard MCP methods remain unchanged.

### 1.6 Protocol Architecture — Selective Minimalism

MGP extends MCP with only **10 protocol primitives** (4 methods + 6 notifications) organized in three layers. All
other functionality is provided as standard MCP tools exposed by the kernel.

```
┌──────────────────────────────────────────────────────────┐
│  Layer 1: Metadata Extensions (0 new methods)            │
│  └─ _mgp fields on initialize, tools/list, tools/call   │
├──────────────────────────────────────────────────────────┤
│  Layer 2: Protocol Notifications (6)                     │
│  ├─ notifications/mgp.audit                              │
│  ├─ notifications/mgp.stream.chunk                       │
│  ├─ notifications/mgp.stream.progress                    │
│  ├─ notifications/mgp.lifecycle                          │
│  ├─ notifications/mgp.callback.request                   │
│  └─ notifications/mgp.event                              │
├──────────────────────────────────────────────────────────┤
│  Layer 3: Protocol Methods (4 — irreducible)             │
│  ├─ mgp/permission/await                                 │
│  ├─ mgp/permission/grant                                 │
│  ├─ mgp/callback/respond                                 │
│  └─ mgp/stream/cancel                                    │
├──────────────────────────────────────────────────────────┤
│  Layer 4: Kernel Tools (16 — standard tools/call)        │
│  ├─ mgp.access.*    (query, grant, revoke)       — §5   │
│  ├─ mgp.health.*    (ping, status)               — §11  │
│  ├─ mgp.lifecycle.* (shutdown)                   — §11  │
│  ├─ mgp.events.*    (subscribe, unsubscribe)     — §13  │
│  ├─ mgp.discovery.* (list, register, deregister) — §15  │
│  └─ mgp.tools.*     (discover, request, session) — §16  │
└──────────────────────────────────────────────────────────┘
```

#### Design Rationale

**Layers 1-3** are protocol-level primitives. All MGP implementations MUST support
these. They are irreducible: each requires bidirectional agreement or fire-and-forget
notification that cannot be expressed as a tool call.

**Layer 4** tools are exposed by the kernel as standard MCP tools via `tools/call`.
They do NOT require new protocol methods because:

1. **The kernel is the enforcement point.** Access control, health checks, and tool
   discovery are kernel-side operations. Whether invoked via a protocol method or a
   tool call, the kernel enforces the same rules.
2. **Servers cannot bypass kernel tools.** In the MGP architecture, the kernel is the
   MCP client. Servers cannot call kernel tools — only agents and operators can.
3. **No interoperability loss.** Compliant kernels SHOULD expose the standard kernel
   tools defined in §5, §11, §13, §15, and §16. The tool schemas are standardized
   even though the invocation mechanism is `tools/call` rather than a dedicated method.

This architecture reduces MGP's protocol surface area by 60% (25 → 10 primitives)
while maintaining full security guarantees and MCP structural limitation breakthroughs.

### 1.7 Relationship to MCP & Migration Policy

MGP is a strict superset of MCP. As MCP evolves, some features currently unique to
MGP may be adopted into MCP itself. MGP's migration policy ensures continuity for
implementors.

#### Migration Commitment

When MCP officially adopts functionality equivalent to an MGP extension, MGP will:

1. **Provide a compatibility layer** that maps between MGP method names/formats and
   the MCP equivalents, allowing existing MGP implementations to work with both
   protocols during a transition period
2. **Deprecate the MGP-specific extension** with at least one minor version of overlap
   (e.g., if MCP adds security in MGP 0.6, the MGP `security` extension remains
   supported through 0.7 and is removed in 0.8)
3. **Document the migration path** in the Version History (§18) with concrete
   before/after examples

#### Extension Migration Categories

| Category | MGP Extensions | Migration Likelihood | Notes |
|----------|---------------|---------------------|-------|
| Security | §3-5, §7 | Medium | MCP has discussed auth; MGP will adapt |
| Observability | §6 | Medium | OpenTelemetry integration is common |
| Lifecycle | §11 | Low | MCP has no lifecycle primitives planned |
| Communication | §12, §13 | Low-Medium | MCP Streamable HTTP addresses some |
| Discovery | §15 | Low | Static config is MCP's current approach |
| **Intelligence** | **§16** | **Very Low** | **No MCP equivalent planned or proposed** |

#### Strategic Differentiation

MGP's unique value lies in the **Intelligence Layer** (§16). While security and
lifecycle features are natural candidates for eventual MCP adoption, Dynamic Tool
Discovery (§16) addresses a structural limitation of the MCP architecture:

- MCP requires all tool definitions in the LLM context before use
- This fundamentally limits scalability and prevents autonomous tool acquisition
- §16 solves this at the protocol level with discovery, active request, and session
  management — capabilities that require a kernel/orchestrator role not present in
  MCP's direct client-server model

Even if MCP adds all security features, MGP §16 alone justifies the protocol's
existence for any system managing more than ~20 tools.

---

## 2. Capability Negotiation

### 2.1 Overview

MGP capability negotiation piggybacks on the standard MCP `initialize` handshake. The client
includes an `mgp` field in its `capabilities` object. The server responds with its supported
MGP capabilities. If either side omits the `mgp` field, the connection operates in standard
MCP mode.

### 2.2 Client → Server (initialize request)

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "protocolVersion": "2024-11-05",
    "capabilities": {
      "mgp": {
        "version": "0.1.0",
        "extensions": ["security", "access_control", "audit"]
      }
    },
    "clientInfo": {
      "name": "ClotoCore",
      "version": "0.2.8"
    }
  }
}
```

The `mgp` object is OPTIONAL. Standard MCP clients will not include it, and standard MCP
servers will ignore it.

**Fields:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `version` | string | Yes | MGP protocol version (semver) |
| `extensions` | string[] | Yes | List of MGP extensions the client supports |

**Standard Extensions:**

| Extension | Layer | Description | Spec |
|-----------|-------|-------------|------|
| `security` | 1+3 (Metadata + Method) | Permission declarations and tool security metadata | §3, §4 |
| `access_control` | 4 (Kernel Tool) | Agent-scoped tool access control | §5 |
| `audit` | 2 (Notification) | Structured audit trail notifications | §6 |
| `code_safety` | 1 (Metadata) | Code execution safety framework | §7 |
| `lifecycle` | 2+4 (Notification + Kernel Tool) | State transitions, health checks, shutdown | §11 |
| `streaming` | 2+3 (Notification + Method) | Stream chunks, progress, cancellation | §12 |
| `bidirectional` | 2+3+4 (All) | Callbacks, events, subscriptions | §13 |
| `discovery` | 4 (Kernel Tool) | Server registration, deregistration | §15 |
| `tool_discovery` | 4 (Kernel Tool) | Dynamic tool search, active tool request | §16 |
| `error_handling` | 1 (Metadata) | Structured error categories and recovery hints | §14 |

Negotiating a Layer 4 extension means the kernel exposes the corresponding standard
MCP tools (see §1.6). Layer 1-3 extensions activate protocol-level behavior.

### 2.3 Server → Client (initialize response)

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2024-11-05",
    "capabilities": {
      "tools": {},
      "mgp": {
        "version": "0.1.0",
        "extensions": ["security", "audit"],
        "permissions_required": ["shell", "network"],
        "server_id": "mind.cerebras",
        "trust_level": "standard"
      }
    },
    "serverInfo": {
      "name": "cerebras-engine",
      "version": "1.0.0"
    }
  }
}
```

**Fields:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `version` | string | Yes | MGP version the server supports |
| `extensions` | string[] | Yes | Extensions the server supports (intersection with client) |
| `permissions_required` | string[] | No | Permissions this server needs to operate |
| `server_id` | string | No | Unique server identifier |
| `trust_level` | string | No | `trusted`, `standard`, or `sandboxed` |

### 2.4 Negotiation Rules

1. The active extension set is the **intersection** of client and server extensions
2. If the server does not include `mgp` in its response, the connection is standard MCP
3. If the client did not include `mgp` in its request, the server MUST NOT include `mgp`
   in its response
4. Version compatibility uses semver: major version must match, minor is backward compatible

### 2.5 Versioning Policy

#### Stable Releases (1.0.0+)

Standard semantic versioning:
- **Major** version changes indicate breaking protocol changes
- **Minor** version changes add new extensions or features; backward compatible
- **Patch** version changes fix errata or clarify wording; no behavioral changes

#### Pre-1.0 Period (0.x.y)

During the pre-1.0 development period:
- **Minor** version changes (e.g., 0.3 → 0.4) **MAY** contain breaking changes
- **Patch** version changes (e.g., 0.4.0 → 0.4.1) **MUST NOT** contain breaking changes
- Implementations SHOULD log a warning when connecting to a peer with a different
  minor version (e.g., client 0.3 ↔ server 0.4) but SHOULD still attempt connection
- Breaking changes in minor versions MUST be documented in the Version History (§18)
  with migration guidance

#### 1.0.0 Stability Milestone

MGP will be declared 1.0.0 (stable) when all of the following criteria are met:

1. At least **two independent implementations** (client and/or server) exist
2. A **conformance test suite** covers all Tiers (1-4) as defined in §17.5
3. The specification has been in draft status for at least **6 months** without
   breaking changes to the core protocol (§2-7)
4. The `mgp-validate` tool can verify compliance at all Tiers

---

## 3. Permission Declarations

### 3.1 Overview

MGP servers declare what permissions they need to function. The client decides whether to
grant, deny, or defer to a human operator. This formalizes the "Permission Gate" pattern.

### 3.2 Standard Permission Types

| Permission | Description | Risk Level |
|------------|-------------|------------|
| `filesystem.read` | Read files from the host filesystem | moderate |
| `filesystem.write` | Write/create/delete files | dangerous |
| `network.outbound` | Make outbound network requests | moderate |
| `network.listen` | Bind to a network port | dangerous |
| `shell.execute` | Execute shell commands | dangerous |
| `code_execution` | Execute arbitrary code | dangerous |
| `memory.read` | Read from memory/knowledge stores | safe |
| `memory.write` | Write to memory/knowledge stores | moderate |
| `system.info` | Read system information (OS, CPU, etc.) | safe |
| `camera` | Access camera/vision devices | dangerous |
| `notification` | Send notifications to the user | safe |

Implementations MAY define custom permission types using reverse-domain notation
(e.g., `com.example.custom_permission`).

### 3.3 Client Approval Policies

The client applies one of these policies to permission requests:

| Policy | Behavior |
|--------|----------|
| `interactive` | Present each permission to the human operator for approval |
| `auto_approve` | Automatically approve all permissions (YOLO mode) |
| `deny_all` | Deny all permissions not pre-configured |
| `config_only` | Only approve permissions listed in configuration |

### 3.4 Permission Request Flow

```
Server                          Client
  │                               │
  │  initialize (permissions_required: ["shell"])
  │──────────────────────────────>│
  │                               │
  │                               │ (client checks policy)
  │                               │
  │  mgp/permission/await         │
  │<──────────────────────────────│  (if interactive: "await my decision")
  │                               │
  │                               │ (operator approves/denies)
  │                               │
  │  mgp/permission/grant         │
  │<──────────────────────────────│  (delivers decision to server)
  │                               │
  │  initialize result            │
  │<──────────────────────────────│  (connection proceeds or is rejected)
```

Both methods flow Client → Server, consistent with MCP's transport model where
the client (kernel) is always the initiator.

### 3.5 Permission Await Method

**Method:** `mgp/permission/await`

Direction: Client → Server

The client instructs the server to wait while the operator reviews the requested
permissions. The server MUST NOT proceed with restricted operations until a
corresponding `mgp/permission/grant` is received.

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "mgp/permission/await",
  "params": {
    "request_id": "perm-001",
    "permissions": ["shell.execute", "filesystem.read"],
    "policy": "interactive",
    "message": "Waiting for operator approval"
  }
}
```

### 3.6 Permission Grant Method

**Method:** `mgp/permission/grant`

Direction: Client → Server

The client delivers the operator's decision to the server. This completes the
permission flow initiated by `mgp/permission/await`.

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "mgp/permission/grant",
  "params": {
    "request_id": "perm-001",
    "grants": {
      "shell.execute": "approved",
      "filesystem.read": "approved"
    },
    "approved_by": "admin",
    "expires_at": "2026-03-01T00:00:00Z"
  }
}
```

**Grant Values:**

| Value | Meaning |
|-------|---------|
| `approved` | Permission granted |
| `denied` | Permission denied (server should degrade gracefully) |
| `deferred` | Decision deferred (server should wait or retry) |

---

## 4. Tool Security Metadata

### 4.1 Overview

MGP extends the standard MCP `tools/list` response with a `security` object on each tool
definition. This allows clients to make informed decisions about tool execution without
inspecting tool internals.

### 4.2 Extended Tool Definition

```json
{
  "name": "execute_command",
  "description": "Execute a shell command",
  "inputSchema": {
    "type": "object",
    "properties": {
      "command": { "type": "string" }
    },
    "required": ["command"]
  },
  "security": {
    "risk_level": "dangerous",
    "permissions_required": ["shell.execute"],
    "side_effects": ["filesystem", "process"],
    "validator": "sandbox",
    "reversible": false,
    "confirmation_required": true
  }
}
```

Standard MCP clients will ignore the `security` field (it is not part of MCP's tool schema).

### 4.3 Security Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `risk_level` | string | Yes | `safe`, `moderate`, or `dangerous` |
| `permissions_required` | string[] | No | Permissions needed to call this tool |
| `side_effects` | string[] | No | Categories of side effects: `filesystem`, `network`, `process`, `database`, `notification` |
| `validator` | string | No | Kernel-side validator to apply: `sandbox`, `readonly`, `none` |
| `reversible` | boolean | No | Whether the tool's effects can be undone |
| `confirmation_required` | boolean | No | Whether the client should prompt the user before execution |

### 4.4 Risk Levels

| Level | Definition | Client Behavior |
|-------|-----------|-----------------|
| `safe` | No side effects, read-only operations | Execute without confirmation |
| `moderate` | Limited side effects, data writes | Execute with optional confirmation |
| `dangerous` | System-level side effects, irreversible | Require explicit confirmation or permission |

### 4.5 Standard Validators

| Validator | Description |
|-----------|-------------|
| `sandbox` | Block dangerous shell patterns, metacharacters, recursive delete |
| `readonly` | Block any write operations (enforce read-only tool usage) |
| `network_restricted` | Block requests to localhost, private IPs, metadata endpoints |
| `code_safety` | Apply code safety framework (see §7) to code arguments |
| `none` | No kernel-side validation (server handles its own safety) |

Validators are applied by the **client/kernel** before forwarding the tool call to the server.
This provides defense-in-depth: even a compromised server cannot bypass kernel validation.

---

## 5. Access Control — Kernel Tool Layer

### 5.1 Overview

The kernel exposes standard MCP tools for managing agent-to-tool access control.
These are **Layer 4 kernel tools** (see §1.6) — invoked via standard `tools/call`,
not dedicated protocol methods.

The enforcement point is always the kernel. Servers cannot bypass access control
regardless of how the tools are invoked.

### 5.2 Access Control Hierarchy

```
Priority (highest to lowest):
  1. tool_grant   — Explicit per-tool permission for an agent
  2. server_grant — Server-wide permission for an agent
  3. default_policy — Server's default (opt-in or opt-out)
```

### 5.3 Entry Types

| Type | Scope | Description |
|------|-------|-------------|
| `server_grant` | All tools on a server | Agent has access to entire server |
| `tool_grant` | Single tool | Agent has access to specific tool |

### 5.4 Kernel Tools

#### mgp.access.query

**Tool Name:** `mgp.access.query`
**Category:** Kernel Tool (Layer 4)

Query the current access state for an agent-tool combination.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "agent_id": { "type": "string", "description": "Agent identifier" },
    "server_id": { "type": "string", "description": "Target server" },
    "tool_name": { "type": "string", "description": "Target tool (optional)" }
  },
  "required": ["agent_id", "server_id"]
}
```

**Output:**
```json
{
  "permission": "allow",
  "source": "server_grant",
  "granted_by": "admin",
  "granted_at": "2026-02-27T12:00:00Z",
  "expires_at": null
}
```

#### mgp.access.grant

**Tool Name:** `mgp.access.grant`
**Category:** Kernel Tool (Layer 4)

Grant access to an agent. Requires operator-level permissions.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "entry_type": { "type": "string", "enum": ["server_grant", "tool_grant"] },
    "agent_id": { "type": "string" },
    "server_id": { "type": "string" },
    "tool_name": { "type": "string", "description": "Required for tool_grant" },
    "permission": { "type": "string", "enum": ["allow", "deny"] },
    "justification": { "type": "string" },
    "expires_at": { "type": "string", "format": "date-time" }
  },
  "required": ["entry_type", "agent_id", "server_id", "permission"]
}
```

#### mgp.access.revoke

**Tool Name:** `mgp.access.revoke`
**Category:** Kernel Tool (Layer 4)

Revoke an existing access grant.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "agent_id": { "type": "string" },
    "server_id": { "type": "string" },
    "entry_type": { "type": "string", "enum": ["server_grant", "tool_grant"] },
    "tool_name": { "type": "string" }
  },
  "required": ["agent_id", "server_id", "entry_type"]
}
```

### 5.5 Default Policies

| Policy | Behavior |
|--------|----------|
| `opt-in` | Deny by default. Agents must be explicitly granted access. |
| `opt-out` | Allow by default. Agents have access unless explicitly denied. |

---

## 6. Audit Trail

### 6.1 Overview

MGP defines a standard **audit event format** and **trace ID propagation** at the protocol
level. The storage, querying, and analysis of audit events is delegated to an Audit MGP
server (see §19.4).

This separation ensures that the protocol defines **what** audit events look like, while
**how** they are stored and processed remains an implementation concern.

### 6.2 Protocol Scope vs Server Scope

| Concern | Scope | Defined In |
|---------|-------|------------|
| Audit event format (structure, fields) | **Protocol** | This section |
| Standard event types | **Protocol** | This section |
| Trace ID propagation | **Protocol** | This section |
| Audit event storage and persistence | **Server** | §19.4 |
| Audit event querying and search | **Server** | §19.4 |
| Audit analytics and alerting | **Server** | §19.4 |

### 6.3 Audit Event Format

The kernel emits audit events as JSON-RPC notifications. All MGP implementations MUST
use this format for interoperability.

**Method:** `notifications/mgp.audit`

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/mgp.audit",
  "params": {
    "timestamp": "2026-02-27T12:00:00.000Z",
    "trace_id": "550e8400-e29b-41d4-a716-446655440000",
    "event_type": "TOOL_EXECUTED",
    "actor": {
      "type": "agent",
      "id": "agent.cloto_default"
    },
    "target": {
      "server_id": "tool.terminal",
      "tool_name": "execute_command"
    },
    "result": "SUCCESS",
    "details": {
      "risk_level": "dangerous",
      "validator_applied": "sandbox",
      "duration_ms": 1200
    }
  }
}
```

#### Audit Event Delivery

The kernel acts as an **MCP client** to all connected servers, including the Audit server.
This means audit event delivery uses the standard MCP Client → Server notification mechanism:

```
Kernel (MCP Client)                    Audit MGP Server
  │                                      │
  │  notifications/mgp.audit             │
  │─────────────────────────────────────>│  (standard Client → Server notification)
  │                                      │
  │  notifications/mgp.audit             │
  │─────────────────────────────────────>│  (each event is a separate notification)
```

The kernel SHOULD forward `notifications/mgp.audit` to all connected servers that
declared `audit` in their negotiated extensions (§2). If no Audit server is connected,
the kernel SHOULD log events locally.

### 6.4 Standard Event Types

| Event Type | Description |
|-----------|-------------|
| `TOOL_EXECUTED` | A tool was called and completed |
| `TOOL_BLOCKED` | A tool call was blocked by validation or access control |
| `PERMISSION_GRANTED` | A permission was approved |
| `PERMISSION_DENIED` | A permission was denied |
| `PERMISSION_REVOKED` | A previously granted permission was revoked |
| `ACCESS_GRANTED` | Agent access to a server/tool was granted |
| `ACCESS_REVOKED` | Agent access was revoked |
| `SERVER_CONNECTED` | An MGP/MCP server connected |
| `SERVER_DISCONNECTED` | A server disconnected |
| `VALIDATION_FAILED` | Kernel-side validation rejected a tool call |
| `CODE_REJECTED` | Code safety framework rejected submitted code |
| `TOOL_CREATED_DYNAMIC` | A tool was dynamically generated via Active Tool Request (§16.6) |

Implementations MAY define custom event types using reverse-domain notation
(e.g., `com.example.custom_event`).

### 6.5 Trace ID Propagation

Every request from the client SHOULD include a `trace_id` in the `params` object (or as a
top-level field in MGP-extended requests). Servers SHOULD propagate this trace ID in
their audit notifications to enable distributed tracing across multi-server configurations.

---

## 7. Code Safety Framework

### 7.1 Overview

For tools that accept code as input (e.g., dynamic server creation, code execution), MGP
defines a standard safety framework with validation levels and response formats.

### 7.2 Safety Levels

| Level | Description | Validation |
|-------|-------------|------------|
| `unrestricted` | No code restrictions | None |
| `standard` | Block known dangerous patterns | Import blocklist + pattern blocklist |
| `strict` | Allowlist-only imports, max size limits | Import allowlist + pattern blocklist + size limit |
| `readonly` | Code may only read data, no side effects | All of strict + no write operations |

### 7.3 Validation Declaration

Servers that accept code input SHOULD declare the safety level in their tool security metadata:

```json
{
  "name": "create_mcp_server",
  "security": {
    "risk_level": "dangerous",
    "permissions_required": ["code_execution"],
    "validator": "code_safety",
    "code_safety": {
      "level": "standard",
      "language": "python",
      "max_code_size_bytes": 10000,
      "blocked_imports": ["subprocess", "shutil", "socket", "ctypes"],
      "blocked_patterns": ["eval(", "exec(", "__import__(", "os.system"],
      "allowed_imports": ["asyncio", "json", "httpx", "os", "datetime", "typing"]
    }
  }
}
```

### 7.4 Validation Response Format

When code is rejected, the tool SHOULD return a structured rejection:

```json
{
  "status": "rejected",
  "reason": "Code validation failed",
  "violations": [
    "Blocked import: 'subprocess'",
    "Blocked pattern: 'eval('"
  ],
  "hints": {
    "blocked_imports": ["subprocess", "shutil"],
    "allowed_imports": ["asyncio", "json", "httpx"],
    "max_code_size_bytes": 10000
  }
}
```

This format enables AI agents to self-correct their code without human intervention.

---

---

# Part II: Communication & Lifecycle Layer

---

## 11. Lifecycle Management — Notification + Kernel Tool Layer

### 11.1 Overview

MGP defines lifecycle management through a combination of **protocol notifications**
(Layer 2) and **kernel tools** (Layer 4). MCP provides no lifecycle primitives —
servers are either running or not, with no protocol-level health monitoring or
graceful shutdown.

- **Layer 2:** `notifications/mgp.lifecycle` — state transition notifications
- **Layer 4:** `mgp.health.ping`, `mgp.health.status`, `mgp.lifecycle.shutdown` — kernel tools

### 11.2 Server State Machine

```
                    ┌──────────────┐
                    │  Registered  │ (config loaded, not started)
                    └──────┬───────┘
                           │ start
                           ▼
    ┌──────────┐    ┌──────────────┐    ┌──────────────┐
    │  Error   │◄───│ Connecting   │───►│  Connected   │
    └────┬─────┘    └──────────────┘    └──────┬───────┘
         │                                      │ shutdown request
         │ restart                              ▼
         │               ┌──────────────┐    ┌──────────────┐
         └──────────────►│  Restarting  │◄───│   Draining   │
                         └──────┬───────┘    └──────────────┘
                                │                   │
                                ▼                   ▼ (drain complete)
                         ┌──────────────┐    ┌──────────────┐
                         │  Connected   │    │ Disconnected │
                         └──────────────┘    └──────────────┘
```

**States:**

| State | Description |
|-------|-------------|
| `registered` | Server configuration loaded but not yet started |
| `connecting` | Transport initializing, handshake in progress |
| `connected` | Operational — accepting tool calls |
| `draining` | Graceful shutdown initiated — finishing in-flight requests, rejecting new ones |
| `disconnected` | Transport closed, server stopped |
| `error` | Connection failed or runtime error |
| `restarting` | Server is being stopped and restarted |

### 11.3 Health Check — Kernel Tools

#### mgp.health.ping

**Tool Name:** `mgp.health.ping`
**Category:** Kernel Tool (Layer 4)

A lightweight liveness check. Servers MUST respond within 5 seconds.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "server_id": { "type": "string", "description": "Target server to check" }
  },
  "required": ["server_id"]
}
```

**Output:**
```json
{
  "status": "healthy",
  "timestamp": "2026-02-27T12:00:00.005Z",
  "uptime_secs": 3600,
  "server_id": "mind.cerebras"
}
```

**Status Values:**

| Status | Meaning |
|--------|---------|
| `healthy` | Server is fully operational |
| `degraded` | Server is running but some capabilities are limited |
| `unhealthy` | Server is experiencing errors but still responding |

#### mgp.health.status

**Tool Name:** `mgp.health.status`
**Category:** Kernel Tool (Layer 4)

Detailed readiness check including resource usage and capability status.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "server_id": { "type": "string", "description": "Target server to check" }
  },
  "required": ["server_id"]
}
```

**Output:**
```json
{
  "status": "healthy",
  "uptime_secs": 3600,
  "tools_available": 3,
  "tools_total": 3,
  "pending_requests": 0,
  "resources": {
    "memory_bytes": 52428800,
    "open_connections": 2
  },
  "checks": {
    "api_key_configured": true,
    "model_reachable": true,
    "database_connected": true
  }
}
```

The `resources` and `checks` objects are server-defined. Clients SHOULD NOT depend on specific
keys being present.

### 11.4 Graceful Shutdown — Kernel Tool

#### mgp.lifecycle.shutdown

**Tool Name:** `mgp.lifecycle.shutdown`
**Category:** Kernel Tool (Layer 4)

Request a server to shut down gracefully. The server finishes in-flight requests, transitions
to `draining` state, and then closes the transport.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "server_id": { "type": "string", "description": "Target server" },
    "reason": { "type": "string", "enum": ["operator_request", "configuration_change", "resource_limit", "idle_timeout", "kernel_shutdown"] },
    "timeout_ms": { "type": "number", "description": "Max drain time in milliseconds" }
  },
  "required": ["server_id", "reason"]
}
```

**Output:**
```json
{
  "accepted": true,
  "pending_requests": 2,
  "estimated_drain_ms": 5000
}
```

### 11.5 Lifecycle Notifications — Protocol Layer

#### notifications/mgp.lifecycle

State transition notification emitted by the server. This is a **Layer 2 protocol
notification**, not a kernel tool.

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/mgp.lifecycle",
  "params": {
    "server_id": "mind.cerebras",
    "previous_state": "connected",
    "new_state": "draining",
    "reason": "operator_request",
    "timestamp": "2026-02-27T12:00:00.000Z"
  }
}
```

### 11.6 Restart Policies

Defined in the server configuration (not negotiated at runtime).

| Policy | Behavior |
|--------|----------|
| `never` | Do not restart on failure |
| `on_failure` | Restart only when the server exits with an error |
| `always` | Restart on any exit (includes graceful shutdown) |

**Restart Configuration:**

```json
{
  "restart_policy": "on_failure",
  "max_restarts": 5,
  "restart_window_secs": 300,
  "backoff_base_ms": 1000,
  "backoff_max_ms": 30000
}
```

If `max_restarts` is exceeded within `restart_window_secs`, the server transitions to
`error` state and stops retrying. The client SHOULD emit a `SERVER_DISCONNECTED` audit event
with a `restart_limit_exceeded` detail.

---

## 12. Streaming

### 12.1 Overview

MCP tool calls are synchronous: the client sends a request and waits for a complete response.
For LLM-powered tools (token-by-token generation) or long-running operations, this creates
poor UX and timeout risks.

MGP defines streaming as an optional capability where servers can emit partial results
before the final response.

### 12.2 Capability Declaration

```json
{
  "mgp": {
    "version": "0.1.0",
    "extensions": ["streaming"]
  }
}
```

### 12.3 Stream Initiation

When a client calls a tool, it MAY include a `stream` parameter to request streaming:

```json
{
  "jsonrpc": "2.0",
  "id": 20,
  "method": "tools/call",
  "params": {
    "name": "think",
    "arguments": {
      "agent_id": "agent.cloto_default",
      "message": "Explain quantum computing"
    },
    "_mgp": {
      "stream": true
    }
  }
}
```

The `_mgp` field is ignored by standard MCP servers (unknown fields are discarded).

### 12.4 Stream Chunks

The server emits partial results as notifications before the final response:

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/mgp.stream.chunk",
  "params": {
    "request_id": 20,
    "index": 0,
    "content": {
      "type": "text",
      "text": "Quantum computing is"
    },
    "done": false
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/mgp.stream.chunk",
  "params": {
    "request_id": 20,
    "index": 1,
    "content": {
      "type": "text",
      "text": " a paradigm that uses"
    },
    "done": false
  }
}
```

### 12.5 Stream Completion

The final response is a standard JSON-RPC response to the original request:

```json
{
  "jsonrpc": "2.0",
  "id": 20,
  "result": {
    "content": [
      { "type": "text", "text": "Quantum computing is a paradigm that uses..." }
    ],
    "_mgp": {
      "streamed": true,
      "chunks_sent": 15,
      "duration_ms": 3200
    }
  }
}
```

The complete text is included in the final response for clients that did not process chunks.
This ensures backward compatibility: even if a client ignores `notifications/mgp.stream.chunk`,
it still receives the full result.

### 12.6 Progress Reporting

For non-streaming long operations, servers can report progress:

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/mgp.stream.progress",
  "params": {
    "request_id": 21,
    "progress": 0.65,
    "message": "Processing batch 13/20",
    "estimated_remaining_ms": 4500
  }
}
```

### 12.7 Cancellation

Clients can cancel an in-flight streaming or long-running request:

**Method:** `mgp/stream/cancel`

```json
{
  "jsonrpc": "2.0",
  "id": 22,
  "method": "mgp/stream/cancel",
  "params": {
    "request_id": 20,
    "reason": "user_cancelled"
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 22,
  "result": {
    "cancelled": true,
    "partial_result": {
      "content": [
        { "type": "text", "text": "Quantum computing is a paradigm that uses..." }
      ]
    }
  }
}
```

The server SHOULD return any partial results accumulated before cancellation.

---

## 13. Bidirectional Communication

### 13.1 Overview

Standard MCP is primarily unidirectional: the client calls tools on the server. MGP adds
standardized patterns for server-initiated communication — event subscriptions, push
notifications, and callback requests.

- **Layer 2 (Protocol Notifications):** `notifications/mgp.callback.request` — server requests
  information from the kernel during tool execution
- **Layer 3 (Protocol Methods):** `mgp/callback/respond` — kernel responds to callback requests
- **Layer 4 (Kernel Tools):** `mgp.events.subscribe`, `mgp.events.unsubscribe` — event
  subscription management

### 13.2 Event Subscription — Kernel Tools

#### mgp.events.subscribe

**Tool Name:** `mgp.events.subscribe`
**Category:** Kernel Tool (Layer 4)

Subscribe to server-defined event channels.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "channels": { "type": "array", "items": { "type": "string" }, "description": "Event channels to subscribe to" },
    "filter": {
      "type": "object",
      "properties": {
        "min_severity": { "type": "string", "enum": ["info", "warning", "error"] }
      }
    }
  },
  "required": ["channels"]
}
```

**Output:**
```json
{
  "subscribed": ["model.token_usage", "system.error"],
  "unsupported": [],
  "subscription_id": "sub-001"
}
```

#### mgp.events.unsubscribe

**Tool Name:** `mgp.events.unsubscribe`
**Category:** Kernel Tool (Layer 4)

Cancel an existing event subscription.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "subscription_id": { "type": "string" }
  },
  "required": ["subscription_id"]
}
```

### 13.3 Server Push Notifications — Protocol Layer

After subscription, the server emits events as **Layer 2 protocol notifications**:

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/mgp.event",
  "params": {
    "subscription_id": "sub-001",
    "channel": "model.token_usage",
    "timestamp": "2026-02-27T12:05:00.000Z",
    "data": {
      "tokens_used": 1500,
      "tokens_remaining": 8500,
      "model": "llama3.1-70b"
    }
  }
}
```

### 13.4 Callback Requests

Servers can request information from the client during tool execution. This enables
human-in-the-loop workflows without blocking the entire protocol.

#### notifications/mgp.callback.request

Server → Client (as notification with a callback_id for response routing):

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/mgp.callback.request",
  "params": {
    "callback_id": "cb-001",
    "request_id": 20,
    "type": "confirmation",
    "message": "This operation will delete 15 files. Continue?",
    "options": ["confirm", "cancel"],
    "timeout_ms": 60000
  }
}
```

#### mgp/callback/respond

Client → Server:

```json
{
  "jsonrpc": "2.0",
  "id": 32,
  "method": "mgp/callback/respond",
  "params": {
    "callback_id": "cb-001",
    "response": "confirm"
  }
}
```

**Callback Types:**

| Type | Description |
|------|-------------|
| `confirmation` | Yes/no confirmation for dangerous operations |
| `input` | Request additional input from the user |
| `selection` | Present options for the user to choose from |
| `notification` | Informational — no response required |
| `llm_completion` | Request LLM completion from the host (MCP Sampling equivalent) |

The `llm_completion` callback type enables MCP servers to request LLM completions
from the kernel without holding API keys. This is the MGP equivalent of
MCP's `sampling/createMessage` primitive. The kernel holds all LLM provider
credentials and routes requests to the appropriate provider based on `model_hints`.

**llm_completion request params:**

```json
{
  "callback_id": "llm-001",
  "type": "llm_completion",
  "messages": [
    { "role": "system", "content": "..." },
    { "role": "user", "content": "..." }
  ],
  "model_hints": {
    "speed_priority": 0.7,
    "intelligence_priority": 0.5,
    "provider": "deepseek"
  },
  "tools": [],
  "timeout_ms": 120000
}
```

**llm_completion response:**

```json
{
  "callback_id": "llm-001",
  "response": {
    "content": "...",
    "model": "deepseek-chat",
    "usage": { "prompt_tokens": 25, "completion_tokens": 10 },
    "tool_calls": []
  }
}
```

#### Relationship to MCP `sampling/createMessage`

MCP defines `sampling/createMessage` as a dedicated method for the same purpose. MGP's
`llm_completion` callback achieves the same goal through the generic callback mechanism
(§13.4), with the following key differences:

| Aspect | MCP Sampling | MGP `llm_completion` |
|--------|-------------|----------------------|
| Mechanism | Dedicated protocol method | Callback type (extensible) |
| Streaming | Not supported (atomic) | §12 chunk delivery |
| Timeout / Cancel | Not defined | `timeout_ms` + `mgp/stream/cancel` |
| Audit | None | §6 audit with trace_id |
| Access control | None | §5 hierarchy |
| Error handling | 2 codes (`-1`, `-32602`) | §14 structured codes with recovery |
| Extensibility | New method per feature | New callback type, no protocol change |

Per §1.7 (Migration Policy), if MCP Sampling evolves to match these capabilities, MGP
will provide a compatibility layer during the transition period.

### 13.5 Standard Event Channels

| Channel Pattern | Description |
|----------------|-------------|
| `model.*` | LLM-related events (token usage, rate limits, errors) |
| `system.*` | System-level events (resource usage, errors) |
| `tool.*` | Tool execution events (started, completed, failed) |
| `security.*` | Security events (access denied, validation failed) |

Servers define their own channels within these patterns. Clients SHOULD NOT assume specific
channels exist — use the `mgp.events.subscribe` kernel tool to discover available channels.

---

## 14. Error Handling

### 14.1 Overview

MCP inherits JSON-RPC 2.0 error codes but defines no protocol-specific error semantics.
MGP extends the error model with structured error categories, recovery hints, and retry
guidance.

### 14.2 MGP Error Code Ranges

JSON-RPC 2.0 reserves codes -32768 to -32000. MGP defines application-level codes:

| Code Range | Category |
|-----------|----------|
| -32600 to -32603 | JSON-RPC standard errors (parse, invalid request, method not found, invalid params) |
| 1000–1099 | Security errors |
| 2000–2099 | Lifecycle errors |
| 3000–3099 | Resource errors |
| 4000–4099 | Validation errors |
| 5000–5099 | External service errors |

### 14.3 Standard Error Codes

| Code | Name | Description |
|------|------|-------------|
| 1000 | `PERMISSION_DENIED` | Caller lacks required permission |
| 1001 | `ACCESS_DENIED` | Agent does not have access to this tool |
| 1002 | `AUTH_REQUIRED` | Authentication is required |
| 1003 | `AUTH_EXPIRED` | Authentication credentials have expired |
| 1010 | `VALIDATION_BLOCKED` | Kernel-side validator blocked the request |
| 1011 | `CODE_SAFETY_VIOLATION` | Code safety framework rejected the code |
| 2000 | `SERVER_NOT_READY` | Server is not in `connected` state |
| 2001 | `SERVER_DRAINING` | Server is shutting down, not accepting new requests |
| 2002 | `SERVER_RESTARTING` | Server is restarting |
| 3000 | `RATE_LIMITED` | Too many requests |
| 3001 | `RESOURCE_EXHAUSTED` | Server resource limit reached (memory, connections, etc.) |
| 3002 | `QUOTA_EXCEEDED` | Usage quota exceeded (tokens, API calls, etc.) |
| 3003 | `TIMEOUT` | Operation timed out |
| 4000 | `INVALID_TOOL_ARGS` | Tool arguments failed validation |
| 4001 | `TOOL_NOT_FOUND` | Requested tool does not exist |
| 4002 | `TOOL_DISABLED` | Tool exists but is currently disabled |
| 5000 | `UPSTREAM_ERROR` | External API returned an error |
| 5001 | `UPSTREAM_TIMEOUT` | External API timed out |
| 5002 | `UPSTREAM_UNAVAILABLE` | External API is unreachable |

### 14.4 Extended Error Response

MGP errors include a `_mgp` object with recovery information:

```json
{
  "jsonrpc": "2.0",
  "id": 20,
  "error": {
    "code": 3000,
    "message": "Rate limited: 10 requests per minute exceeded",
    "data": {
      "_mgp": {
        "category": "resource",
        "retryable": true,
        "retry_after_ms": 5000,
        "retry_strategy": "exponential_backoff",
        "max_retries": 3,
        "details": {
          "limit": 10,
          "window_secs": 60,
          "current": 12
        }
      }
    }
  }
}
```

### 14.5 Recovery Fields

| Field | Type | Description |
|-------|------|-------------|
| `category` | string | Error category: `security`, `lifecycle`, `resource`, `validation`, `external` |
| `retryable` | boolean | Whether the client should retry the request |
| `retry_after_ms` | number | Minimum time to wait before retrying |
| `retry_strategy` | string | `immediate`, `fixed_delay`, `exponential_backoff` |
| `max_retries` | number | Maximum number of retry attempts |
| `fallback_tool` | string | Alternative tool the client can try |
| `details` | object | Error-specific details (server-defined) |

### 14.6 Client Retry Behavior

When `retryable` is `true`, the client SHOULD:

1. Wait at least `retry_after_ms` milliseconds
2. Apply the specified `retry_strategy`
3. Stop after `max_retries` attempts
4. If `fallback_tool` is provided, try the alternative tool after all retries are exhausted
5. Emit an audit event for each retry attempt

When `retryable` is `false`, the client MUST NOT retry and SHOULD report the error.

---

## 15. Discovery — Configuration + Kernel Tool Layer

### 15.1 Overview

MGP defines server discovery through **static configuration** (protocol-level concept)
and **runtime registry tools** (Layer 4 kernel tools).

### 15.2 Server Advertisement

MGP servers MAY advertise themselves via a well-known configuration file or registry
kernel tools.

#### Configuration File Discovery

Clients look for MGP servers in these locations (in order):

1. `./mgp.toml` — Project-local configuration
2. `~/.config/mgp/servers.toml` — User-level configuration
3. `$MGP_CONFIG_PATH` — Environment variable override

**Format (mgp.toml):**

```toml
[[servers]]
id = "mind.cerebras"
command = "python"
args = ["mcp-servers/cerebras/server.py"]
transport = "stdio"

[servers.mgp]
extensions = ["security", "lifecycle", "streaming"]
permissions_required = ["network.outbound"]
trust_level = "standard"
restart_policy = "on_failure"

[servers.env]
CEREBRAS_API_KEY = "${CEREBRAS_API_KEY}"
```

The `[servers.mgp]` section is OPTIONAL. If omitted, the server is treated as standard MCP.
The file format is backward compatible with MCP configuration files — the `mgp` section
is ignored by MCP-only clients.

### 15.3 Capability Advertisement

Connected servers advertise their capabilities via the `initialize` response (§2). For
pre-connection discovery, the `mgp.toml` configuration provides the same information
without establishing a transport connection.

### 15.4 Registry — Kernel Tools

For distributed environments and runtime discovery, the kernel exposes registry tools.

#### mgp.discovery.list

**Tool Name:** `mgp.discovery.list`
**Category:** Kernel Tool (Layer 4)

Query connected and registered servers.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "filter": {
      "type": "object",
      "properties": {
        "extensions": { "type": "array", "items": { "type": "string" } },
        "permissions": { "type": "array", "items": { "type": "string" } },
        "status": { "type": "string", "enum": ["connected", "disconnected", "all"] }
      }
    }
  }
}
```

**Output:**
```json
{
  "servers": [
    {
      "id": "mind.cerebras",
      "status": "connected",
      "mgp_version": "0.1.0",
      "extensions": ["security", "lifecycle", "streaming"],
      "tools": ["think", "analyze"],
      "trust_level": "standard"
    }
  ]
}
```

#### mgp.discovery.register

**Tool Name:** `mgp.discovery.register`
**Category:** Kernel Tool (Layer 4)

Register a server created at runtime (e.g., by agents).

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "id": { "type": "string", "description": "Server identifier" },
    "command": { "type": "string" },
    "args": { "type": "array", "items": { "type": "string" } },
    "transport": { "type": "string", "enum": ["stdio", "http"] },
    "mgp": {
      "type": "object",
      "properties": {
        "extensions": { "type": "array", "items": { "type": "string" } },
        "permissions_required": { "type": "array", "items": { "type": "string" } },
        "trust_level": { "type": "string", "enum": ["trusted", "standard", "sandboxed"] }
      }
    },
    "created_by": { "type": "string" },
    "justification": { "type": "string" }
  },
  "required": ["id", "command", "transport"]
}
```

Dynamic registrations with `trust_level: "sandboxed"` are subject to stricter validation
(code safety framework, limited permissions) than `standard` or `trusted` servers.

#### mgp.discovery.deregister

**Tool Name:** `mgp.discovery.deregister`
**Category:** Kernel Tool (Layer 4)

Remove a dynamically registered server.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "id": { "type": "string", "description": "Server to deregister" },
    "reason": { "type": "string" }
  },
  "required": ["id"]
}
```

---

# Part III: Intelligence Layer

---

## 16. Dynamic Tool Discovery & Active Tool Request — Kernel Tool Layer

### 16.1 Overview

The most significant structural limitation of MCP is that **all tool definitions must be
injected into the LLM's context before use**. This creates two compounding problems:

1. **Context Overhead**: At ~400-500 tokens per tool, 50 tools consume 20,000-25,000 tokens.
   Large ecosystems exceed 200,000 tokens — degrading model reasoning quality and crowding
   out actual task context.

2. **Passive Tool Consumption**: The LLM can only select from tools it already knows about.
   If a tool is not in the prompt, it cannot be used — regardless of how relevant it is.
   This forces a fundamentally passive model where agents wait to be told what tools exist.

MGP solves both problems at the protocol level with two complementary mechanisms:

- **Dynamic Tool Discovery** (Mode A): The LLM searches for tools based on intent
- **Active Tool Request** (Mode B): The LLM autonomously identifies capability gaps and
  requests tools during task execution

Together, these reduce context usage by up to 99% while enabling fully autonomous tool
acquisition without explicit user instruction.

**Strategic Significance:** Dynamic Tool Discovery is MGP's primary structural
differentiator from MCP (see §1.6 Migration Policy). MCP's architecture requires all
tool schemas to be injected into the LLM context before use and has no planned mechanism
for runtime tool search or autonomous tool acquisition. This structural gap is unlikely
to be addressed by MCP in the near term because it requires a kernel/orchestrator layer
that is not part of MCP's direct client-server model.

### 16.2 Capability Declaration

```json
{
  "mgp": {
    "version": "0.2.0",
    "extensions": ["tool_discovery"]
  }
}
```

When `tool_discovery` is negotiated, the client MAY omit most tool definitions from the
LLM context. Instead, the LLM receives a single meta-tool (`mgp.tools.discover`) and
optionally a small set of pinned core tools.

### 16.3 Context Reduction Model

```
L0 (Standard MCP):  All tools in context           ~150,000 tokens
L1 (Category):      Category index only              ~5,000 tokens
L2 (Discovery):     Meta-tool + on-demand results    ~1,000 tokens
L3 (Hybrid):        Pinned tools + discovery cache   ~2,000 tokens
```

MGP clients SHOULD implement L3 (Hybrid) for optimal balance between performance and
autonomous capability. L2 is the minimum for `tool_discovery` compliance.

### 16.4 Tool Index

The kernel maintains a searchable index of all tools across all connected servers. The index
contains:

```json
{
  "tool_id": "filesystem.read_file",
  "server_id": "tool.terminal",
  "name": "read_file",
  "description": "Read the contents of a file at the given path",
  "categories": ["filesystem", "read"],
  "keywords": ["file", "read", "open", "content", "text"],
  "security": {
    "risk_level": "moderate",
    "permissions_required": ["filesystem.read"]
  },
  "embedding": [0.012, -0.034, ...]   // OPTIONAL — see below
}
```

The index supports three search strategies:

| Strategy | Method | Best For |
|----------|--------|----------|
| Keyword | Exact and fuzzy keyword matching | Precise tool names |
| Semantic | Embedding vector similarity | Natural language intent |
| Category | Hierarchical category filtering | Browsing available capabilities |

#### Semantic Search is Optional

The `embedding` field in the Tool Index is **OPTIONAL**. Implementations that do not
provide embedding vectors cannot use the `semantic` search strategy, but this does not
affect protocol compliance.

**Keyword + Category search alone is sufficient for `tool_discovery` extension
compliance.** A conforming implementation MUST support at least keyword search and
category filtering. Semantic search is an enhancement for improved natural-language
matching but is not required.

For implementations that want semantic search without running a local embedding model:

| Approach | Description |
|----------|-------------|
| **Server-side** | Each MGP server generates embeddings for its own tools and includes them in `tools/list` responses |
| **Dedicated service** | An Embedding MGP server (e.g., `tool.embedding`) generates embeddings on demand via a tool call |
| **Pre-computed** | Embeddings are computed at build/deploy time and stored in the tool index configuration |
| **None** | Keyword + category search only. No embedding model required. |

When `strategy: "semantic"` is requested but the kernel has no embeddings available,
it SHOULD fall back to keyword search and include `"fallback_strategy": "keyword"` in
the response metadata.

### 16.5 Mode A: Dynamic Tool Discovery

The LLM searches for tools based on a natural language description of what it needs.
This is the **user-intent-driven** mode — the LLM translates the user's request into a
tool search.

#### mgp.tools.discover

**Tool Name:** `mgp.tools.discover`
**Category:** Kernel Tool (Layer 4)

Search for tools based on natural language description.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "query": { "type": "string", "description": "Natural language description of needed capability" },
    "strategy": { "type": "string", "enum": ["keyword", "semantic", "category"], "default": "keyword" },
    "max_results": { "type": "number", "default": 5 },
    "filter": {
      "type": "object",
      "properties": {
        "categories": { "type": "array", "items": { "type": "string" } },
        "risk_level_max": { "type": "string", "enum": ["safe", "moderate", "dangerous"] },
        "status": { "type": "string", "enum": ["connected", "all"] }
      }
    }
  },
  "required": ["query"]
}
```

**Output:**
```json
{
  "tools": [
    {
      "name": "read_file",
      "server_id": "tool.terminal",
      "description": "Read the contents of a file at the given path",
      "relevance_score": 0.95,
      "inputSchema": {
        "type": "object",
        "properties": {
          "path": { "type": "string", "description": "File path to read" }
        },
        "required": ["path"]
      },
      "security": {
        "risk_level": "moderate",
        "permissions_required": ["filesystem.read"],
        "validator": "sandbox"
      }
    },
    {
      "name": "grep",
      "server_id": "tool.terminal",
      "description": "Search file contents using pattern matching",
      "relevance_score": 0.72,
      "inputSchema": { "..." : "..." },
      "security": { "..." : "..." }
    }
  ],
  "total_available": 47,
  "search_strategy": "keyword",
  "query_time_ms": 12
}
```

The response includes **full tool schemas** for the top results, allowing the LLM to
immediately call any discovered tool without a second round trip.

#### Flow Diagram

```
User: "このファイルの中身を見せて"
  │
  ▼
LLM context: [mgp.tools.discover meta-tool] + [user message]
  │
  ▼ LLM decides it needs file-reading capability
  │
  ▼ tools/call → mgp.tools.discover({ query: "read file contents" })
  │
  ▼ Kernel searches tool index
  │
  ▼ Returns: read_file (0.95), grep (0.72), cat (0.68)
  │
  ▼ LLM selects read_file, calls it with { path: "..." }
  │
  ▼ Result returned to user
```

### 16.6 Mode B: Active Tool Request

The LLM autonomously detects a capability gap **during task execution** and requests new
tools without user intervention. This is the **agent-autonomy-driven** mode.

Unlike Mode A (which responds to user intent), Mode B enables proactive behavior:
the agent recognizes "I cannot complete this step with my current tools" and initiates
tool acquisition independently.

#### mgp.tools.request

**Tool Name:** `mgp.tools.request`
**Category:** Kernel Tool (Layer 4)

Request tools to fill a capability gap during task execution.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "reason": { "type": "string", "enum": ["capability_gap", "performance", "preference"] },
    "context": { "type": "string", "description": "Why the tool is needed" },
    "requirements": {
      "type": "object",
      "properties": {
        "capabilities": { "type": "array", "items": { "type": "string" } },
        "input_types": { "type": "array", "items": { "type": "string" } },
        "output_types": { "type": "array", "items": { "type": "string" } },
        "preferred_risk_level": { "type": "string", "enum": ["safe", "moderate", "dangerous"] }
      }
    },
    "task_trace_id": { "type": "string", "description": "Trace ID for audit" }
  },
  "required": ["reason", "context", "requirements"]
}
```

**Output:**
```json
{
    "status": "fulfilled",
    "tools_loaded": [
      {
        "name": "analyze_csv",
        "server_id": "tool.data_processing",
        "description": "Compute statistics on CSV data",
        "inputSchema": { "..." : "..." },
        "security": {
          "risk_level": "safe",
          "permissions_required": ["memory.read"]
        }
      }
    ],
    "tools_unavailable": [],
    "session_tools_count": 4,
    "context_tokens_added": 380
  }
}
```

**Request Status Values:**

| Status | Meaning |
|--------|---------|
| `fulfilled` | Matching tools found and loaded into session |
| `partial` | Some requirements met, others unavailable |
| `unavailable` | No matching tools found |
| `pending_approval` | Tools found but require permission approval (§3) |
| `creating` | No existing tools match — tool creation initiated if enabled (§7) |

#### The `creating` Status — Autonomous Tool Generation

**Default: DISABLED.** The `creating` status is only available when explicitly opted
in during capability negotiation:

```json
{
  "mgp": {
    "version": "0.4.0",
    "extensions": ["tool_discovery"],
    "tool_creation": { "enabled": true }
  }
}
```

When `status: "creating"` is returned, the kernel has determined that no existing tool
satisfies the requirement and tool creation is enabled. The following safety guardrails
apply:

##### Safety Guardrails

1. **Opt-in required**: The client MUST declare `tool_creation: { enabled: true }` in
   the capability negotiation (§2). Without this, `creating` status is never returned.

2. **Ephemeral by default**: Generated tools are **session-scoped** and automatically
   deregistered when the session ends. They are NOT persisted to the tool index.

3. **Trust level**: Generated tools always receive `trust_level: "experimental"`.
   They MUST NOT inherit the trust level of the requesting agent or server.

4. **Code safety validation**: All generated tool code MUST pass Code Safety Framework
   (§7) validation at the `strict` level before registration.

5. **Approval policy applies**: Under `interactive` policy, the operator MUST approve
   the generated tool before it becomes available. Under `auto_approve`, the tool is
   registered immediately after passing safety validation.

6. **Audit trail**: A `TOOL_CREATED_DYNAMIC` audit event (§6.4) MUST be emitted for
   every dynamically generated tool, including the generating agent, tool code hash,
   and safety validation result.

##### Flow

When all guardrails pass, the kernel:

1. Instructs the agent to generate tool code via the Code Safety Framework (§7)
2. Validates the code at `strict` safety level
3. Registers the tool via Dynamic Registration (§15.4) as ephemeral
4. Emits `TOOL_CREATED_DYNAMIC` audit event
5. Returns the newly created tool in a follow-up response

This closes the loop: **discover → request → create → use** — autonomous tool
lifecycle with mandatory safety controls.

#### Flow Diagram

```
LLM executing multi-step task
  │
  ├─ Step 1: Read CSV file ✓ (tool available)
  │
  ├─ Step 2: Parse data ✓ (tool available)
  │
  ├─ Step 3: Statistical analysis ✗ (no tool available)
  │    │
  │    ▼ LLM detects capability gap
  │    │
  │    ▼ tools/call → mgp.tools.request({
  │    │     reason: "capability_gap",
  │    │     context: "need statistical analysis",
  │    │     requirements: { capabilities: ["statistics"] }
  │    │  })
  │    │
  │    ▼ Kernel searches → finds tool.data_processing
  │    │
  │    ▼ Returns analyze_csv tool with full schema
  │    │
  │    ▼ LLM calls analyze_csv ✓
  │
  ├─ Step 4: Generate report ✓
  │
  ▼ Task complete
```

### 16.7 Session Tool Cache

To avoid repeated discovery calls, the kernel maintains a per-session tool cache:

| Category | Behavior |
|----------|----------|
| **Pinned tools** | Always in context (configured per agent, e.g., `think`, `store`) |
| **Session cache** | Tools used in current session — retained until session ends |
| **Discovery results** | Cached for the duration of the request — discarded after use |

#### mgp.tools.session

**Tool Name:** `mgp.tools.session`
**Category:** Kernel Tool (Layer 4)

Query the current session's loaded tools.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {}
}
```

**Output:**
```json
{
  "pinned": ["think", "store", "recall"],
  "cached": ["read_file", "analyze_csv"],
  "total_tokens": 2100,
  "max_tokens": 8000
}
```

#### mgp.tools.session.evict

**Tool Name:** `mgp.tools.session.evict`
**Category:** Kernel Tool (Layer 4)

Remove tools from the session cache to free context space.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "tools": { "type": "array", "items": { "type": "string" }, "description": "Tool names to evict" },
    "reason": { "type": "string" }
  },
  "required": ["tools"]
}
```

### 16.8 Context Budget

The kernel enforces a **context budget** for tool definitions:

```json
{
  "tool_context_budget": {
    "max_tokens": 8000,
    "pinned_reserve": 2000,
    "discovery_reserve": 3000,
    "cache_limit": 3000
  }
}
```

When the budget is exceeded, the kernel automatically evicts the least-recently-used
cached tools. Pinned tools are never evicted. Discovery results that would exceed the
budget are truncated (fewer results returned).

#### Kernel Tool Visibility

Layer 4 Kernel Tools (`mgp.access.*`, `mgp.health.*`, `mgp.events.*`, etc.) are
management tools intended for operators and administrative agents. They SHOULD NOT be
included in the LLM's tool context by default:

| Kernel Tool Category | `tools/list` | LLM Context | Rationale |
|---------------------|-------------|------------|-----------|
| `mgp.tools.discover` | Yes | Yes (as meta-tool) | LLM needs this for dynamic discovery |
| `mgp.tools.request` | Yes | Yes (as meta-tool) | LLM needs this for active tool request |
| `mgp.access.*` | Yes | No | Administrative — operator/API only |
| `mgp.health.*` | Yes | No | Administrative — monitoring only |
| `mgp.lifecycle.*` | Yes | No | Administrative — operator only |
| `mgp.events.*` | Yes | No | Administrative — subscription mgmt |
| `mgp.discovery.*` | Yes | No | Administrative — server registration |
| `mgp.tools.session*` | Yes | Optional | Context management — LLM MAY use |

Kernel tools appear in `tools/list` responses (for API discoverability) but are excluded
from the LLM context budget unless explicitly pinned. Only `mgp.tools.discover` and
`mgp.tools.request` are injected into the LLM context as meta-tools.

### 16.9 Comparison with Existing Approaches

| Approach | Discovery | Multi-Step | Protocol Standard | Context Reduction |
|----------|-----------|------------|-------------------|-------------------|
| Standard MCP | None (all tools injected) | N/A | Yes | 0% |
| RAG-MCP | Pre-query semantic retrieval | No | No | ~80% |
| MCP-Zero (paper) | Active tool request | Yes | No (research) | ~95% |
| Cursor/Copilot | Hard limits (40/128 tools) | No | No | Truncation |
| **MGP §16** | **A + B combined** | **Yes** | **Yes** | **~99%** |

MGP is the first protocol to standardize both passive discovery (A) and active request (B)
as first-class protocol methods, with session management and context budgeting built in.

---

## 17. Implementation & Adoption Guide

### 17.1 For Server Implementors

1. **Minimal MGP support**: Include `mgp` in your `initialize` response capabilities.
   Even supporting just the `security` extension (tool security metadata) adds significant
   value for clients.

2. **Graceful degradation**: If the client does not send `mgp` in its `initialize` request,
   behave as a standard MCP server. Do not require MGP support.

3. **Permission declarations**: List all permissions your server needs in
   `permissions_required`. Clients may deny startup if permissions are not granted.

4. **Layer 4 tools are kernel-provided**: Servers do NOT implement kernel tools (§5, §11,
   §13 events, §15, §16). These are exposed by the kernel. Servers only need to respond
   to health checks and lifecycle commands when the kernel invokes them.

### 17.2 For Client/Kernel Implementors

1. **Discovery**: Check for `mgp` in the server's `initialize` response. If absent,
   treat as standard MCP.

2. **Fallback validators**: Even without MGP server support, clients SHOULD apply kernel-side
   validators (sandbox, code_safety) based on tool names and server configuration.

3. **Kernel tools**: Expose Layer 4 tools (§5, §11, §13 events, §15, §16) as standard
   MCP tools via `tools/call`. These tools are invoked by operators and agents, not by
   servers. The kernel is the enforcement point for all access control decisions.

4. **Standard tool names**: Use the `mgp.*` naming convention for kernel tools
   (e.g., `mgp.access.query`, `mgp.tools.discover`). This ensures discoverability and
   avoids naming conflicts with server-provided tools.

### 17.3 Relationship to ClotoCore

ClotoCore is the reference implementation of MGP. The following ClotoCore components
map to MGP specifications:

| MGP Spec | Layer | ClotoCore Component | File |
|----------|-------|-------------------|------|
| §2 Capability Negotiation | 1 | `cloto/handshake` | `managers/mcp.rs` |
| §3 Permission Declarations | 3 | Permission Gate (D) | `managers/mcp.rs` |
| §4 Tool Security Metadata | 1 | `tool_validators` config | `managers/mcp_protocol.rs` |
| §5 Access Control | 4 | `mcp_access_control` table | `db.rs`, `handlers.rs` |
| §6 Audit Trail | 2 | `audit_logs` table | `handlers.rs` |
| §7 Code Safety | 1 | `validate_mcp_code()` | `managers/mcp.rs` |
| §11 Lifecycle | 2+4 | `ServerStatus`, `auto_restart` | `managers/mcp.rs` |
| §12 Streaming | 2+3 | — (not yet implemented) | — |
| §13 Bidirectional | 2+3+4 | SSE event bus, callbacks | `handlers.rs`, `lib.rs` |
| §14 Error Handling | — | `JsonRpcError` | `managers/mcp_protocol.rs` |
| §15 Discovery | 4 | `mcp.toml`, `add_dynamic_server()` | `managers/mcp.rs` |
| §16 Tool Discovery | 4 | — (not yet implemented) | — |

### 17.4 License and Distribution Strategy

| Component | License | Repository |
|-----------|---------|------------|
| MGP Specification | MIT | `mgp-spec` (independent) |
| MGP SDK (Python / TypeScript) | MIT | `mgp-sdk` (independent) |
| MGP Validation Tool | MIT | `mgp-sdk` (bundled) |
| ClotoCore (Reference Implementation) | BSL 1.1 → MIT (2028) | `ClotoCore` (existing) |

MGP specification and SDKs are fully separated from ClotoCore and published under MIT.
Any project can adopt MGP regardless of ClotoCore's commercial protection period.

### 17.5 Staged Adoption Path

MGP does not require implementing all extensions at once. Both clients and servers can
adopt incrementally. Each Tier includes all previous Tiers.

```
Tier 1 ──── Tier 2 ──── Tier 3 ──── Tier 4
 Hours       1 week      2-4 weeks    1-2 months
 Minimal     Security    Communication Full
```

**Layer Mapping:** Tier 1-2 primarily use Layer 1 (Metadata) and Layer 2 (Notifications).
Tier 3-4 additionally use Layer 3 (Protocol Methods) and Layer 4 (Kernel Tools).
Kernel Tools (Layer 4) require no server-side implementation — the kernel provides them.

**Tier 1 — Minimal (hours):** Add `mgp` to `initialize` capabilities + `security` metadata
on `tools/list`. ~80 lines of code for clients, ~70 lines for servers.

```python
# Server: 3 lines to add MGP Tier 1 support
from mgp import enable_mgp
enable_mgp(server, permissions=["network.outbound"], trust_level="standard")
```

**Tier 2 — Security (1 week):** Permission approval flow (§3), audit events (§6),
structured error handling (§14), access control (§5).

**Tier 3 — Communication (2-4 weeks):** Lifecycle management (§11), streaming (§12),
bidirectional communication (§13).

**Tier 4 — Full (1-2 months):** Dynamic tool discovery (§16 Mode A+B), context budget
management, session tool cache. Semantic search is OPTIONAL — keyword + category is
sufficient for Tier 4 compliance.

### 17.6 Implementation Difficulty Matrix

#### Client/Kernel Implementation

| Extension | Tier | Lines (est.) | Difficulty | Dependencies |
|-----------|------|-------------|------------|-------------|
| §2 Negotiation | 1 | ~50 | Very Low | None |
| §4 Security Metadata | 1 | ~30 | Very Low | §2 |
| §3 Permission Approval | 2 | ~200 | Low | §2 |
| §14 Error Handling | 2 | ~100 | Low | None |
| §6 Audit | 2 | ~80 | Low | §2 |
| §5 Access Control (Kernel Tool) | 2 | ~300 (kernel) | Medium | §2 |
| §11 Lifecycle (Kernel Tool) | 3 | ~200 (kernel) | Low-Med | §2 |
| §12 Streaming | 3 | ~400 | Medium | §2 |
| §13 Bidirectional | 3 | ~500 | Medium | §2 |
| §15 Discovery (Kernel Tool) | 3 | ~150 (kernel) | Low | §2 |
| §16 Tool Discovery (Kernel Tool) | 4 | ~800-1500 (kernel) | Med-High | §2, §15 |

#### Server Implementation

| Extension | Tier | Lines (est.) | Difficulty |
|-----------|------|-------------|------------|
| §2 Negotiation Response | 1 | ~40 | Very Low |
| §4 Security Metadata Declaration | 1 | ~20/tool | Very Low |
| §3 Permission Declaration | 1 | ~10 | Very Low |
| §11 Health Check Response | 3 | ~80 | Low |
| §12 Streaming Emission | 3 | ~200 | Medium |
| §13 Event Publishing | 3 | ~150 | Low-Med |

**Server Tier 1 total: ~70 lines.** Just declare `security` fields on tools.

### 17.7 SDK Design

**Principles:** Zero-config, gradual extensions, non-invasive MCP wrapping, type-safe.

**Python SDK:** `mgp/` — `__init__.py`, `types.py`, `negotiate.py`, `security.py`,
`lifecycle.py`, `streaming.py`, `discovery.py`, `audit.py`, `errors.py`, `server.py`

**TypeScript SDK:** `@mgp/sdk/src/` — `index.ts`, `types.ts`, `client.ts`, `server.ts`,
`security.ts`, `lifecycle.ts`, `streaming.ts`, `discovery.ts`, `audit.ts`, `errors.ts`

### 17.8 Validation Tool — mgp-validate

`mgp-validate` tests MGP compliance for servers and clients.

**"5 minutes to MGP-compatible server":** Using `mgp-validate` and the minimal sample
server (`examples/minimal-server/`), a developer can have a working MGP server and pass
compliance tests within 5 minutes.

```bash
mgp-validate server ./my-server.py
# ✓ Tier 1: Capability negotiation ... PASS
# ✓ Tier 1: Security metadata on tools ... PASS
# ✓ Tier 2: Permission declarations ... PASS
# ✗ Tier 3: Health check response ... MISSING
# Result: Tier 2 compliant (6/11 extensions)
```

Compliance badges: `[MGP Tier 1]` `[MGP Tier 2]` `[MGP Tier 3]` `[MGP Tier 4]`

### 17.9 Ecosystem Relationships

| Project | Relationship to MGP |
|---------|-------------------|
| MCP (Anthropic) | Base protocol. MGP is a strict superset of MCP |
| Claude Code | Standard MCP client. MGP Tier 1 enables security metadata |
| Cursor | 40-tool limit. MGP §16 effectively removes this limitation |
| LangChain / LlamaIndex | Tool frameworks. MGP SDK integrates as an adapter |

### 17.10 Roadmap

| Phase | Deliverable | Status |
|-------|-----------|--------|
| Phase 0 | MGP Specification | Draft complete |
| Phase 1 | Python SDK (Tier 1-2) | Concept |
| Phase 2 | TypeScript SDK (Tier 1-2) | Concept |
| Phase 3 | Validation Tool | Concept |
| Phase 4 | SDK Tier 3-4 Extensions | Concept |
| Phase 5 | Independent repo + npm/PyPI publish | Concept |
| Phase 6 | ClotoCore as MGP reference implementation | Concept |

---

## 18. Version History & Review Response

### 18.1 Version History

| Version | Date | Changes |
|---------|------|---------|
| 0.1.0-draft | 2026-02-27 | Initial draft — Security layer (§2-7) |
| 0.2.0-draft | 2026-02-27 | Communication & Lifecycle layer (§11-15) |
| 0.3.0-draft | 2026-02-27 | Intelligence layer — Dynamic Tool Discovery & Active Tool Request (§16) |
| 0.4.0-draft | 2026-02-28 | Expert review response (see §18.2) |
| 0.5.0-draft | 2026-02-28 | Selective Minimalism (see §18.3) |
| 0.5.1-draft | 2026-02-28 | Document consolidation: merged MGP_PATTERNS.md, MGP_ADOPTION.md, MGP_REVIEW_RESPONSE.md into single specification |
| 0.5.2-draft | 2026-02-28 | Second review response: sequential section numbering (§17-19), `notifications/mgp.event` added to Layer 2, kernel tool visibility rules (§16.8), §14 Layer classification, MCP comparison compressed |

### 18.2 Expert Review Response (0.3.0 → 0.4.0)

Expert review of 0.3.0-draft identified 6 concerns and 3 strategic recommendations:

| Concern | Resolution | Sections |
|---------|-----------|----------|
| **MCP superset political vulnerability** | Added §1.7 Migration Policy with deprecation timeline and migration categories | §1.7 |
| **Permission method naming** | Renamed `mgp/permission/request` → `await`, `response` → `grant` | §3.4-3.6 |
| **Semantic search embedding dependency** | Marked `embedding` as OPTIONAL; keyword + category sufficient for compliance | §16.4 |
| **Versioning strategy undefined** | Added §2.5 with 0.x rules and 1.0 stability criteria | §2.5 |
| **Audit event transport** | Explicitly documented kernel as MCP client for notification delivery | §6.3 |
| **`creating` status security risk** | Disabled by default, 6 safety guardrails, ephemeral tools, `TOOL_CREATED_DYNAMIC` event | §16.6, §6.4 |

Strategic additions: §1.7 Migration Policy, §16.1 differentiator emphasis, "5 minutes to
MGP-compatible server" experience in §17.8.

### 18.3 Selective Minimalism (0.4.0 → 0.5.0)

Structural analysis revealed that 16 of 25 protocol methods are kernel-side operations
that do not require bidirectional protocol agreement. Converting these to standard MCP
tools via `tools/call` preserves all functionality while reducing protocol surface area
by 64%.

**Result:** 25 → 10 protocol primitives (4 methods + 6 notifications).

- **Layer 1 (Metadata):** `_mgp` fields on existing MCP messages — 0 new methods
- **Layer 2 (Notifications):** 6 protocol notifications
- **Layer 3 (Methods):** 4 irreducible methods (permission/await, permission/grant,
  callback/respond, stream/cancel)
- **Layer 4 (Kernel Tools):** 16 methods converted to standard MCP tools with `mgp.*`
  naming convention

Security guarantees and MCP structural limitation breakthroughs are fully maintained
because the kernel remains the sole enforcement point regardless of invocation mechanism.

---

## 19. Application Patterns

The following capabilities are intentionally **not part of the MGP protocol specification**.
They can be fully implemented as MGP servers using the existing protocol primitives
(§2-7, §11-16). Each pattern can be deployed independently.

| Pattern | Implementation | Complexity |
|---------|---------------|------------|
| Multi-Agent Coordination | Coordination MGP server | Low |
| Context Management | Summarizer + Memory MGP servers | Medium |
| Federation | Proxy MGP server | High |
| Audit Service | Dedicated Audit MGP server | Low |

### 19.1 Multi-Agent Coordination

Multiple agents collaborate — delegating tasks, sharing results, and coordinating
work — through a Coordinator MGP server that exposes coordination tools.

```
┌─────────────┐     ┌─────────────────────────┐     ┌─────────────┐
│   Agent A   │────>│   MGP Kernel            │────>│   Agent B   │
│             │     │                         │     │             │
│  tools/call │     │  ┌───────────────────┐  │     │  think()    │
│  delegate() │────>│  │  Coordinator      │  │────>│  store()    │
│             │     │  │  MGP Server       │  │     │  recall()   │
│  discover() │     │  │                   │  │     │             │
│             │     │  │  - delegate_task  │  │     │             │
└─────────────┘     │  │  - query_agents   │  │     └─────────────┘
                    │  │  - collect_results│  │
                    │  └───────────────────┘  │
                    └─────────────────────────┘
```

**Coordination Patterns:**

- **Fan-Out / Fan-In**: Distribute subtasks to multiple agents, collect all results
- **Chain**: Sequential delegation (translate → summarize → format)
- **Specialist Routing**: `query_agents(capabilities)` → delegate to best match

**MGP Primitives Used:** Tool calls (MCP base), Access Control (§5), Tool Discovery (§16),
Audit Trail (§6), Streaming (§12)

### 19.2 Context Management

Conversations accumulate context from chat history, file contents, and tool outputs.
A three-tier context management architecture prevents context window overflow.

```
┌──────────────────────────────────────────┐
│          Context Manager                  │
│  ┌─────────┐  ┌──────────┐  ┌─────────┐ │
│  │ Active  │  │ Summary  │  │ Evicted │ │
│  │ Context │  │ Buffer   │  │ Archive │ │
│  │ (60%)   │  │ (25%)    │  │ (ext.)  │ │
│  └─────────┘  └──────────┘  └─────────┘ │
└──────────────────────────────────────────┘
```

| Tier | Content | Eviction |
|------|---------|----------|
| **Active** | Current turn messages, active tool schemas | Never (current turn) |
| **Summary** | Compressed older messages | Re-summarize when full |
| **Archive** | Full history in memory server (KS22 etc.) | Never (persistent) |

**MGP Primitives Used:** Tool calls, Context Budget (§16.8), Tool Discovery (§16),
Lifecycle (§11)

### 19.3 Federation

Multiple MGP-compatible systems share servers and tools across network boundaries
through a Federation Proxy MGP server.

```
┌──────────────────┐           ┌──────────────────┐
│  Instance A      │  HTTPS    │  Instance B      │
│  ┌────────────┐  │◄────────►│  ┌────────────┐  │
│  │ Federation │  │           │  │ Federation │  │
│  │ Proxy      │  │           │  │ Proxy      │  │
│  └─────┬──────┘  │           │  └─────┬──────┘  │
│  ┌─────▼──────┐  │           │  ┌─────▼──────┐  │
│  │   Kernel   │  │           │  │   Kernel   │  │
│  └────────────┘  │           │  └────────────┘  │
└──────────────────┘           └──────────────────┘
```

Transparent federation via `mgp.discovery.register`: remote tools appear local.

**Security:** TLS + API key validation, local access control (§5) applies, audit events
(§6) include remote instance in `target` field.

**MGP Primitives Used:** Discovery (§15, §16), Security (§3, §4), Lifecycle (§11),
Streaming (§12), Error Handling (§14)

### 19.4 Audit Service

The protocol defines the audit event **format** (§6), but storage and querying are
implementation concerns handled by a dedicated Audit MGP server.

```
Kernel (MCP Client) ─── notifications/mgp.audit ──► Audit MGP Server
                                                      │
                                                      ├─ query_audit_log
                                                      ├─ get_audit_stats
                                                      └─ export_audit
```

**Retention Policies:** `keep_all`, `time_based` (N days), `size_based` (N MB), `tiered`

**MGP Primitives Used:** Audit Event Format (§6.3), Trace ID (§6.5), Tool Discovery (§16),
Access Control (§5)

### 19.5 Future Protocol Extensions

The following MAY be added to the MGP protocol in future versions if they cannot be
adequately expressed as application-layer patterns:

| Extension | Description | Priority |
|-----------|-------------|----------|
| `observability` | OpenTelemetry-compatible metrics and traces | Low |
| `versioning` | Tool schema versioning and migration | Low |

These will follow the same design principle: optional extensions negotiated during
`initialize`, with full backward compatibility to both MCP and earlier MGP versions.
