# MGP — Model General Protocol

**Version:** 0.6.0-draft
**Status:** Draft
**Authors:** ClotoCore Project
**Date:** 2026-03-06

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
5. **Auditable** — All security-relevant actions are persisted to the kernel's local
   audit store and forwarded as structured notifications to subscribed servers

### 1.3 Compatibility Matrix

| Client | Server | Behavior |
|--------|--------|----------|
| Standard MCP | Standard MCP | Standard MCP operation |
| Standard MCP | MGP Server | MCP operation (MGP extensions silent) |
| MGP Client | Standard MCP | MCP operation (client uses fallback behavior) |
| MGP Client | MGP Server | Full MGP operation |

All four patterns are functional. No configuration changes are required.

### 1.4 Transport

MGP inherits MCP's transport layer. All transports supported by MCP (stdio, Streamable HTTP)
are supported by MGP. MGP additionally defines optional transport extensions that can be
negotiated via §2. These optional transports are never required — every MGP server MUST
support at least one MCP-standard transport as fallback.

**Implementation Note:** MGP Kernel implementations MUST handle Server→Client JSON-RPC
notifications (messages without `id`) on all transports. This is required by the JSON-RPC
2.0 specification and MCP, but is commonly unimplemented in practice. Without this capability,
MGP §13 (Bidirectional Communication) and §19.6 (External Event Bridge) cannot function.

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

MGP extends MCP with only **12 protocol primitives** (4 methods + 8 notifications) organized in four layers. All
other functionality is provided as standard MCP tools exposed by the kernel.

```
┌──────────────────────────────────────────────────────────┐
│  Layer 1: Metadata Extensions (0 new methods)            │
│  └─ _mgp fields on initialize, tools/list, tools/call   │
├──────────────────────────────────────────────────────────┤
│  Layer 2: Protocol Notifications (8)                     │
│  ├─ notifications/mgp.audit                              │
│  ├─ notifications/mgp.stream.chunk                       │
│  ├─ notifications/mgp.stream.progress                    │
│  ├─ notifications/mgp.stream.pace                        │
│  ├─ notifications/mgp.stream.gap                         │
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
│  Layer 4: Kernel Tools (17 — standard tools/call)        │
│  ├─ mgp.access.*    (query, grant, revoke)       — §5   │
│  ├─ mgp.audit.*     (replay)                    — §6   │
│  ├─ mgp.health.*    (ping, status)               — §11  │
│  ├─ mgp.lifecycle.* (shutdown)                   — §11  │
│  ├─ mgp.events.*    (subscribe, unsubscribe,     — §13  │
│  │                    replay)                            │
│  ├─ mgp.discovery.* (list, register, deregister) — §15  │
│  └─ mgp.tools.*     (discover, request,          — §16  │
│                       session, session.evict)            │
└──────────────────────────────────────────────────────────┘
```

#### Design Rationale

**Layers 1-3** are protocol-level primitives. All MGP implementations MUST support
these. They are irreducible: each requires bidirectional agreement or fire-and-forget
notification that cannot be expressed as a tool call.

#### 1.6.2 Layer 2 Notification Reliability

JSON-RPC 2.0 notifications have no delivery guarantee (no `id`, no acknowledgment).
Layer 2 notifications are therefore **best-effort** by design. MGP compensates for
this with per-notification recovery mechanisms:

| Notification | Direction | Reliability | Recovery Mechanism |
|---|---|---|---|
| `mgp.stream.chunk` | Server→Kernel | Best-effort | §12.5: final response contains complete result. §12.9: gap detection |
| `mgp.stream.progress` | Server→Kernel | Best-effort | None required (informational only) |
| `mgp.stream.pace` | Kernel→Server | Best-effort | None required (rate hint, non-binding) |
| `mgp.stream.gap` | Kernel→Server | Best-effort | Final response is fallback |
| `mgp.lifecycle` | Server→Kernel | Best-effort | `mgp.health.ping` provides eventual consistency |
| `mgp.audit` | Kernel→Server | Best-effort + local persistence | Kernel MUST persist locally (§6.3). `mgp.audit.replay` for catch-up |
| `mgp.callback.request` | Server→Kernel | Retry-with-timeout | Server retries with same `callback_id`. Kernel deduplicates (§13.4) |
| `mgp.event` | Kernel→Server | Best-effort + sequence tracking | `_mgp.seq` enables gap detection. `mgp.events.replay` for catch-up (§13.6) |

