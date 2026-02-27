# MGP — Model General Protocol

**Version:** 0.2.0-draft
**Status:** Draft
**Authors:** ClotoCore Project
**Date:** 2026-02-27

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

| Extension | Description | Spec |
|-----------|-------------|------|
| `security` | Permission declarations and tool security metadata | §3, §4 |
| `access_control` | Agent-scoped tool access control protocol | §5 |
| `audit` | Structured audit trail notifications | §6 |
| `code_safety` | Code execution safety framework | §7 |
| `lifecycle` | Health checks, graceful shutdown, state management | §11 |
| `streaming` | Streaming tool responses, progress, cancellation | §12 |
| `bidirectional` | Server→Client notifications, event subscriptions, callbacks | §13 |
| `discovery` | Server advertisement, registration, deregistration | §15 |
| `tool_discovery` | Dynamic tool search, active tool request, session cache | §16 |

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
  │  mgp/permission/request       │
  │<──────────────────────────────│  (if interactive)
  │                               │
  │  mgp/permission/response      │
  │──────────────────────────────>│
  │                               │
  │  initialize result            │
  │<──────────────────────────────│  (connection proceeds or is rejected)
```

### 3.5 Permission Request Method

**Method:** `mgp/permission/request`

Direction: Client → Server (the client asks the server to wait for approval)

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "mgp/permission/request",
  "params": {
    "request_id": "perm-001",
    "permissions": ["shell.execute", "filesystem.read"],
    "policy": "interactive",
    "message": "Waiting for operator approval"
  }
}
```

### 3.6 Permission Response Method

**Method:** `mgp/permission/response`

Direction: Server → Client (after operator decision)

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
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

## 5. Access Control Protocol

### 5.1 Overview

MGP defines protocol-level methods for managing agent-to-tool access control. This replaces
implementation-specific access control (database tables, config files) with a standardized
protocol that any MGP-compatible system can implement.

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

### 5.4 Methods

#### mgp/access/query

Query the current access state for an agent-tool combination.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "mgp/access/query",
  "params": {
    "agent_id": "agent.cloto_default",
    "server_id": "mind.cerebras",
    "tool_name": "think"
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "permission": "allow",
    "source": "server_grant",
    "granted_by": "admin",
    "granted_at": "2026-02-27T12:00:00Z",
    "expires_at": null
  }
}
```

#### mgp/access/grant

Grant access to an agent.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "mgp/access/grant",
  "params": {
    "entry_type": "server_grant",
    "agent_id": "agent.cloto_default",
    "server_id": "mind.cerebras",
    "tool_name": null,
    "permission": "allow",
    "granted_by": "admin",
    "justification": "Required for agent reasoning",
    "expires_at": null
  }
}
```

#### mgp/access/revoke

Revoke an existing access grant.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "mgp/access/revoke",
  "params": {
    "agent_id": "agent.cloto_default",
    "server_id": "mind.cerebras",
    "entry_type": "server_grant"
  }
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
server (see `MGP_PATTERNS.md`).

This separation ensures that the protocol defines **what** audit events look like, while
**how** they are stored and processed remains an implementation concern.

### 6.2 Protocol Scope vs Server Scope

| Concern | Scope | Defined In |
|---------|-------|------------|
| Audit event format (structure, fields) | **Protocol** | This section |
| Standard event types | **Protocol** | This section |
| Trace ID propagation | **Protocol** | This section |
| Audit event storage and persistence | **Server** | `MGP_PATTERNS.md` |
| Audit event querying and search | **Server** | `MGP_PATTERNS.md` |
| Audit analytics and alerting | **Server** | `MGP_PATTERNS.md` |

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

The kernel MAY forward these notifications to a connected Audit MGP server for persistence.
If no Audit server is connected, the kernel SHOULD log events locally.

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

## 11. Lifecycle Management

### 11.1 Overview

MGP defines standard lifecycle methods for managing server health, state transitions, and
restart behavior. MCP provides no lifecycle primitives — servers are either running or not,
with no protocol-level health monitoring or graceful shutdown.

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

### 11.3 Health Check Protocol

#### mgp/health/ping

