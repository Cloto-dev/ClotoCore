# MGP — Security & Foundation

> Part of the [MGP Specification](MGP_SPEC.md) (v0.6.0-draft, 2026-03-06)
> This document covers §2-§7. For overview and architecture, see [MGP_SPEC.md](MGP_SPEC.md).

**Section Map:** §1 [MGP_SPEC.md](MGP_SPEC.md) · §2-§7 [MGP_SECURITY.md](MGP_SECURITY.md) · §11-§14 [MGP_COMMUNICATION.md](MGP_COMMUNICATION.md) · §15-§16 [MGP_DISCOVERY.md](MGP_DISCOVERY.md) · §17-§20 [MGP_GUIDE.md](MGP_GUIDE.md)

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
        "version": "0.6.0",
        "extensions": ["permissions", "tool_security", "access_control", "audit"]
      }
    },
    "clientInfo": {
      "name": "CLOTO-KERNEL",
      "version": "0.6.0"
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
| `permissions` | 1+3 (Metadata + Method) | Permission declarations and approval flow | §3 |
| `tool_security` | 1 (Metadata) | Tool-level security metadata (`security` field in `tools/list`) | §4 |
| `access_control` | 4 (Kernel Tool) | Agent-scoped tool access control | §5 |
| `audit` | 2 (Notification) | Structured audit trail notifications | §6 |
| `code_safety` | 1 (Metadata) | Code execution safety framework | §7 |
| `lifecycle` | 2+4 (Notification + Kernel Tool) | State transitions, health checks, shutdown | §11 |
| `streaming` | 2+3 (Notification + Method) | Stream chunks, cancellation, flow control | §12 |
| `progress` | 2 (Notification) | Progress reporting for long-running operations | §12.6 |
| `callbacks` | 2+3 (Notification + Method) | Server-to-kernel callback requests (including `llm_completion`) | §13.3, §13.4 |
| `events` | 2+4 (Notification + Kernel Tool) | Event bus notifications, subscribe/unsubscribe | §13.1, §13.2 |
| `discovery` | 4 (Kernel Tool) | Server registration, deregistration | §15 |
| `tool_discovery` | 4 (Kernel Tool) | Dynamic tool search, active tool request | §16 |
| `error_handling` | 1 (Metadata) | Structured error categories and recovery hints | §14 |

Each extension is independently negotiable. Negotiating a Layer 4 extension means the
kernel exposes the corresponding standard MCP tools (see §1.6). Layer 1-3 extensions
activate protocol-level behavior.

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
        "version": "0.6.0",
        "extensions": ["permissions", "tool_security", "audit"],
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
| `trust_level` | string | No | `core`, `standard`, `experimental`, or `untrusted` (see MGP_ISOLATION_DESIGN.md §3.1) |

> **Note:** The `trust_level` value in the server's handshake response is
> **informational only**. The kernel determines the effective trust level from
> `mcp.toml` configuration and Magic Seal verification (see MGP_ISOLATION_DESIGN.md
> §10 Security Invariant 3). If the server-declared value exceeds the
> kernel-determined level, the kernel silently downgrades it.

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

**Simple format** (no scope):

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

**Scoped format** (with resource constraints):

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "mgp/permission/grant",
  "params": {
    "request_id": "perm-001",
    "grants": {
      "shell.execute": {
        "decision": "approved",
        "scope": {
          "commands": ["git", "npm", "cargo"],
          "deny_commands": ["rm", "dd", "mkfs"],
          "allow_sudo": false
        }
      },
      "filesystem.read": {
        "decision": "approved",
        "scope": {
          "paths": ["/home/user/projects/**", "/tmp/**"],
          "deny_paths": ["/home/user/.ssh/**", "/etc/shadow"]
        }
      }
    },
    "approved_by": "admin",
    "expires_at": "2026-03-01T00:00:00Z"
  }
}
```

Both formats are valid. When the grant value is a string (e.g., `"approved"`), it is
equivalent to `{ "decision": "approved" }` with no scope constraints.

**Grant Decision Values:**

| Value | Meaning |
|-------|---------|
| `approved` | Permission granted (with optional scope) |
| `denied` | Permission denied (server should degrade gracefully) |
| `deferred` | Decision deferred (server should wait or retry) |

### 3.7 Permission Scopes

When a permission is granted with a `scope` object, the server is restricted to
operating within the specified boundaries. Scope constraints are enforced by the
kernel and, where available, by OS-level isolation (see MGP_ISOLATION_DESIGN.md).

#### 3.7.1 Standard Scope Fields

| Permission | Scope Field | Type | Description |
|------------|-------------|------|-------------|
| `filesystem.read`, `filesystem.write` | `paths` | string[] (glob) | Allowed path patterns |
| | `deny_paths` | string[] (glob) | Denied path patterns (takes precedence over `paths`) |
| `shell.execute` | `commands` | string[] | Allowed command names (basename only) |
| | `deny_commands` | string[] | Denied command names |
| | `allow_sudo` | boolean | Whether `sudo` prefix is permitted (default: `false`) |
| `network.outbound`, `network.listen` | `hosts` | string[] (glob) | Allowed hostnames |
| | `deny_hosts` | string[] | Denied hostnames |
| | `ports` | number[] | Allowed ports (empty = all) |
| `code_execution` | `languages` | string[] | Allowed languages (e.g., `["python", "javascript"]`) |
| | `max_execution_time_ms` | number | Maximum execution time per invocation |

#### 3.7.2 Scope Resolution Rules

1. `deny_*` fields always take precedence over allow fields (deny-first)
2. If a `scope` is present but the relevant field is absent, no constraint applies
   for that dimension (e.g., `{ "paths": ["/data/**"] }` without `deny_paths`
   means deny_paths is empty)
3. Glob patterns follow POSIX glob syntax: `*` matches any single path component,
   `**` matches zero or more path components
4. The kernel propagates scope constraints to the OS isolation layer where
   applicable (see MGP_ISOLATION_DESIGN.md §3.5)

#### 3.7.3 Custom Permission Scopes

Custom permissions (reverse-domain notation) MAY define their own scope fields.
The kernel treats unrecognized scope fields as opaque metadata and passes them to
the enforcement layer without validation.

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

| Field | Type | Source | Description |
|-------|------|--------|-------------|
| `risk_level` | string | Server (informational) | Server's self-declared risk: `safe`, `moderate`, or `dangerous` |
| `effective_risk_level` | string | Kernel (authoritative) | Kernel-derived risk level — see §4.6. Injected by kernel into `tools/list` relay |
| `permissions_required` | string[] | Server | Permissions needed to call this tool |
| `side_effects` | string[] | Server (informational) | Categories of side effects: `filesystem`, `network`, `process`, `database`, `notification` |
| `validator` | string | Kernel (from mcp.toml) | Kernel-side validator to apply: `sandbox`, `readonly`, `none` |
| `reversible` | boolean | Server (informational) | Whether the tool's effects can be undone |
| `confirmation_required` | boolean | Merged | Server declares; kernel MAY override to `true` for `dangerous` tools |

Fields marked **informational** are self-declared by the server and MUST NOT be used
for security decisions. Fields marked **authoritative** are determined by the kernel
and can be trusted. The `Source` column indicates the trust boundary.

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

### 4.6 Effective Risk Level Derivation

The kernel computes `effective_risk_level` for each tool using three inputs:

```
effective_risk_level = max(
    derive_from_trust_level(server.trust_level),
    derive_from_validator(tool.validator),
    derive_from_permissions(tool.permissions_required)
)
```

**Trust level mapping:**

| `trust_level` | Derived risk |
|----------------|-------------|
| `core` | `safe` |
| `standard` | `moderate` |
| `experimental` | `dangerous` |
| `untrusted` | `dangerous` |

**Validator mapping:**

| `validator` | Derived risk |
|-------------|-------------|
| `readonly` | `safe` |
| `sandbox` | `moderate` |
| `network_restricted` | `moderate` |
| `code_safety` | `moderate` |
| `none` | `dangerous` |

**Permission mapping:**

| Condition | Derived risk |
|-----------|-------------|
| `permissions_required` includes `shell.execute` or `code_execution` | `dangerous` |
| `permissions_required` includes `filesystem.write` | `moderate` |
| Otherwise | `safe` |

The `max()` function uses the ordering `safe < moderate < dangerous`.

When the kernel relays `tools/list` to the LLM or client, it injects
`effective_risk_level` into each tool's `security` object. If
`effective_risk_level` differs from the server-declared `risk_level`, the kernel
SHOULD generate an audit event with `event_type: "RISK_LEVEL_OVERRIDE"`.

LLM agents and UI components MUST use `effective_risk_level` (not `risk_level`)
for security-relevant decisions such as confirmation prompts.

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

### 5.6 Delegated Execution

When a server executes a tool call on behalf of another agent (e.g., a coordinator
server delegating tasks in a multi-agent system), the tool call SHOULD include a
`delegation` object in the `_mgp` field:

```json
{
  "method": "tools/call",
  "params": {
    "name": "read_file",
    "arguments": { "path": "/data/report.txt" },
    "_mgp": {
      "delegation": {
        "original_actor": "agent-A",
        "delegated_via": "coordinator-server",
        "delegation_id": "del-001"
      }
    }
  }
}
```

#### 5.6.1 Permission Evaluation

When a `delegation` field is present, the kernel evaluates access control using the
**intersection** of the original actor's and the delegating server's permissions:

```
effective_permissions = intersect(
    permissions(original_actor),
    permissions(delegated_via)
)
```

This ensures that:
- The delegating server cannot escalate the original actor's privileges
- The original actor cannot gain access to tools beyond the delegating server's scope
- Both parties must independently hold the required permission for the call to succeed

#### 5.6.2 Delegation Chain Limits

Delegation chains MUST NOT exceed a depth of **3** (original actor → delegate 1 →
delegate 2). The kernel MUST reject tool calls with deeper chains with error code
`1000 PERMISSION_DENIED` and detail `"delegation_depth_exceeded"`.

The `delegation` field for chained delegations:

```json
{
  "_mgp": {
    "delegation": {
      "original_actor": "agent-A",
      "delegated_via": "sub-coordinator",
      "chain": ["agent-A", "coordinator-server", "sub-coordinator"],
      "delegation_id": "del-002"
    }
  }
}
```

The effective permissions are the intersection of **all** actors in the chain.

#### 5.6.3 Kernel Verification

The kernel MUST verify delegation claims:
- `original_actor` must be a known, active agent
- `delegated_via` must be the server that sent the tool call
- If either check fails, the call is rejected with `1000 PERMISSION_DENIED`

#### 5.6.4 Audit Integration

Delegated tool calls generate audit events with `actor.type: "delegated"`:

```json
{
  "actor": {
    "type": "delegated",
    "original_actor": "agent-A",
    "delegated_via": "coordinator-server",
    "delegation_id": "del-001"
  }
}
```

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

The kernel MUST persist all audit events to its local audit store **before**
forwarding them as notifications. The local store is the primary record; notifications
are a secondary delivery channel for real-time consumption.

The kernel SHOULD forward `notifications/mgp.audit` to all connected servers that
declared `audit` in their negotiated extensions (§2). Forwarded audit notifications
include `_mgp.seq` for gap detection by the receiving server:

```json
{
  "method": "notifications/mgp.audit",
  "params": {
    "_mgp": { "seq": 43 },
    "event_type": "TOOL_EXECUTED",
    "timestamp": "2026-02-27T12:00:00.000Z",
    "..."
  }
}
```

The `seq` value is a per-server monotonically increasing integer managed by the
kernel. Each server receives its own independent sequence. Servers detect gaps by
tracking the last received `seq` and checking for non-consecutive values.

#### mgp.audit.replay

**Tool Name:** `mgp.audit.replay`
**Category:** Kernel Tool (Layer 4)

Allows audit-subscribed servers to retrieve missed events after gap detection or
reconnection.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "since_seq": { "type": "integer", "description": "Resume from this sequence number (exclusive)" },
    "since_timestamp": { "type": "string", "format": "date-time", "description": "Alternative: resume from this timestamp" },
    "limit": { "type": "integer", "default": 100, "description": "Maximum events to return" }
  }
}
```

**Output:**
```json
{
  "events": [
    { "seq": 44, "event_type": "TOOL_EXECUTED", "..." },
    { "seq": 45, "event_type": "PERMISSION_GRANTED", "..." }
  ],
  "has_more": true,
  "next_seq": 46
}
```

The server calls `mgp.audit.replay` with `since_seq` set to the last successfully
received sequence number. The kernel returns events from its local store. If
`has_more` is true, the server should call again with `since_seq: next_seq - 1`.

Either `since_seq` or `since_timestamp` must be provided. If both are present,
`since_seq` takes precedence.

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

> **§8–§10 Reserved.** These section numbers are reserved for future Part I extensions
> (e.g., Isolation Profiles, Trust Level Enforcement, Magic Seal Verification).
> See `docs/MGP_ISOLATION_DESIGN.md` for current design work in these areas.

---

# Part II: Communication & Lifecycle Layer

---