**Kernel→Server notifications** include `_mgp.seq` (a per-server monotonically
increasing sequence number) to enable gap detection by the receiving server.
Server→Kernel notifications do not require sequence numbers because each has an
independent recovery mechanism.

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

This architecture reduces MGP's protocol surface area by 52% (25 → 12 primitives)
while maintaining full security guarantees and MCP structural limitation breakthroughs.

#### 1.6.3 Kernel Tool Namespace and Visibility

**Namespace reservation:** The `mgp.*` tool name prefix is reserved for kernel tools.
MCP servers MUST NOT register tools with names starting with `mgp.`. If a server
includes an `mgp.*`-prefixed tool in its `tools/list` response, the kernel MUST
reject the tool and SHOULD log a `TOOL_NAME_CONFLICT` audit event.

**Source identification:** Kernel tools include `_mgp.source: "kernel"` in their
tool definitions to distinguish them from server-provided tools:

```json
{
  "name": "mgp.access.grant",
  "_mgp": { "source": "kernel", "admin_only": true }
}
```

**LLM context visibility:** Kernel tools have strict visibility rules for LLM
agent tool contexts:

| Category | In `tools/list` | In LLM Context | Rationale |
|----------|----------------|-----------------|-----------|
| `mgp.tools.discover` | Yes | MUST include | LLM needs discovery to find tools |
| `mgp.tools.request` | Yes | MUST include | LLM needs active tool request |
| `mgp.tools.session`, `mgp.tools.session.evict` | Yes | MAY include | Optional context management |
| `mgp.access.*` | Yes | MUST NOT include | Security management is operator-only |
| `mgp.health.*` | Yes | MUST NOT include | Operational monitoring is operator-only |
| `mgp.lifecycle.*` | Yes | MUST NOT include | Server management is operator-only |
| `mgp.events.*` | Yes | MUST NOT include | Subscription management is operator-only |
| `mgp.discovery.*` | Yes | MUST NOT include | Server registration is operator-only |

**Admin-only enforcement:** Tools marked `admin_only: true` accept calls only from
operator-level requests (e.g., HTTP API, CLI). Tool calls from LLM agents to
admin-only kernel tools MUST be rejected with `1000 PERMISSION_DENIED`.

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


## Document Structure

| File | Sections | Content |
|------|----------|---------|
| [MGP_SECURITY.md](MGP_SECURITY.md) | §2-§7 | Security & Foundation |
| [MGP_COMMUNICATION.md](MGP_COMMUNICATION.md) | §11-§14 | Communication & Lifecycle |
| [MGP_DISCOVERY.md](MGP_DISCOVERY.md) | §15-§16 | Discovery & Intelligence |
| [MGP_GUIDE.md](MGP_GUIDE.md) | §17-§20 | Implementation, History, Patterns |
| [MGP_ISOLATION_DESIGN.md](MGP_ISOLATION_DESIGN.md) | (§8-§10 reserved) | OS-Level Isolation |

### Cross-References

| Section | Detailed Coverage |
|---------|------------------|
| §2-7 Security & Capabilities | [MGP_SECURITY.md](MGP_SECURITY.md) |
| §11-14 Communication & Errors | [MGP_COMMUNICATION.md](MGP_COMMUNICATION.md) |
| §15-16 Discovery & Registration | [MGP_DISCOVERY.md](MGP_DISCOVERY.md) |
| §17-19 Implementation Guide | [MGP_GUIDE.md](MGP_GUIDE.md) |
