# MGP — Implementation, History & Patterns

> Part of the [MGP Specification](MGP_SPEC.md) (v0.6.0-draft, 2026-03-06)
> This document covers §17-§20. For overview and architecture, see [MGP_SPEC.md](MGP_SPEC.md).

**Section Map:** §1 [MGP_SPEC.md](MGP_SPEC.md) · §2-§7 [MGP_SECURITY.md](MGP_SECURITY.md) · §11-§14 [MGP_COMMUNICATION.md](MGP_COMMUNICATION.md) · §15-§16 [MGP_DISCOVERY.md](MGP_DISCOVERY.md) · §17-§20 [MGP_GUIDE.md](MGP_GUIDE.md)

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

All paths relative to `crates/core/src/`.

| MGP Spec | Layer | ClotoCore Component | File |
|----------|-------|-------------------|------|
| §2 Capability Negotiation | 1 | `cloto/handshake`, `MgpNegotiated` | `managers/mcp.rs`, `managers/mcp_mgp.rs` |
| §3 Permission Declarations | 3 | Permission Gate (D) | `managers/mcp.rs` |
| §4 Tool Security Metadata | 1 | `ToolSecurityMetadata`, `effective_risk_level` | `managers/mcp_mgp.rs`, `managers/mcp_tool_validator.rs` |
| §5 Access Control | 4 | `mcp_access_control` table, delegation anti-spoofing | `db/mcp.rs`, `managers/mcp.rs`, `managers/mcp_kernel_tool.rs` |
| §6 Audit Trail | 2 | `audit_logs` table, `mgp.audit.replay` | `db/audit.rs`, `managers/mcp_kernel_tool.rs` |
| §7 Code Safety | 1 | `validate_mcp_code()` | `managers/mcp.rs` |
| §8–10 Isolation | — | — (see MGP_ISOLATION_DESIGN.md) | — |
| §11 Lifecycle | 2+4 | `ServerStatus`, health checks, graceful shutdown | `managers/mcp_lifecycle.rs`, `managers/mcp_kernel_tool.rs` |
| §12 Streaming | 2+3 | Stream chunks, progress, flow control, cancellation | `managers/mcp_streaming.rs`, `managers/mcp_kernel_tool.rs` |
| §13 Bidirectional | 2+3+4 | Event subscriptions, replay, callbacks | `managers/mcp_events.rs`, `managers/mcp_kernel_tool.rs` |
| §14 Error Handling | — | MGP error codes (1000-5099), structured hints | `managers/mcp_mgp.rs`, `managers/mcp_protocol.rs` |
| §15 Discovery | 4 | Server registry, runtime register/deregister | `managers/mcp_discovery.rs` |
| §16 Tool Discovery | 4 | `ToolIndex`, `SessionToolCache`, LLM meta-tools | `managers/mcp_tool_discovery.rs`, `managers/mcp_kernel_tool.rs` |

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

**Tier 1 — Minimal (hours):** Add `mgp` to `initialize` capabilities + `tool_security`
metadata on `tools/list`. ~80 lines of code for clients, ~70 lines for servers.

```python
# Server: 3 lines to add MGP Tier 1 support
from mgp import enable_mgp
enable_mgp(server, permissions=["network.outbound"], trust_level="standard")
```

**Tier 2 — Security (1 week):** Permission approval flow (`permissions`), audit events
(`audit`), structured error handling (`error_handling`), access control (`access_control`).

**Tier 3 — Communication (2-4 weeks):** Lifecycle management (`lifecycle`), streaming
(`streaming`, `progress`), callbacks (`callbacks`), events (`events`).

**Tier 4 — Full (1-2 months):** Dynamic tool discovery (`tool_discovery` — Mode A+B),
context budget management, session tool cache. Semantic search is OPTIONAL — keyword +
category is sufficient for Tier 4 compliance.

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

**Server Tier 1 total: ~70 lines.** Just declare `tool_security` fields on tools.

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
| Phase 0 | MGP Specification (v0.6.0-draft) | Draft complete |
| — | ClotoCore Tier 1-4 Implementation (20 kernel tools, 13 extensions) | **Complete** |
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
| 0.6.0-draft | 2026-03-06 | Transport layer analysis (see §18.4) + structural audit & architectural revision (see §18.5) |
| 0.6.0-impl | 2026-03-07 | ClotoCore Tier 1-4 implementation complete: 20 kernel tools, 13 extensions, bug-182 to bug-226 fixed. New modules: `mcp_mgp.rs`, `mcp_lifecycle.rs`, `mcp_streaming.rs`, `mcp_events.rs`, `mcp_discovery.rs`, `mcp_tool_discovery.rs` |

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