A lightweight liveness check. Servers MUST respond within 5 seconds.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 10,
  "method": "mgp/health/ping",
  "params": {
    "timestamp": "2026-02-27T12:00:00.000Z"
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 10,
  "result": {
    "status": "healthy",
    "timestamp": "2026-02-27T12:00:00.005Z",
    "uptime_secs": 3600,
    "server_id": "mind.cerebras"
  }
}
```

**Status Values:**

| Status | Meaning |
|--------|---------|
| `healthy` | Server is fully operational |
| `degraded` | Server is running but some capabilities are limited |
| `unhealthy` | Server is experiencing errors but still responding |

#### mgp/health/status

Detailed readiness check including resource usage and capability status.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 11,
  "method": "mgp/health/status",
  "params": {}
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 11,
  "result": {
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
}
```

The `resources` and `checks` objects are server-defined. Clients SHOULD NOT depend on specific
keys being present.

### 11.4 Graceful Shutdown

#### mgp/lifecycle/shutdown

Request a server to shut down gracefully. The server finishes in-flight requests, transitions
to `draining` state, and then closes the transport.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 12,
  "method": "mgp/lifecycle/shutdown",
  "params": {
    "reason": "operator_request",
    "timeout_ms": 30000
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 12,
  "result": {
    "accepted": true,
    "pending_requests": 2,
    "estimated_drain_ms": 5000
  }
}
```

**Shutdown Reasons:**

| Reason | Description |
|--------|-------------|
| `operator_request` | Human-initiated shutdown |
| `configuration_change` | Configuration updated, restart needed |
| `resource_limit` | Server exceeded resource limits |
| `idle_timeout` | Server has been idle beyond threshold |
| `kernel_shutdown` | Entire kernel is shutting down |

#### notifications/mgp.lifecycle

State transition notification emitted by the server.

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

### 11.5 Restart Policies

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

### 13.2 Event Subscription

Clients can subscribe to server-defined event channels:

#### mgp/events/subscribe

```json
{
  "jsonrpc": "2.0",
  "id": 30,
  "method": "mgp/events/subscribe",
  "params": {
    "channels": ["model.token_usage", "system.error"],
    "filter": {
      "min_severity": "warning"
    }
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 30,
  "result": {
    "subscribed": ["model.token_usage", "system.error"],
    "unsupported": [],
    "subscription_id": "sub-001"
  }
}
```

#### mgp/events/unsubscribe

```json
{
  "jsonrpc": "2.0",
  "id": 31,
  "method": "mgp/events/unsubscribe",
  "params": {
    "subscription_id": "sub-001"
  }
}
```

### 13.3 Server Push Notifications

After subscription, the server emits events as notifications:

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

### 13.5 Standard Event Channels

| Channel Pattern | Description |
|----------------|-------------|
| `model.*` | LLM-related events (token usage, rate limits, errors) |
| `system.*` | System-level events (resource usage, errors) |
| `tool.*` | Tool execution events (started, completed, failed) |
| `security.*` | Security events (access denied, validation failed) |

Servers define their own channels within these patterns. Clients SHOULD NOT assume specific
channels exist — use `mgp/events/subscribe` to discover available channels.

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

## 15. Discovery

### 15.1 Overview

MGP defines a standard mechanism for server advertisement and discovery. This enables
clients to automatically find available servers without manual configuration.

### 15.2 Server Advertisement

MGP servers MAY advertise themselves via a well-known configuration file or registry endpoint.

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

#### Registry Endpoint Discovery

For distributed environments, clients can query a registry:

**Method:** `mgp/discovery/list`

```json
{
  "jsonrpc": "2.0",
  "id": 40,
  "method": "mgp/discovery/list",
  "params": {
    "filter": {
      "extensions": ["streaming"],
      "permissions": ["network.outbound"],
      "status": "connected"
    }
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 40,
  "result": {
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
}
```

### 15.3 Capability Advertisement

Connected servers advertise their capabilities via the `initialize` response (§2). For
pre-connection discovery, the `mgp.toml` configuration or registry provides the same
information without establishing a transport connection.

### 15.4 Dynamic Registration

Servers created at runtime (e.g., by agents) register themselves with the kernel:

**Method:** `mgp/discovery/register`

```json
{
  "jsonrpc": "2.0",
  "id": 41,
  "method": "mgp/discovery/register",
  "params": {
    "id": "dynamic.custom_tool",
    "command": "python",
    "args": ["scripts/mcp_custom.py"],
    "transport": "stdio",
    "mgp": {
      "extensions": ["security"],
      "permissions_required": ["network.outbound"],
      "trust_level": "sandboxed"
    },
    "created_by": "agent.cloto_default",
    "justification": "Agent needs custom data processing tool"
  }
}
```

Dynamic registrations with `trust_level: "sandboxed"` are subject to stricter validation
(code safety framework, limited permissions) than `standard` or `trusted` servers.

### 15.5 Deregistration

**Method:** `mgp/discovery/deregister`

```json
{
  "jsonrpc": "2.0",
  "id": 42,
  "method": "mgp/discovery/deregister",
  "params": {
    "id": "dynamic.custom_tool",
    "reason": "no_longer_needed"
  }
}
```

---

# Part III: Intelligence Layer

---

## 16. Dynamic Tool Discovery & Active Tool Request

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
LLM context. Instead, the LLM receives a single meta-tool (`mgp/tools/discover`) and
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
  "embedding": [0.012, -0.034, ...]
}
```

The index supports three search strategies:

| Strategy | Method | Best For |
|----------|--------|----------|
| Keyword | Exact and fuzzy keyword matching | Precise tool names |
| Semantic | Embedding vector similarity | Natural language intent |
| Category | Hierarchical category filtering | Browsing available capabilities |

### 16.5 Mode A: Dynamic Tool Discovery

The LLM searches for tools based on a natural language description of what it needs.
This is the **user-intent-driven** mode — the LLM translates the user's request into a
tool search.

#### mgp/tools/discover

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 50,
  "method": "mgp/tools/discover",
  "params": {
    "query": "read file contents from disk",
    "strategy": "semantic",
    "max_results": 5,
    "filter": {
      "categories": ["filesystem"],
      "risk_level_max": "moderate",
      "status": "connected"
    }
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 50,
  "result": {
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
    "search_strategy": "semantic",
    "query_time_ms": 12
  }
}
```

The response includes **full tool schemas** for the top results, allowing the LLM to
immediately call any discovered tool without a second round trip.

#### Flow Diagram

```
User: "このファイルの中身を見せて"
  │
  ▼
LLM context: [mgp/tools/discover meta-tool] + [user message]
  │
  ▼ LLM decides it needs file-reading capability
  │
  ▼ tools/call → mgp/tools/discover({ query: "read file contents" })
  │
  ▼ Kernel searches tool index (semantic)
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

#### mgp/tools/request

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 51,
  "method": "mgp/tools/request",
  "params": {
    "reason": "capability_gap",
    "context": "Processing CSV data but need statistical analysis functions",
    "requirements": {
      "capabilities": ["data_analysis", "statistics"],
      "input_types": ["csv", "tabular_data"],
      "output_types": ["numeric", "chart"],
      "preferred_risk_level": "safe"
    },
    "task_trace_id": "550e8400-e29b-41d4-a716-446655440000"
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 51,
  "result": {
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
| `creating` | No existing tools match — agent-generated tool creation initiated (§7) |

#### The `creating` Status — Autonomous Tool Generation

When `status: "creating"` is returned, the kernel has determined that no existing tool
satisfies the requirement. If the agent has `code_execution` permission and the system
is in `auto_approve` policy, the kernel MAY:

1. Instruct the agent to generate tool code via the Code Safety Framework (§7)
2. Validate and register the new tool via Dynamic Registration (§15.4)
3. Return the newly created tool in a follow-up response

This closes the loop: **discover → request → create → use** — fully autonomous tool
lifecycle without human intervention.

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
  │    ▼ tools/call → mgp/tools/request({
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

#### mgp/tools/session

Query the current session's loaded tools:

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 52,
  "method": "mgp/tools/session",
  "params": {}
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 52,
  "result": {
    "pinned": ["think", "store", "recall"],
    "cached": ["read_file", "analyze_csv"],
    "total_tokens": 2100,
    "max_tokens": 8000
  }
}
```

#### mgp/tools/session/evict

Remove tools from the session cache to free context space:

```json
{
  "jsonrpc": "2.0",
  "id": 53,
  "method": "mgp/tools/session/evict",
  "params": {
    "tools": ["analyze_csv"],
    "reason": "no_longer_needed"
  }
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

## 8. Implementation Notes

### 8.1 For Server Implementors

1. **Minimal MGP support**: Include `mgp` in your `initialize` response capabilities.
   Even supporting just the `security` extension (tool security metadata) adds significant
   value for clients.

2. **Graceful degradation**: If the client does not send `mgp` in its `initialize` request,
   behave as a standard MCP server. Do not require MGP support.

3. **Permission declarations**: List all permissions your server needs in
   `permissions_required`. Clients may deny startup if permissions are not granted.

### 8.2 For Client Implementors

1. **Discovery**: Check for `mgp` in the server's `initialize` response. If absent,
   treat as standard MCP.

2. **Fallback validators**: Even without MGP server support, clients SHOULD apply kernel-side
   validators (sandbox, code_safety) based on tool names and server configuration.

3. **Access control**: Implement access control at the client/kernel level. The protocol
   methods (`mgp/access/*`) are for inter-system communication; the enforcement point is
   always the client.

### 8.3 Relationship to ClotoCore

ClotoCore is the reference implementation of MGP. The following ClotoCore components
map to MGP specifications:

| MGP Spec | ClotoCore Component | File |
|----------|-------------------|------|
| §2 Capability Negotiation | `cloto/handshake` | `managers/mcp.rs` |
| §3 Permission Declarations | Permission Gate (D) | `managers/mcp.rs` |
| §4 Tool Security Metadata | `tool_validators` config | `managers/mcp_protocol.rs` |
| §5 Access Control | `mcp_access_control` table | `db.rs`, `handlers.rs` |
| §6 Audit Trail | `audit_logs` table | `handlers.rs` |
| §7 Code Safety | `validate_mcp_code()` | `managers/mcp.rs` |
| §11 Lifecycle | `ServerStatus`, `auto_restart`, `restart_server()` | `managers/mcp.rs` |
| §12 Streaming | — (not yet implemented) | — |
| §13 Bidirectional | `notifications/cloto.event`, SSE event bus | `handlers.rs`, `lib.rs` |
| §14 Error Handling | `JsonRpcError` | `managers/mcp_protocol.rs` |
| §15 Discovery | `mcp.toml`, `add_dynamic_server()` | `managers/mcp.rs` |
| §16 Tool Discovery | — (not yet implemented) | — |

---

## 9. Version History

| Version | Date | Changes |
|---------|------|---------|
| 0.1.0-draft | 2026-02-27 | Initial draft — Security layer (§2-7) |
| 0.2.0-draft | 2026-02-27 | Communication & Lifecycle layer (§11-15) |
| 0.3.0-draft | 2026-02-27 | Intelligence layer — Dynamic Tool Discovery & Active Tool Request (§16) |

---

## 10. Application Patterns & Future Extensions

### 10.1 Application Patterns

The following capabilities are intentionally **not part of the MGP protocol specification**.
They can be fully implemented as MGP servers using the existing protocol primitives
(§2-7, §11-15, §16). See `docs/MGP_PATTERNS.md` for reference architectures.

| Pattern | Implementation | MGP Primitives Used |
|---------|---------------|---------------------|
| Multi-Agent Coordination | Coordination MGP server | Tool calls, Discovery (§16), Access Control (§5) |
| Context Management | Summarization + Memory MGP servers | Tool calls, Context Budget (§16.8) |
| Federation | Proxy MGP server | Discovery (§15, §16), Lifecycle (§11) |

### 10.2 Future Protocol Extensions

The following MAY be added to the MGP protocol in future versions if they cannot be
adequately expressed as application-layer patterns:

| Extension | Description | Priority |
|-----------|-------------|----------|
| `observability` | OpenTelemetry-compatible metrics and traces | Low |
| `versioning` | Tool schema versioning and migration | Low |

These will follow the same design principle: optional extensions negotiated during
`initialize`, with full backward compatibility to both MCP and earlier MGP versions.
