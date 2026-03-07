# MGP Implementation Roadmap

**Status**: Tier 1-4 Complete
**Created**: 2026-03-07
**Base Version**: v0.6.0-beta.3
**Implementation Commit**: `03b0785` (2026-03-07)

## Overview

This document defines the implementation order for the MGP (Model General Protocol)
in ClotoCore. MGP is the most significant architectural change in the project —
implementation must be cautious and incremental.

MGP is a strict superset of MCP. All existing MCP functionality remains unchanged.
MGP extensions activate only when both client and server negotiate support.

## Implementation Order

```
Tier 1 ✅ → Tier 2 ✅ → Tier 3 ✅ → Tier 4 ✅ → Delegation → SDK Discussion
                                                    ↑
                                    OS Isolation Phase 1 inserted at optimal timing
```

---

## Tier 1: Capability Negotiation + Metadata — ✅ Complete

**Scope**: MGP_SPEC.md §1-2, MGP_SECURITY.md §2, §4
**Effort**: Hours (~70 lines server-side, ~80 lines kernel-side)
**Difficulty**: Very Low
**Dependencies**: None

### What

- Add `mgp` field to `initialize` handshake (capability negotiation)
- Extend `tools/list` response with `tool_security` metadata (risk level,
  permissions required, side effects, validator type)
- Graceful degradation: behave as standard MCP if counterpart doesn't support MGP
- Kernel derives `effective_risk_level` from `trust_level × validator × permissions`

### Key Files

- `crates/core/src/managers/mcp.rs` — negotiation logic
- `crates/core/src/managers/mcp_protocol.rs` — message extensions
- `crates/core/src/managers/mcp_tool_validator.rs` — security metadata

### Acceptance Criteria

- MGP client ↔ MGP server: full negotiation
- MGP client ↔ MCP server: standard MCP operation (fallback)
- MCP client ↔ MGP server: standard MCP operation (extensions silent)
- `effective_risk_level` correctly derived and injected

---

## Tier 2: Security — ✅ Complete

**Scope**: MGP_SECURITY.md §3, §5-7, MGP_COMMUNICATION.md §14
**Effort**: ~1 week
**Difficulty**: Low-Medium
**Dependencies**: Tier 1

### What

- **§3 Permission Approval Flow**: `mgp/permission/await` + `mgp/permission/grant`
  with 11 standard permission types and scopes
- **§5 Access Control**: `mgp.access.grant`, `mgp.access.query`, `mgp.access.revoke`
  kernel tools (extends existing `mcp_access_control`)
- **§6 Audit Trail**: `notifications/mgp.audit` with 12 standard event types,
  `mgp.audit.replay` for catch-up after reconnection
- **§7 Code Safety Framework**: 4 safety levels (unrestricted, standard, strict, readonly)
- **§14 Error Handling**: MGP error code ranges (1000-5099), structured recovery hints

### Key Files

- `crates/core/src/managers/mcp.rs` — permission flow
- `crates/core/src/db/mcp.rs` — access control queries
- `crates/core/src/handlers/mcp.rs` — kernel tool handlers
- `crates/core/src/db/audit.rs` — audit event persistence
- `crates/core/src/managers/mcp_protocol.rs` — error codes

### Acceptance Criteria

- Permission flow blocks server until explicit grant/deny
- Access control resolves: tool_grant > server_grant > default_policy
- All security-relevant actions emit audit notifications
- Code safety validator blocks violations with structured hints
- MGP error codes return retry strategy and fallback suggestions

---

## Multi-Agent Delegation

**Scope**: MULTI_AGENT_DESIGN.md (6 use cases)
**Effort**: 2-4 weeks
**Difficulty**: Medium
**Dependencies**: Tier 2 (access control + audit)

### What

- `ask_agent` tool for agent-to-agent delegation
- Delegation depth limit (default 3), circular reference detection
- Permission matrix (agent-to-agent RBAC via existing `mcp_access_control`)
- Context isolation (only prompt passed, no history/permissions leak)
- Resource limits (timeout, token budget, concurrent delegations)

### Use Cases (priority order)

1. **UC-2 Specialist Consultation** — main agent queries specialist
2. **UC-4 Task Decomposition** — coordinator splits work by capability
3. **UC-5 Review/Verification** — one agent reviews another's work
4. **UC-1 Character Interaction** — multi-turn dialogue between agents
5. **UC-3 Second Opinion** — agent incorporates peer review
6. **UC-6 Cross-Engine Collaboration** — different engines specializing

### Key Files

- `crates/core/src/handlers/system.rs` — `handle_delegation()` method
- `crates/core/src/managers/agents.rs` — delegation access queries
- `crates/core/migrations/` — delegation access control table
- `crates/core/src/events.rs` — `DelegationRequested`/`DelegationCompleted` events

### Acceptance Criteria

- Agent A can delegate to Agent B via `ask_agent` tool
- Delegation depth tracked and enforced (max 3)
- Circular delegation detected and rejected
- Delegation permissions respect existing RBAC
- All delegations logged in audit trail
- UC-2 (Specialist Consultation) works end-to-end

---

## Tier 3: Communication — ✅ Complete

**Scope**: MGP_COMMUNICATION.md §11-13
**Effort**: 2-4 weeks
**Difficulty**: Medium
**Dependencies**: Tier 2