Structural analysis revealed that 15 of 25 protocol methods are kernel-side operations
that do not require bidirectional protocol agreement. Converting these to standard MCP
tools via `tools/call` preserves all functionality while reducing protocol surface area
by 64%.

**Result:** 25 → 12 protocol primitives (4 methods + 8 notifications).

- **Layer 1 (Metadata):** `_mgp` fields on existing MCP messages — 0 new methods
- **Layer 2 (Notifications):** 8 protocol notifications
- **Layer 3 (Methods):** 4 irreducible methods (permission/await, permission/grant,
  callback/respond, stream/cancel)
- **Layer 4 (Kernel Tools):** 17 standard MCP tools with `mgp.*` naming convention
  (originally 15; `mgp.audit.replay` and `mgp.events.replay` added for notification
  reliability — see §6.3 and §13.6)

Security guarantees and MCP structural limitation breakthroughs are fully maintained
because the kernel remains the sole enforcement point regardless of invocation mechanism.

### 18.4 Transport Layer Analysis (0.5.2 → 0.6.0)

Discord Bridge implementation revealed a structural gap in MCP's transport model.
MCP transports assume the Kernel (Client) is the initiator; external event-driven
servers (Discord, Slack, IoT) invert this assumption.

**Key finding:** JSON-RPC notifications (Server→Client) are protocol-legal on all
transports, but most MCP client implementations only handle request-response pairs.
This is a specification-implementation gap affecting the entire MCP ecosystem.

**Changes:**

| Section | Change |
|---------|--------|
| §1.4 | Added: MGP may define optional transport extensions; notification handling is MUST |
| §19.5 | Added `transport_websocket` (Medium priority) with rationale and design constraints |
| §19.6 | New: External Event Bridge Pattern — recommended architecture for event-driven servers |

**Design decision:** Two-phase approach. Phase 1 standardizes the Bridge Pattern (§19.6)
and requires Kernel notification handling. Phase 2 (future) introduces `transport_websocket`
as an optional extension informed by Phase 1 operational experience.

### 18.5 Structural Audit & Architectural Revision (0.6.0)

Comprehensive audit against ClotoCore codebase and architectural flaw analysis.

#### Documentation Audit Fixes

| Area | Change |
|------|--------|
| trust_level taxonomy | Unified 3-level (Spec) → 4-level: `core > standard > experimental > untrusted` (§2.3, §16) |
| Error codes | Fixed mismatches: `3002→3001`, `1004→1010`, `1001→1000` in MGP_ISOLATION_DESIGN.md |
| Config naming | `mgp.toml` → `mcp.toml` unified (§15.2) |
| Security Invariant 3 | "cannot be self-declared" → "cannot be self-elevated" (MGP_ISOLATION_DESIGN.md §10) |
| File paths | `db.rs` → `db/mcp.rs` etc. in §17.3 mapping table |
| ServerStatus | Added `Implemented` column to §11.2 |
| clientInfo | `ClotoCore/0.2.8` → `CLOTO-KERNEL/0.6.0` |

#### Structural Flaw Revisions (#2–#7)

| # | Flaw | Resolution | Sections |
|---|------|-----------|----------|
| #2 | Extension negotiation too coarse | Split `security` → `permissions` + `tool_security`, `bidirectional` → `callbacks` + `events`, added `progress` (9 → 12 extensions) | §2.2 |
| #3 | Delegation concept missing | `_mgp.delegation` field, `intersect(actor, delegator)` permission model, chain depth max 3 | §5.6 (new) |
| #4 | Permission scope missing | `scope` object with `paths`/`deny_paths`, `commands`/`deny_commands`, `hosts`/`deny_hosts`, deny-first precedence | §3.6, §3.7 (new) |
| #5 | Security metadata self-declaration | `effective_risk_level` (kernel-derived, authoritative) vs `risk_level` (server-declared, informational). Source column in §4.3 | §4.3, §4.6 (new) |
| #6 | Kernel tools self-referential | `mgp.*` namespace reservation, `_mgp.source: "kernel"`, `admin_only` flag, MUST NOT in LLM context | §1.6.3 (new), §4.3 |
| #7 | Streaming backpressure missing | `notifications/mgp.stream.pace` (rate hints), `notifications/mgp.stream.gap` (gap detection + retransmission) | §12.8, §12.9 (new) |

