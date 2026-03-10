# ClotoCore Architecture Compliance Audit

**Audit Date:** 2026-03-10
**Target Version:** v0.5.6
**Audit Scope:** 9 Design Principles + 8 Development Guardrails + UI Rules + API Conventions

---

## Executive Summary

| Area | Verdict | Violations |
|------|---------|------------|
| Principle 1: Core Minimalism | **NON-COMPLIANT** | 21 |
| Principle 2: Capability over Concrete Type | **NON-COMPLIANT** | 11 |
| Principle 3: Event-First Communication | **COMPLIANT** | 0 |
| Principle 4: Data Sovereignty | **NON-COMPLIANT** | 6 |
| Principle 5: Strict Permission Isolation | **PARTIAL** | 4 |
| Principle 6: Seamless Integration & DevEx | **COMPLIANT** | -- |
| Principle 7: Polyglot Extension | **FULLY COMPLIANT** | 0 |
| Principle 8: Dynamic Intelligence Orchestration | **NOT IMPLEMENTED** | 3 (design gaps) |
| Principle 9: Self-Healing AI Containerization | **MOSTLY COMPLIANT** | 2 |
| Guardrail 1.1: Event Envelopes | **COMPLIANT** | 0 |
| Guardrail 1.2: Cascading Protection | **FULLY COMPLIANT** | 0 |
| Guardrail 1.3: State Management | **NON-COMPLIANT** | 4 |
| Guardrail 1.4: Storage & Memory | **NON-COMPLIANT** | 2 |
| Guardrail 1.5: UI/UX Clarity | **PARTIAL** | 1 |
| Guardrail 1.6: Physical Safety | **COMPLIANT** | 0 |
| Guardrail 1.7: MCP Resource Control | **COMPLIANT** | 0 |
| Guardrail 1.8: Privacy & Biometrics | **COMPLIANT** | 0 |
| API Response Format Convention | **FULLY COMPLIANT** | 0 |
| Dashboard UI Text Readability | **COMPLIANT** | 0 |

**Overall: 10/18 areas compliant (55.6%) / 12 CRITICAL, 22 HIGH, 5 MEDIUM, 1 LOW, 3 design gaps**

---

## Violation Severity Summary

| Severity | Count | Primary Areas |
|----------|-------|---------------|
| CRITICAL | 12 | P1 (LLM/plugin ID hard-coding), P4 (schema intrusion), P5 (YOLO/network) |
| HIGH | 22 | P1+P2 (tool name hard-coding), P4 (metadata interpretation), G1.3+G1.4 |
| MEDIUM | 5 | P9 (process cleanup), G1.3 (lock type inconsistency) |
| LOW | 1 | Documentation mismatch |
| Design Gap | 3 | P8 (dynamic permissions) |

---

## 1. Principle 1: Core Minimalism -- NON-COMPLIANT (21 violations)

> *"The Kernel is the stage, not the actor."*

The most severe concentration of violations. The kernel actively participates in domain
logic for memory, vision, LLM, and consensus operations instead of acting as neutral
infrastructure.

### 1.1 CRITICAL Violations (7)

#### C1-01: Anthropic-Specific Authentication in Kernel

- **File:** `crates/core/src/managers/llm_proxy.rs`
- **Lines:** 29, 146-149
- **Detail:** Kernel hard-codes `ANTHROPIC_PROVIDER_ID = "claude"` and implements
  provider-specific header handling (`x-api-key`, `anthropic-version`).
- **Impact:** Adding a new LLM provider with non-standard auth requires kernel modification.

#### C1-02: Memory Plugin Called by ID (3 locations)

- **File:** `crates/core/src/handlers.rs`
- **Lines:** 336, 380, 395
- **Detail:** `call_server_tool("memory.cpersona", ...)` hard-coded in three locations
  for memory operations (`list_memories`, `list_episodes`, `delete_memory`,
  `delete_episode`).
- **Impact:** Cannot swap `memory.cpersona` for another memory plugin without kernel changes.

#### C1-03: Memory Response Format Parsing

- **File:** `crates/core/src/handlers.rs`
- **Lines:** 321-343
- **Detail:** `parse_cpersona_result()` and `call_cpersona_with_fallback()` parse
  CPersona's specific JSON response structure (extracts first text content, parses as JSON).