### What

- **§11 Lifecycle Management**: Server state machine (7 states), health checks
  (`mgp.health.ping`, `mgp.health.status`), graceful shutdown
  (`mgp.lifecycle.shutdown`), restart policies
- **§12 Streaming**: `notifications/mgp.stream.chunk` for partial results,
  progress reporting, flow control (`mgp.stream.pace`), gap detection,
  cancellation (`mgp/stream/cancel`)
- **§13 Bidirectional Communication**: Event subscriptions
  (`mgp.events.subscribe/unsubscribe/replay`), server push notifications
  (`notifications/mgp.event` with `_mgp.seq`), callback requests
  (`notifications/mgp.callback.request` + `mgp/callback/respond`),
  `llm_completion` callback type

### Key Files

- `crates/core/src/managers/mcp_lifecycle.rs` — lifecycle state machine, restart policies
- `crates/core/src/managers/mcp_streaming.rs` — stream chunks, progress, flow control
- `crates/core/src/managers/mcp_events.rs` — event subscriptions, replay, callbacks
- `crates/core/src/managers/mcp_kernel_tool.rs` — kernel tool schemas and executors
- `crates/core/src/managers/mcp_health.rs` — health check monitoring

### Acceptance Criteria

- Server lifecycle state transitions visible in dashboard
- Streaming tool calls deliver partial results to UI
- Event subscriptions with gap detection and replay
- Callback requests pause for human input, then resume
- `llm_completion` callback routes to correct LLM engine

---

## Tier 4: Intelligence — ✅ Complete

**Scope**: MGP_DISCOVERY.md §15-16
**Effort**: 1-2 months
**Difficulty**: Medium-High
**Dependencies**: Tier 3

### What

- **§15 Server Discovery**: Static config (mcp.toml), runtime registry
  (`mgp.discovery.list/register/deregister`)
- **§16 Dynamic Tool Discovery**:
  - **Mode A** (user-intent-driven): `mgp.tools.discover` — keyword + category
    search (semantic search optional), returns full tool schemas
  - **Mode B** (agent-autonomy-driven): `mgp.tools.request` — LLM detects
    capability gap, kernel finds/loads matching tools
  - Session tool cache with LRU eviction and context budget enforcement
  - Pinned tools (always in context) + discovery cache
  - Autonomous tool generation (opt-in, ephemeral, experimental trust level)

### Context Reduction Target

| Level | Strategy | Tokens | Reduction |
|-------|----------|--------|-----------|
| L0 (MCP) | All tools injected | ~150K | 0% |
| L1 | Category index | ~5K | 97% |
| L2 | Meta-tool + on-demand | ~1K | 99% |
| L3 (target) | Pinned + discovery cache | ~2K | 99% |

### Key Files

- `crates/core/src/managers/mcp_discovery.rs` — §15 server discovery (3 kernel tools)
- `crates/core/src/managers/mcp_tool_discovery.rs` — §16 tool discovery (ToolIndex, SessionToolCache, 4 kernel tools)
- `crates/core/src/managers/mcp_kernel_tool.rs` — LLM meta-tool schemas, all kernel tool definitions
- `crates/core/src/managers/mcp.rs` — rich_tool_index integration, execute_tool routing

### Acceptance Criteria

- `mgp.tools.discover` returns relevant tools by keyword/category
- `mgp.tools.request` enables LLM to autonomously acquire tools mid-task
- Session cache respects context budget (configurable max_tokens)
- Pinned tools never evicted from context
- Context reduction measurable (>95% vs baseline)
- Autonomous tool generation (if enabled) passes code safety validation

---

## OS Isolation Phase 1 (Flexible Timing)

**Scope**: MGP_ISOLATION_DESIGN.md Phase 1
**Effort**: 1-2 weeks
**Difficulty**: Low
**Timing**: Inserted at the most natural point during development

### What

- **L0 Magic Seal**: HMAC-SHA256 verification of server binaries
- **Soft filesystem isolation**: `chdir` to sandbox + `HOME`/`TMPDIR` env vars
- **Soft network isolation**: `CLOTO_LLM_PROXY` env var
- **Trust level derivation**: `trust_level` → `IsolationProfile` mapping
- **Config**: `[servers.isolation]` section in `mcp.toml`
- **CLI**: `cloto seal generate` / `cloto seal verify`

### Note

Phase 2 (OS resource limits) and Phase 3 (hard isolation with seccomp/namespaces)
are deferred to post-Tier 4. Phase 1 provides security against bugs in honest
servers; hard isolation against adversarial servers comes later.

---

## Post-Tier 4: SDK Discussion

After all tiers are implemented and stabilized, discuss:

- **MGP Specification** publication (MIT license, independent repository)
- **Python SDK** (`mgp-sdk-python`, MIT)
- **TypeScript SDK** (`mgp-sdk-typescript`, MIT)
- **Validation Tool** (`mgp-validate`, compliance testing)
- SDK scope, API surface, and distribution strategy

---

## Principles

1. **Incremental delivery** — each tier is independently valuable
2. **Backward compatibility** — existing MCP behavior never breaks
3. **Security first** — Tier 2 before communication features
4. **Caution over speed** — MGP is the largest change in ClotoCore history
5. **Test at each tier** — no tier begins without prior tier verified