#### Notification Reliability (#1)

Layer 2 notifications (JSON-RPC 2.0) have no delivery guarantee. This revision adds
compensating mechanisms for both communication directions:

| Mechanism | Direction | Sections |
|-----------|-----------|----------|
| `_mgp.seq` on audit notifications + `mgp.audit.replay` kernel tool | K→S | §6.3 |
| Callback retry with `_mgp.attempt` + kernel deduplication | S→K | §13.4 |
| `_mgp.seq` on event notifications + `mgp.events.replay` kernel tool | K→S | §13.3, §13.6 (new) |
| Layer 2 Notification Reliability table | — | §1.6.2 (new) |

Kernel Tool count: 15 → 17 (`mgp.audit.replay`, `mgp.events.replay` added).

§1.2 Auditable principle revised: "persist to local audit store and forward as
notifications" (previously: "produce structured audit events").

#### Future Considerations

§20 (new): Protocol Layer Evolution — documents structural asymmetry (JSON-RPC 2.0
inherited), layer isolation analysis, potential migration path, and decision criteria
with earliest evaluation 2027-Q2.

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
| **Archive** | Full history in memory server (CPersona etc.) | Never (persistent) |

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
| `transport_websocket` | Full-duplex WebSocket transport for event-driven servers | Medium |
| `observability` | OpenTelemetry-compatible metrics and traces | Low |
| `versioning` | Tool schema versioning and migration | Low |

#### `transport_websocket` — Rationale

The Discord Bridge implementation (§19.6) demonstrated that MCP's existing transports
have a structural limitation for **external event-driven servers**:

| Transport | Event-driven server support |
|-----------|---------------------------|
| stdio | Server can write notifications to stdout, but long-running external connections (WebSocket, MQTT) conflict with stdout for logging and concurrency |
| Streamable HTTP | Server must be an HTTP server; incompatible with servers that are WebSocket *clients* to external systems |
| WebSocket (proposed) | True full-duplex; single process can maintain external connections and MCP communication simultaneously |

The `transport_websocket` extension would allow the Kernel to connect to an MCP/MGP
server via WebSocket, enabling bidirectional JSON-RPC messaging without process separation.
The transport is selected via configuration (pre-connection), not negotiated at `initialize`
time, since the connection is already established when `initialize` is sent.

```json
// initialize response — transport field is informational, not negotiated
{
  "mgp": {
    "version": "0.6.0",
    "extensions": ["permissions", "tool_security", "callbacks", "events"],
    "transport": "websocket"
  }
}
```

**Design constraint:** Every MGP server using `transport_websocket` MUST also support
at least one MCP-standard transport (stdio or Streamable HTTP) as fallback, ensuring
backward compatibility with standard MCP clients.

Phase 2 design will be informed by operational experience from the External Event Bridge
Pattern (§19.6).

These extensions will follow the same design principle: optional extensions negotiated
during `initialize`, with full backward compatibility to both MCP and earlier MGP versions.

### 19.6 External Event Bridge Pattern

External systems (Discord, Slack, Telegram, MQTT, Webhooks, etc.) generate events that
should be processed by MGP agents. These systems maintain persistent connections
(WebSocket, long-polling) that conflict with MCP's stdio model.

#### Problem: MCP's Initiator Model

MCP transports assume the Kernel (Client) is the initiator:

```
stdio:           Kernel starts Server subprocess, writes to stdin
Streamable HTTP: Kernel connects to Server's HTTP endpoint
```

External event sources invert this: the **Server** detects events and needs to
**push** them to the Kernel. While JSON-RPC notifications (Server→Client) are
protocol-legal, most MCP client implementations only handle request-response pairs.

#### Recommended Architecture: Two-Process Bridge

Until `transport_websocket` (§19.5) is available, the following architecture is
the recommended pattern for external event-driven MGP servers:

```
┌──────────────────┐      ┌─────────────────────┐      ┌──────────┐
│ External System  │      │ Bridge Process       │      │ MGP      │
│ (Discord, Slack) │─WS──►│ (persistent daemon)  │─HTTP─►│ Kernel   │
│                  │      │                      │      │          │
│                  │      │  - External WS conn  │      │          │
│                  │      │  - SSE/MGP listener   │◄─SSE─│          │
│                  │      │  - Internal HTTP API  │      │          │
│                  │      └──────────┬───────────┘      │          │
│                  │                 │ HTTP              │          │
│                  │      ┌──────────▼───────────┐      │          │
│                  │      │ MCP Server           │─stdio─►│          │
│                  │      │ (tool provider)      │      │          │
│                  │      │                      │      │          │
│                  │      │  - send_message      │      │          │
│                  │      │  - list_channels     │      │          │
│                  │      │  - get_history       │      │          │
│                  │      └──────────────────────┘      └──────────┘
```

**Bridge Process** handles external connections and Kernel API communication.
**MCP Server** provides tools for agent-initiated actions, delegating to Bridge
via internal HTTP API. This separation ensures stdio is never contaminated by
external connection traffic.

#### Implementation Requirements

1. **Kernel notification handling (MUST):** The Kernel's MCP client implementation
   MUST process Server→Client JSON-RPC notifications (messages without `id`).
   This is a prerequisite for receiving `notifications/mgp.event` and is required
   by the JSON-RPC 2.0 specification but commonly unimplemented.

2. **Bridge Process:** Maintains persistent external connections, forwards
   external events to Kernel via `POST /api/message`, and receives responses
   via SSE or `notifications/mgp.event` subscription.

3. **MCP Server:** Pure tool provider connected to Kernel via stdio. Delegates
   external system operations to Bridge via internal HTTP API (localhost only).

4. **Security:** Bridge internal HTTP API MUST bind to localhost only. External
   system credentials (bot tokens, API keys) MUST be held exclusively by the
   Bridge Process, never passed to the MCP Server.

#### Migration Path

When `transport_websocket` becomes available, this two-process architecture can
be collapsed into a single process:

```
Before (Bridge Pattern):  2 processes + IPC
After  (WebSocket):       1 process, WebSocket to Kernel + WebSocket to external
```

The MCP tool interface remains identical — only the transport and process
topology change.

**MGP Primitives Used:** Event Subscription (§13.2), Push Notifications (§13.3),
Lifecycle (§11), Access Control (§5), Audit (§6)

---

## 20. Future Considerations: Protocol Layer Evolution

### 20.1 Structural Asymmetry

MGP inherits JSON-RPC 2.0's client-server model from MCP. The kernel (client) can
make method calls to servers (with response guarantee), but servers can only send
notifications to the kernel (fire-and-forget, no delivery guarantee).

This asymmetry is compensated by application-level mechanisms (`_mgp.seq`, replay
tools, callback retry — see §1.6.2), but cannot be fully eliminated within the
current protocol layer.

### 20.2 Layer Isolation

MGP's 4-layer architecture cleanly separates transport-dependent and
transport-independent concerns:

| Layer | Transport-dependent? | Notes |
|-------|---------------------|-------|
| Layer 1 (Metadata) | No | `_mgp` fields are payload-level |
| Layer 2 (Notifications) | **Yes** | Asymmetry exists here |
| Layer 3 (Methods) | No | Application semantics only |
| Layer 4 (Kernel Tools) | No | Standard `tools/call` |

A future protocol evolution would only need to replace Layer 2, preserving
Layers 1, 3, and 4 unchanged.

### 20.3 Potential Migration Path

If operational data (callback timeout frequency, event replay `truncated` rate)
demonstrates that Layer 2 compensation is insufficient, a symmetric transport
layer could be introduced:

```
Kernel (Next-Gen)
├── MGP Adapter   ← JSON-RPC 2.0 (notification + callback pattern)
│   └── Existing MGP Servers (unchanged)
└── Native        ← Bidirectional protocol (method calls both directions)
    └── Next-Gen Servers
```

**Backward compatibility is one-directional only:** existing MGP servers work on
a next-gen kernel (via adapter), but next-gen servers do NOT work on an MGP-only
kernel. This follows the HTTP/1.1 → HTTP/2 precedent.

### 20.4 Decision Criteria

This migration is NOT planned. The following conditions should be monitored
before considering it:

1. **MCP evolution**: If MCP adds server→client method calls (e.g., via
   Streamable HTTP), MGP's asymmetry resolves at the MCP level — no custom
   protocol needed
2. **Operational data**: Frequency of `CALLBACK_TIMEOUT` audit events and
   `mgp.events.replay` responses with `truncated: true`
3. **Ecosystem size**: Migration cost scales with the number of third-party
   MGP server implementations

**Earliest evaluation: 2027-Q2.**