- **Impact:** Kernel has intimate knowledge of a specific plugin's response contract.

#### C1-04: Vision Plugin Called by ID (2 locations)

- **File:** `crates/core/src/handlers/agents.rs` (line 491),
  `crates/core/src/handlers/system.rs` (line 1188)
- **Detail:** `call_server_tool("vision.capture", "analyze_image", ...)` hard-coded.
- **Impact:** Cannot replace `vision.capture` without kernel changes.

#### C1-05: Memory Dispatch Fallback Chain

- **File:** `crates/core/src/handlers/system.rs`
- **Lines:** 165-199
- **Detail:** Kernel implements dual dispatch: `agent.metadata.get("preferred_memory")`
  -> `registry.find_memory()` -> MCP memory server search. This is plugin-specific
  orchestration logic belonging outside the kernel.

#### C1-06: Memory Plugin DB Initialization

- **File:** `crates/core/src/db/mod.rs`
- **Line:** 304
- **Detail:** `INSERT OR REPLACE INTO plugin_configs (plugin_id, ...) VALUES ('memory.cpersona', ...)`
  -- Kernel initializes a specific plugin's database configuration at startup.

### 1.2 HIGH Violations (8)

| ID | File | Lines | Detail |
|----|------|-------|--------|
| H1-01 | `db/llm.rs` | 62-64 | Provider-to-API-key env var mapping hard-coded (deepseek, cerebras, claude) |
| H1-02 | `config.rs` | 179 | Default consensus engines `"mind.deepseek,mind.cerebras"` hard-coded |
| H1-03 | `capabilities.rs` | 22-25 | LLM API host whitelist hard-coded (api.deepseek.com, api.cerebras.ai, api.openai.com, api.anthropic.com) |
| H1-04 | `consensus.rs` | 16 | `SYSTEM_CONSENSUS_AGENT = "system.consensus"` hard-coded |
| H1-05 | `handlers/system.rs` | 240-264 | Memory tool contract (`"recall"`, specific arg structure) hard-coded |
| H1-06 | `handlers/system.rs` | 276-295 | `"consensus:"` prefix routing hard-coded |
| H1-07 | `managers/registry.rs` | 98-106 | `find_memory()` assumes plugins have memory trait |
| H1-08 | `managers/mcp.rs` | 1584-1592 | `find_memory_server()` hard-codes `store` + `recall` tool requirement |

### 1.3 Root Cause

The kernel acts as an orchestrator for domain-specific workflows (memory recall,
vision analysis, LLM routing) rather than delegating these to a composable
plugin layer. All 21 violations stem from the kernel "knowing" about specific
plugins and their contracts.

---

## 2. Principle 2: Capability over Concrete Type -- NON-COMPLIANT (11 violations)

> *"Not who it is, but what it can do."*

### 2.1 CRITICAL Violation (1)

#### C2-01: Tool Name-Based Access Control Bypass

- **File:** `crates/core/src/managers/registry.rs`
- **Line:** 301
- **Detail:** Bypasses `check_tool_access()` for tools matching concrete string literals:
  `"mgp."`, `"gui."`, `"create_mcp_server"`, `"ask_agent"`.
- **Impact:** Contradicts DEVELOPMENT.md Guardrail 1.1 (no hard-coded privilege checks).

### 2.2 HIGH Violations (10)

All in `crates/core/src/handlers/system.rs` unless noted:

| ID | Lines | Hard-Coded Tool Name | Context |
|----|-------|---------------------|---------|
| H2-01 | 248 | `"recall"` | Memory context retrieval |
| H2-02 | 483 | `"store"` | Memory storage for agent responses |
| H2-03 | 670 | `"store"` | User message memory persistence |
| H2-04 | 1074 | `"think"` | Engine reasoning capability |
| H2-05 | 1116 | `"think_with_tools"` | Engine tool-use capability |
| H2-06 | 1399-1401 | `"list_memories"` | Memory query |
| H2-07 | 1421-1423 | `"list_episodes"` | Episode query |
| H2-08 | 1472 | `"archive_episode"` | Memory archival |
| H2-09 | 1488 | `"update_profile"` | Profile update |
| H2-10 | `handlers/agents.rs:318` | `"delete_agent_data"` | Agent cleanup |

