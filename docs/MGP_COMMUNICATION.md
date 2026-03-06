# MGP — Communication & Lifecycle

> Part of the [MGP Specification](MGP_SPEC.md) (v0.6.0-draft, 2026-03-06)
> This document covers §11-§14. For overview and architecture, see [MGP_SPEC.md](MGP_SPEC.md).

**Section Map:** §1 [MGP_SPEC.md](MGP_SPEC.md) · §2-§7 [MGP_SECURITY.md](MGP_SECURITY.md) · §11-§14 [MGP_COMMUNICATION.md](MGP_COMMUNICATION.md) · §15-§16 [MGP_DISCOVERY.md](MGP_DISCOVERY.md) · §17-§20 [MGP_GUIDE.md](MGP_GUIDE.md)

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

| State | Description | Implemented |
|-------|-------------|-------------|
| `registered` | Server configuration loaded but not yet started | No |
| `connecting` | Transport initializing, handshake in progress | No |
| `connected` | Operational — accepting tool calls | Yes |
| `draining` | Graceful shutdown initiated — finishing in-flight requests, rejecting new ones | No |
| `disconnected` | Transport closed, server stopped | Yes |
| `error` | Connection failed or runtime error | Yes |
| `restarting` | Server is being stopped and restarted | No |

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
    "version": "0.6.0",
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

### 12.8 Flow Control

Streaming has no built-in backpressure in JSON-RPC. MGP provides a **rate hint**
mechanism that allows the client to request throttling without aborting the stream.

**Notification:** `notifications/mgp.stream.pace`

Direction: Client → Server

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/mgp.stream.pace",
  "params": {
    "request_id": 20,
    "max_chunks_per_second": 10,
    "reason": "client_busy"
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `request_id` | number | Yes | The original tool call request ID |
| `max_chunks_per_second` | number | Yes | Recommended maximum chunk rate. `0` = pause |
| `reason` | string | No | `client_busy`, `rendering`, `user_paused` |

**Server behavior:**

- Servers SHOULD respect rate hints by throttling chunk emission
- `max_chunks_per_second: 0` requests a pause. The server SHOULD stop emitting
  chunks until a subsequent `pace` with a positive value or a `cancel` is received
- Servers that ignore pace hints are not in protocol violation, but the kernel
  MAY buffer and throttle chunks on behalf of the client
- A `pace` notification does not affect the final response (§12.5), which is
  always delivered regardless of pacing state

### 12.9 Gap Detection

Stream chunks include an `index` field (§12.4) that enables gap detection. If the
client detects missing indices, it MAY request retransmission:

**Notification:** `notifications/mgp.stream.gap`

Direction: Client → Server

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/mgp.stream.gap",
  "params": {
    "request_id": 20,
    "missing_indices": [3, 4, 7]
  }
}
```

**Server behavior:**

- If the missing chunks are still in the server's buffer, the server SHOULD
  retransmit them as standard `notifications/mgp.stream.chunk` messages
- If the missing chunks have been discarded, the server SHOULD emit a chunk with
  `"gap_unrecoverable": true` and continue from the current position
- The final response (§12.5) always contains the **complete** result regardless
  of gaps, ensuring data integrity even when individual chunks are lost

**Client behavior:**

- Gap detection is OPTIONAL. Clients that do not track indices simply rely on
  the final response for the complete result
- Clients SHOULD wait a reasonable interval (e.g., 2× the average inter-chunk
  delay) before declaring a gap, to account for out-of-order delivery

---

## 13. Bidirectional Communication

### 13.1 Overview

Standard MCP is primarily unidirectional: the client calls tools on the server. MGP adds
standardized patterns for server-initiated communication — event subscriptions, push
notifications, and callback requests.

- **Layer 2 (Protocol Notifications):** `notifications/mgp.callback.request` — server requests
  information from the kernel during tool execution; `notifications/mgp.event` — server
  pushes subscribed events (with `_mgp.seq` for gap detection)
- **Layer 3 (Protocol Methods):** `mgp/callback/respond` — kernel responds to callback requests
- **Layer 4 (Kernel Tools):** `mgp.events.subscribe`, `mgp.events.unsubscribe` — event
  subscription management; `mgp.events.replay` — catch-up replay for missed events

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

After subscription, the server emits events as **Layer 2 protocol notifications**.

Each event notification MUST include a `_mgp.seq` field — a monotonically increasing
integer (per subscription) that enables gap detection and replay. Subscribers detect
gaps by tracking the last received sequence number; if `received_seq > last_seq + 1`,
one or more events were lost and the subscriber SHOULD request replay via
`mgp.events.replay` (§13.6).

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/mgp.event",
  "params": {
    "subscription_id": "sub-001",
    "channel": "model.token_usage",
    "timestamp": "2026-02-27T12:05:00.000Z",
    "_mgp.seq": 42,
    "data": {
      "tokens_used": 1500,
      "tokens_remaining": 8500,
      "model": "gpt-oss-120b"
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

#### Callback Delivery Reliability

`notifications/mgp.callback.request` is a JSON-RPC 2.0 notification and therefore has
no built-in delivery guarantee. Since callback requests often gate human-in-the-loop
decisions, loss can stall tool execution indefinitely.

**Server-side retry:**

Servers SHOULD implement retry logic for callback requests:

| Parameter | Default | Description |
|-----------|---------|-------------|
| `retry_count` | 3 | Maximum number of retries after initial attempt |
| `retry_interval_ms` | 5000 | Interval between retries |
| `retry_backoff` | `none` | Backoff strategy: `none`, `linear`, `exponential` |

Each retry MUST reuse the same `callback_id` as the original request. The server MAY
include a `_mgp.attempt` field (integer, starting at 1) to indicate the attempt number:

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
    "timeout_ms": 60000,
    "_mgp.attempt": 2
  }
}
```