### 2.3 Impact

`CapabilityType`-based dispatching is not functional. Plugin swapping depends
entirely on string literal matching. A `CapabilityDispatcher` mapping
`CapabilityType -> (server_id, tool_name)` would resolve all 10 HIGH violations.

---

## 3. Principle 3: Event-First Communication -- COMPLIANT

> *"Don't talk directly -- announce it in the plaza."*

- Event bus fully asynchronous via `mpsc` channels
- Concurrent plugin dispatch via `FuturesUnordered`
- `ask_agent()` is kernel-mediated (not direct plugin-to-plugin)
- All cascade events go through `redispatch_plugin_event()`
- Context isolation enforced for inter-agent delegation

**No violations detected.**

---

## 4. Principle 4: Data Sovereignty -- NON-COMPLIANT (6 violations)

> *"The Kernel holds the data, but does not interpret its contents."*

### 4.1 CRITICAL Violations (2)

#### C4-01: Avatar Fields as Schema Columns

- **File:** `crates/core/migrations/20260302100000_add_agent_avatar.sql` (lines 4-5)
- **Detail:** `avatar_path` and `avatar_description` added as dedicated columns to
  `agents` table instead of opaque JSON metadata.

#### C4-02: VRM Path as Schema Column

- **File:** `crates/core/migrations/20260309300000_add_vrm_path.sql` (line 2)
- **Detail:** `vrm_path` added as a dedicated column to `agents` table.

### 4.2 HIGH Violations (4)

| ID | File | Lines | Detail |
|----|------|-------|--------|
| H4-01 | `handlers/engine_routing.rs` | 70-95 | Kernel deserializes `engine_routing` metadata into `RoutingRule` struct and applies routing logic |
| H4-02 | `handlers/system.rs` | 166 | Kernel reads `preferred_memory` metadata to route memory access |
| H4-03 | `managers/agents.rs` | 44-51 | Kernel reads `avatar_*` and `vrm_path` columns and populates metadata |
| H4-04 | `migrations/20260309200000_fix_agent_metadata_ks22.sql` | 6-8 | Kernel modifies `preferred_memory` metadata value via SQL |

### 4.3 Remediation

Move `avatar_path`, `avatar_description`, `vrm_path` into the existing `metadata`
JSON column. Engine routing rules should be handled by a routing plugin, not parsed
by the kernel.

---

## 5. Principle 5: Strict Permission Isolation -- PARTIAL

> *"Capability comes with responsibility and authorization."*

### 5.1 Compliant Areas

- **Event Enveloping:** `EnvelopedEvent` prevents source ID spoofing
- **SafeHttpClient:** Blocks localhost/private IPs with DNS rebinding protection
- **3-Level RBAC:** `tool_grant > server_grant > default_policy` fully implemented
- **ActionRequested Validation:** Requester-issuer verification + rate limiting

### 5.2 CRITICAL Violations (2)

#### C5-01: MCP Servers Have Unrestricted Network Access

- **Files:** All `mcp-servers/**/server.py`
- **Detail:** MCP servers freely instantiate `httpx.AsyncClient` without kernel
  mediation. `SafeHttpClient` constraints are not applied to MCP processes.
- **Impact:** MCP servers can access localhost, private IPs, internal services,
  and exfiltrate data without kernel oversight.

#### C5-02: YOLO Mode Bypasses All Permission Validation

- **File:** `crates/core/src/managers/mcp.rs` (lines 328-360)
- **Detail:** When `yolo_mode = true`, all `required_permissions` are auto-approved
  with approver ID `"YOLO"`. No audit trail differentiation.
- **Impact:** Any MCP server in `mcp.toml` can declare arbitrary permissions and
  receive instant approval, completely bypassing HITL.

### 5.3 HIGH Violations (2)

| ID | Detail |
|----|--------|
| H5-01 | `execute_tool()` global method has zero permission checks (only agent-scoped execution validates access) |
| H5-02 | No runtime capability injection for MCP servers (Rust SafeHttpClient objects cannot cross process boundary) |

---

## 6. Principle 7: Polyglot Extension -- FULLY COMPLIANT

> *"Guard the core with Rust, spread the wings with Python."*

- Rust core (Axum, SQLite, tokio) + 17 Python MCP servers
- Three-Tier Plugin Model completely superseded (macros, plugins/, inventory removed)
- `mcp.toml` supports any language (Python + Rust avatar binary)
- All advanced computation (LLMs, embeddings, vision) via MCP
- Language-agnostic JSON-RPC 2.0 protocol

**No violations detected.**

---

## 7. Principle 8: Dynamic Intelligence Orchestration -- NOT IMPLEMENTED

> *"Capabilities are not given -- they are earned."*

### 7.1 Design Gaps (3)

| ID | Gap | Detail |
|----|-----|--------|
| G8-01 | `PermissionRequested` event type missing | `ClotoEventData` enum has no such variant. Described in ARCHITECTURE.md S2.2 but never implemented |
| G8-02 | No runtime permission escalation flow | All permissions declared at startup in MCP manifest. Plugins cannot request new permissions during execution |
| G8-03 | No Dashboard Security Guard UI | No runtime approval prompt mechanism in the dashboard |

### 7.2 Impact

The principle envisions "intent-based dynamic permission granting via HITL."
The current implementation is a static startup-time permission model with no
runtime escalation capability. This is a **fundamental design gap**, not a
code-level bug.

---

## 8. Principle 9: Self-Healing AI Containerization -- MOSTLY COMPLIANT

> *"Even if it dies, it resurrects and keeps moving forward."*

### 8.1 Compliant Areas

- Health monitor: 30-second interval checks (`mcp_health.rs`)
- Restart policies: Never/OnFailure/Always with exponential backoff (`mcp_lifecycle.rs`)
- PID tracking: `is_alive()` non-blocking check + `kill_on_drop(true)`
- Timeout protection: All MCP requests wrapped with configurable timeouts (10-600s)
- Pending request cleanup on process death
- Windows process isolation (CREATE_NO_WINDOW flag)

### 8.2 MEDIUM Violations (2)

| ID | File | Lines | Detail |
|----|------|-------|--------|
| M9-01 | `managers/mcp.rs` | 426-469 | Race condition on retry failure: I/O tasks (writer/reader/logger) may be orphaned when `StdioTransport` drops during failed initialization. Missing explicit `Drop` impl with `child.kill()` |
| M9-02 | `handlers.rs` | 172-210 | Shutdown handler does not call `drain_server()` for active MCP servers. `drain_server()` exists (mcp.rs:1619) but is never invoked during shutdown |

### 8.3 LOW (1)

| ID | Detail |
|----|--------|
| L9-01 | Default restart policy values differ from MGP_COMMUNICATION.md (max_restarts: 3 vs doc 5, backoff_base: 100ms vs doc 1000ms, backoff_max: 5000ms vs doc 30000ms) |

---

## 9. Guardrail 1.2: Cascading Protection -- FULLY COMPLIANT

- `EnvelopedEvent.depth: u8` tracking
- `MAX_EVENT_DEPTH` configurable (1-50, default 10, validated at startup)
- Depth check: `current_depth >= max_event_depth` at dispatch time
- Increment: `depth: current_depth + 1` on all cascade paths (3 locations)
- No silent drops: All discard paths include `error!` logging with full context

**No violations detected.**

---

## 10. Guardrail 1.3: State Management -- NON-COMPLIANT (4 violations)

> Related settings must be grouped into a single `Arc<RwLock<ConfigStruct>>`.

| ID | File | Lines | Detail | Severity |
|----|------|-------|--------|----------|
| G3-01 | `managers/mcp.rs` | 32, 35, 40 | `servers`, `tool_index`, `stopped_configs` as 3 independent RwLocks. Sequential lock acquisition (line 802) risks inconsistent state | HIGH |
| G3-02 | `managers/registry.rs` | 15-16 | `plugins`, `effective_permissions` as 2 independent RwLocks. Race condition: permissions checked against stale plugin state | HIGH |
| G3-03 | `consensus.rs` | 66-67 | `sessions`, `config` as 2 independent RwLocks. Config updates during consensus processing could cause inconsistency | MEDIUM |
| G3-04 | `lib.rs` | 85, 91 | Mixed lock types: `event_history` uses `tokio::sync::RwLock`, `revoked_keys` uses `std::sync::RwLock` | MEDIUM |