**Kernel-side deduplication:**

The kernel MUST deduplicate callback requests by `callback_id`. If the kernel receives
a `notifications/mgp.callback.request` with a `callback_id` for which a response has
already been sent via `mgp/callback/respond`, the kernel MUST:

1. Ignore the duplicate notification (do not re-prompt the user)
2. Re-send the previously recorded response via `mgp/callback/respond`

This ensures idempotent delivery: servers can safely retry without causing duplicate
prompts or conflicting responses.

**Exhaustion behavior:**

If all retries are exhausted without receiving a response, the server SHOULD:

1. Treat the callback as timed out
2. Emit an audit event: `{ "event_type": "CALLBACK_TIMEOUT", "callback_id": "cb-001" }`
3. Either fail the parent tool call with error code `3003` (TIMEOUT) or proceed with a
   safe default action, depending on the callback type

### 13.5 Standard Event Channels

| Channel Pattern | Description |
|----------------|-------------|
| `model.*` | LLM-related events (token usage, rate limits, errors) |
| `system.*` | System-level events (resource usage, errors) |
| `tool.*` | Tool execution events (started, completed, failed) |
| `security.*` | Security events (access denied, validation failed) |

Servers define their own channels within these patterns. Clients SHOULD NOT assume specific
channels exist — use the `mgp.events.subscribe` kernel tool to discover available channels.

### 13.6 Event Replay — Kernel Tool

#### mgp.events.replay

**Tool Name:** `mgp.events.replay`
**Category:** Kernel Tool (Layer 4)

Replay missed event notifications for a subscription. Subscribers invoke this tool after
detecting a sequence gap (see §13.3) or after reconnection to catch up on events emitted
while disconnected.

The kernel MUST buffer event notifications per subscription. The buffer depth is
implementation-defined but MUST retain at least the most recent 1000 events per
subscription. Events beyond the buffer depth are permanently lost; the kernel indicates
this via the `truncated` field in the response.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "subscription_id": {
      "type": "string",
      "description": "The subscription to replay events for"
    },
    "after_seq": {
      "type": "integer",
      "description": "Replay events with _mgp.seq strictly greater than this value. Use the last successfully received sequence number."
    },
    "limit": {
      "type": "integer",
      "description": "Maximum number of events to return (default: 100, max: 1000)"
    }
  },
  "required": ["subscription_id", "after_seq"]
}
```

**Output:**
```json
{
  "subscription_id": "sub-001",
  "events": [
    {
      "channel": "model.token_usage",
      "timestamp": "2026-02-27T12:05:01.000Z",
      "_mgp.seq": 43,
      "data": { "tokens_used": 1600, "tokens_remaining": 8400, "model": "gpt-oss-120b" }
    },
    {
      "channel": "model.token_usage",
      "timestamp": "2026-02-27T12:05:02.000Z",
      "_mgp.seq": 44,
      "data": { "tokens_used": 1700, "tokens_remaining": 8300, "model": "gpt-oss-120b" }
    }
  ],
  "has_more": false,
  "truncated": false
}
```

**Response fields:**

| Field | Type | Description |
|-------|------|-------------|
| `events` | array | Replayed events in sequence order |
| `has_more` | boolean | `true` if more events exist beyond `limit` — caller should paginate |
| `truncated` | boolean | `true` if events before the requested range were evicted from buffer |

**Error codes:**

| Code | Condition |
|------|-----------|
| 4001 | `subscription_id` does not exist or has been cancelled |
| 1000 | Caller does not own the subscription |

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
| 4003 | `TOOL_NAME_CONFLICT` | Server attempted to register a tool with reserved `mgp.*` prefix |
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