---

## 11. Guardrail 1.4: Storage & Memory -- NON-COMPLIANT (2 violations)

> LLMs expect context where "newer is further down."

| ID | File | Lines | Detail | Severity |
|----|------|-------|--------|----------|
| G4-01 | `handlers/system.rs` | 1336-1387 | `parse_mcp_recall_result()` collects messages in original order without reversal. LLM receives incorrect chronological ordering | HIGH |
| G4-02 | `handlers/system.rs` | 1067-1072 | Context messages passed to engines without order adjustment | HIGH |

---

## 12. Guardrail 1.5: UI/UX Clarity -- PARTIAL (1 violation)

| ID | Detail | Severity |
|----|--------|----------|
| G5-01 | No database-level or backend filtering to prevent function-only plugins from appearing in the agents list. `list_agents()` returns all rows without category filtering. The `agents` table lacks a `category` or `agent_type` column | MEDIUM |

---

## 13. API Response Format Convention -- FULLY COMPLIANT

- All handlers use `ok_data()` helper consistently
- All errors go through `AppError::IntoResponse` with `error` envelope
- Zero instances of direct `json!({ "status": ... })` construction
- Response helpers centralized in `handlers/response.rs`

---

## 14. Dashboard UI Text Readability -- COMPLIANT

- No `text-[8px]` or smaller found
- All `text-content-muted` usage is decorative (icons, disabled states, dividers)

---

## Root Cause Analysis

The majority of violations stem from a single structural pattern:
**the kernel implements domain-specific orchestration logic**.

```
Root Cause: Kernel acts as domain orchestrator
    |
    +-- P1 violations: Kernel knows specific plugins (memory.cpersona, vision.capture)
    +-- P2 violations: Kernel calls tools by name ("recall", "store", "think")
    +-- P4 violations: Kernel interprets plugin metadata (engine_routing, preferred_memory)
    +-- P8 gap: Permission model is kernel-managed static, not plugin-initiated dynamic
```

Introducing a **CapabilityDispatcher** abstraction that maps
`CapabilityType -> (server_id, tool_name)` would resolve the majority of
P1 (15/21) and all P2 (11/11) violations simultaneously.

---

## Recommended Remediation Roadmap

### P0 -- Immediate

| Item | Resolves | Effort |
|------|----------|--------|
| Add safety limits to YOLO mode (explicit warning, audit differentiation, sensitive permission exclusion) | C5-02 | Small |
| Add `.reverse()` to `parse_mcp_recall_result()` return | G4-01, G4-02 | Trivial |

### P1 -- Short-Term

| Item | Resolves | Effort |
|------|----------|--------|
| Implement `CapabilityDispatcher` registry (`CapabilityType -> (server_id, tool_name)`) | 15x P1, 10x P2 | Medium |
| Move `avatar_path`, `avatar_description`, `vrm_path` into metadata JSON | C4-01, C4-02 | Small |
| Add `ClotoEventData::PermissionRequested` variant | G8-01 | Small |
| Add explicit `Drop` for `StdioTransport` with `child.kill()` | M9-01 | Trivial |
| Call `drain_server()` in shutdown handler | M9-02 | Small |

### P2 -- Mid-Term

| Item | Resolves | Effort |
|------|----------|--------|
| MCP server network constraints (HTTP proxy requirement or namespace isolation) | C5-01 | Large |
| Consolidate fragmented RwLocks in McpClientManager | G3-01 | Medium |
| Runtime permission escalation flow + Dashboard Security Guard UI | G8-01, G8-02, G8-03 | Large |
| LLM proxy provider-agnostic auth adapter | C1-01 | Medium |
| Agent/Tool category column + filtering | G5-01 | Small |

### P3 -- Long-Term

| Item | Resolves | Effort |
|------|----------|--------|
| Extract memory orchestration to a dedicated MCP coordinator plugin | C1-02 through C1-06 | Large |
| Move engine routing logic out of kernel | H4-01 | Medium |
| Align default restart policy values with MGP_COMMUNICATION.md | L9-01 | Trivial |

---

*Generated by architecture compliance audit, 2026-03-10.*
*Auditor: Claude Opus 4.6*
