# ClotoCore Code Quality Audit Report

**Version:** 0.6.3-alpha.11
**Audit Date:** 2026-03-22
**Scope:** Rust Core, Dashboard UI, Security, Tests, Build/Infrastructure
**Methodology:** Static analysis across 5 parallel investigation tracks

---

## Executive Summary

A comprehensive code quality audit identified **65 findings** across the full ClotoCore stack. The codebase demonstrates strong fundamentals — zero unsafe code, parameterized SQL, correct path traversal defense, and consistent API response formatting — but has significant gaps in security defaults, test coverage, and frontend robustness.

| Severity | Count |
|----------|-------|
| Critical | 2 |
| High | 19 |
| Medium | 27 |
| Low | 17 |
| **Total** | **65** |

### Category Distribution

| Category | CRIT | HIGH | MED | LOW | Total |
|----------|------|------|-----|-----|-------|
| Security | 2 | 7 | 4 | 2 | 15 |
| Architecture | — | 2 | 1 | 2 | 5 |
| Rust Code Quality | — | — | 8 | 3 | 11 |
| Frontend | — | 3 | 7 | 4 | 14 |
| Tests | — | 5 | 3 | — | 8 |
| CI/CD & Build | — | 2 | 5 | 6 | 13 |

---

## Positive Findings

Before detailing issues, the following strong points should be acknowledged:

- **Zero `unsafe` blocks** — The entire Rust codebase is safe Rust
- **SQL injection resistance** — All queries use `sqlx` parameterized binds; no string interpolation in SQL
- **Path traversal defense** — `canonicalize()` + `starts_with()` correctly defeats `../` and symlink attacks
- **DOMPurify usage** — All `dangerouslySetInnerHTML` sites are sanitized with DOMPurify
- **API response consistency** — All handlers use `ok_data()` helper; no raw `json!()` responses
- **DB migration integrity** — 50 migrations in correct chronological order; rename lifecycle handled properly
- **Version synchronization** — `Cargo.toml`, `package.json`, `tauri.conf.json` all at `0.6.3-alpha.11`
- **Constant-time auth comparison** — `subtle::ConstantTimeEq` used for API key verification
- **Audit logging** — All admin actions logged with retry and backoff

---

## CRITICAL

### C-1: Live API Keys in `.env` File

**File:** `.env:4-8`
**Category:** Secret Exposure

The `.env` file contains real provider API keys:

```
DEEPSEEK_API_KEY=sk-22456fcb...
CEREBRAS_API_KEY=csk-jyn9v4h...
CLOTO_API_KEY=dev-test-key-2026
```

While `.gitignore` correctly excludes `.env`, on-disk keys are exposed if the dev machine is compromised, the directory is shared, or zip-archived. The `CLOTO_API_KEY` value is 18 characters — below the recommended 32-character minimum.

**Recommendation:** Rotate all three keys immediately. Generate a cryptographically random admin key via `openssl rand -hex 32`. Consider per-session env injection instead of `.env` files with real credentials.

---

### C-2: Non-Cryptographic Hash for API Key Revocation

**File:** `crates/core/src/db/api_keys.rs:13-24`
**Category:** Cryptography

`hash_api_key()` uses `std::collections::hash_map::DefaultHasher` — explicitly non-cryptographic and not stable across Rust releases. The output is `val || val XOR constant`, meaning effective entropy is only 64 bits. If Rust's `DefaultHasher` implementation changes between versions, all previously revoked keys silently become valid again.

The `sha2` crate is already in `Cargo.toml` dependencies.

**Recommendation:** Replace with SHA-256. Use `sha2::Sha256` to produce a stable, cryptographic hash. Migrate existing revocation entries on upgrade.

---

## HIGH

### Security

#### H-1: Auth Bypass via `CLOTO_DEBUG_SKIP_AUTH` in Release Builds

**File:** `crates/core/src/handlers.rs:116-128`

When `CLOTO_API_KEY` is unset and `CLOTO_DEBUG_SKIP_AUTH=1`, every admin endpoint is fully open. There is no `#[cfg(debug_assertions)]` guard. A production deployment where `CLOTO_API_KEY` was accidentally unset could be bypassed by any process that can set environment variables.

**Recommendation:** Gate behind `#[cfg(debug_assertions)]` or remove entirely. In release builds, absence of `CLOTO_API_KEY` should be a hard startup error, not a warning.

---

#### H-2: `allow_unsigned` Defaults to `true` in Production

**File:** `crates/core/src/config.rs:479-481`

```rust
allow_unsigned: env::var("CLOTO_ALLOW_UNSIGNED")
    .map(|v| v == "true" || v == "1")
    .unwrap_or(true), // Default: true (development mode)
```

Magic Seal verification is disabled by default in all builds. Unsigned MCP servers are silently accepted unless operators explicitly set `CLOTO_ALLOW_UNSIGNED=false`.

**Recommendation:** Default to `false`. Require explicit opt-in for unsigned servers via env var.

---

#### H-3: Dynamic MCP Server Creation Writes Arbitrary Code to Disk

**File:** `crates/core/src/handlers/mcp.rs:326-371`

`POST /api/mcp/servers` with a `code` field writes user-supplied Python code to `scripts/mcp_{name}.py` and spawns it. Authentication is required, but no sandboxing is applied to the written code at the HTTP endpoint level. The `validate_mcp_code` check only runs through the kernel tool path (YOLO mode), not the HTTP path.

**Recommendation:** Apply `validate_mcp_code` to the HTTP endpoint as well. Consider writing to a controlled data directory rather than the relative `scripts/` path.

---

#### H-4: LLM Proxy Has No Authentication — CLOSED (By Design)

**File:** `crates/core/src/managers/llm_proxy.rs:66-68`

The internal LLM proxy (`127.0.0.1:8082`) accepts requests from any local process without authentication. MCP servers with `NetworkScope::None` (Untrusted) can circumvent network isolation by connecting to `127.0.0.1:8082`, consuming stored API keys.

~~**Recommendation:** Require a per-session token for LLM proxy access. Issue tokens only to authorized MCP servers at startup.~~

**Resolution (2026-03-24):** Closed as By Design. The proxy separation is required by P5 (Strict Permission Isolation). Merging into the `/api` router would require sharing admin API keys with MCP server subprocesses, which is strictly worse. The `127.0.0.1` binding is the security boundary. If future hardening is needed, per-session lightweight tokens (not admin keys) injected at MCP spawn time would be the correct approach.

---

#### H-5: LLM Proxy Has No Rate Limiting — CLOSED (By Design)

**File:** `crates/core/src/managers/llm_proxy.rs`

The main API rate limiter applies only to port 8081. The LLM proxy on port 8082 has zero rate limiting. A runaway MCP subprocess can issue unlimited LLM API calls.

~~**Recommendation:** Apply a configurable rate limit to the LLM proxy endpoint.~~

**Resolution (2026-03-24):** Closed as By Design. Callers are kernel-spawned trusted MCP servers, not untrusted clients. Upstream LLM providers enforce their own rate limits (429 responses are already translated to structured errors). Adding kernel-side rate limiting would only add latency for trusted processes.

---

#### H-6: Tauri CSP Contains `unsafe-inline` and `unsafe-eval`

**File:** `dashboard/src-tauri/tauri.conf.json:29`

```
script-src 'self' 'unsafe-inline' 'unsafe-eval'
```

`unsafe-eval` enables `eval()`, `new Function()`, and string-based `setTimeout`. Together with `unsafe-inline`, CSP is largely ineffective against XSS in the webview.

**Recommendation:** Remove `unsafe-eval` if possible. If required by a dependency (Three.js/VRM), document which dependency needs it and track removal.

---

#### H-7: `shell:allow-execute` Unscoped in Tauri Capabilities

**File:** `dashboard/src-tauri/capabilities/default.json:20`

The `shell:allow-execute` capability applies to both `main` and `vrm-viewer` windows without scope restrictions. The frontend can execute arbitrary shell commands.

**Recommendation:** Scope to specific executables. Separate capabilities per window (vrm-viewer likely needs no shell access).

---

### Architecture Violations

#### H-8: Hard-Coded `"output.avatar"` Bypasses CapabilityDispatcher

**Files:** `crates/core/src/handlers/system.rs:565`, `crates/core/src/managers/mcp.rs:1300`

```rust
granted_server_ids.contains(&"output.avatar".to_string())  // system.rs:565
if server_id == "output.avatar" && tool.name == "speak"     // mcp.rs:1300
```

These bypass the `CapabilityDispatcher` entirely, violating Principle 1 (Core Minimalism) and Principle 2 (Capability over Concrete Type).

**Recommendation:** Add `CapabilityType::Speech` or `CapabilityType::Avatar` variant and route through the dispatcher.

---

#### H-9: Access Control Fields Use Magic Strings Instead of Enums

**Files:** `crates/core/src/managers/agents.rs:226-228`, `crates/core/src/db/mcp.rs`, `crates/core/src/handlers/chat.rs:101`

`entry_type`, `permission`, and `status` fields are raw `String` values compared against string literals like `"server_grant"`, `"allow"`, `"deny"`. A typo in any call site silently evaluates to the wrong branch.

**Recommendation:** Define `EntryType`, `PermissionLevel`, and `MessageSource` enums with `serde` deserialization.

---

### Test Coverage Gaps

#### H-10: `handlers/system.rs` — 1,865 Lines, Zero Tests

The agentic loop, memory recall, tool execution, consensus routing, and approval gating are completely untested. 7 open HIGH/CRITICAL bugs exist in this file.

#### H-11: `mcp_kernel_tool.rs` — 1,914 Lines, Zero Tests

All 25 MGP kernel-native tools are untested. This file was the source of a 30+ bug sprint in v0.6.0-beta.3.

#### H-12: `handlers/marketplace.rs` — 1,746 Lines, Zero Tests

The install flow, catalog cache, and rate limiting have no test coverage.

#### H-13: DB Persistence — 8 of 9 Modules Have Zero Tests

Only `db/mod.rs` has 4 tests. The following have none: `api_keys.rs`, `audit.rs`, `chat.rs`, `cron.rs`, `llm.rs`, `mcp.rs`, `permissions.rs`, `trusted_commands.rs`.

#### H-14: Three Vacuously Passing Tests

| Test | Issue |
|------|-------|
| `security_forging_test.rs:233` | `assert!(blocked)` where `blocked` is a hardcoded `true` literal |
| `plugin_lifecycle_test.rs:141` | No assertion — "test passes if no panic" |
| `sse_streaming_test.rs:169` | No assertion — silently passes on timeout |

---

### Frontend

#### H-15: Zero Frontend Component Tests

Only 3 utility test files exist (13 `it()` cases). No tests for any React component, hook, page, or service. 13 React bugs (244-271) were fixed in batch with no regression tests.

#### H-16: Near-Zero Accessibility (`aria-label`)

Only 1 `aria-label` in the entire frontend. All buttons rely solely on `title` attributes, which screen readers handle differently.

#### H-17: Missing `React.memo` — Only 2 Components Memoized

Only `MemoryCore` and `CronJobs` use `memo()`. `AgentConsole` (995 lines), `AgentTerminal` (646 lines), and `SetupWizard` (825 lines) re-render fully on every parent state change.

---

### CI/CD

#### H-18: `tauri-apps/tauri-action@v0` Not SHA-Pinned

**File:** `.github/workflows/release.yml:257,338,420`

The release workflow uses a floating `@v0` tag for the action that signs production binaries with `TAURI_SIGNING_PRIVATE_KEY`.

**Recommendation:** Pin to a specific commit SHA.

#### H-19: `dtolnay/rust-toolchain@stable` Not SHA-Pinned

**Files:** `ci.yml` (4 occurrences), `release.yml` (4 occurrences)

A supply chain compromise of this widely-used action would affect all CI jobs.

**Recommendation:** Pin all uses to a commit SHA.

---

## MEDIUM

### Rust Code Quality

| # | Finding | File(s) |
|---|---------|---------|
| M-1 | `let _ = sender.send(...)` silently discards SSE broadcast / agentic loop send errors; UI may miss responses | `events.rs:250,302,310,337` |
| M-2 | 37+ fire-and-forget `tokio::spawn` without JoinHandle tracking; panics undetectable, graceful shutdown impossible | `events.rs`, `lib.rs`, `system.rs` |
| M-3 | `std::sync::Mutex` in async context with `.lock().unwrap()`; poison cascades on panic | `mcp_tool_discovery.rs`, `mcp_streaming.rs` |
| M-4 | Read lock held during clone + spawn in `PermissionGranted` handler; blocks other writers | `events.rs:368-385` |
| M-5 | Trusted-command persistence error discarded in `command_approval` | `command_approval.rs:169` |
| M-6 | Secret-masking logic duplicated verbatim in same file | `handlers/mcp.rs:494-547` |
| M-7 | `CLOTO_MAX_CRON_GENERATION` parsed outside `AppConfig` with no range validation | `lib.rs:397-400` |
| M-8 | `create_mcp_server` uses synchronous `std::fs::write` in async handler + relative path | `handlers/mcp.rs:363-365` |

### Frontend

| # | Finding | File(s) |
|---|---------|---------|
| M-9 | `handleToggle`/`handleRunNow` in `CronJobs.tsx` have no error handling; API failures are silent | `CronJobs.tsx:105-123` |
| M-10 | `AdvancedSection.tsx` fires direct `api.put()` on toggle with no Save pattern; violates Agent Config Rule (strict reading) | `AdvancedSection.tsx:33,43` |
| M-11 | `text-content-muted` used on visible/interactive text in 10+ components; violates Dashboard UI Rules | Multiple components |
| M-12 | Modals lack focus trap, `role="dialog"`, and `aria-modal` | `PowerToggleModal.tsx`, `AgentTerminal.tsx` |
| M-13 | `LogSection.tsx` auto-scroll fires on mount only (empty deps); new logs do not auto-scroll | `LogSection.tsx:40-42` |
| M-14 | `setTimeout(refetch, 500)` after server state changes; race condition if operation takes longer | `McpServersPage.tsx:87,96,105` |
| M-15 | `eslint-disable react-hooks/exhaustive-deps` in 3 locations; structural fix needed in `useAsyncAction` | `McpServerSettingsTab.tsx:42` et al. |

### Security (Medium)

| # | Finding | File(s) |
|---|---------|---------|
| M-16 | External `POST /api/events/publish` creates `EnvelopedEvent::system()` (issuer=None); external events can masquerade as kernel events | `handlers/events.rs:55` |
| M-17 | Marketplace progress SSE and Setup endpoints are unauthenticated; leaks internal paths and config | `lib.rs:1005`, `handlers/setup.rs` |
| M-18 | OS isolation is Phase 1 only — memory_limit, max_child_processes are metadata labels, not enforced | `mcp_isolation.rs` |
| M-19 | CORS defaults to `http://` origins; API keys transmitted in cleartext without TLS | `config.rs:161-162` |

### Tests (Medium)

| # | Finding | File(s) |
|---|---------|---------|
| M-20 | `consensus.rs` (state machine, proposal collection, synthesis) has zero tests | `consensus.rs` |
| M-21 | `CLOTO_DEBUG_SKIP_AUTH` env var set in test but never cleaned up; bleeds into parallel tests | `handlers_http_test.rs:164` |
| M-22 | No Unicode/multibyte input tests across HTTP handlers or DB layer | All test files |

### Build & Dependencies

| # | Finding | File(s) |
|---|---------|---------|
| M-23 | `base64` version mismatch: 0.22 (core) vs 0.21 (app); not in workspace dependencies | `Cargo.toml` (core/app) |
| M-24 | `tracing-subscriber` overrides workspace definition; may lose `env-filter` feature | `crates/core/Cargo.toml:17` |
| M-25 | `sqlx` features split between workspace and crate-level; `uuid`/`chrono` potentially missing | `Cargo.toml`, `crates/core/Cargo.toml` |
| M-26 | `@types/marked` v5 declared for `marked` v17; v5+ ships own types, this dependency is dead | `dashboard/package.json` |
| M-27 | `Cross.toml` uses `:main` mutable Docker image tag; non-reproducible release builds | `Cross.toml` |

---

## LOW

| # | Finding | File(s) |
|---|---------|---------|
| L-1 | `unwrap_or_default()` on `serde_json::to_string` silently produces empty strings on chat content | `system.rs:151,551,684` |
| L-2 | `unwrap_or_default()` on `get_granted_server_ids` silently grants no server access on DB error | `system.rs:178-180` |
| L-3 | `CapabilityMapping.priority` is dead code with `#[allow(dead_code)]` | `capability_dispatcher.rs:29` |
| L-4 | `agent.synthesizer` and `system.cron` used as magic strings in event data | `consensus.rs:254`, `scheduler.rs:139` |
| L-5 | `to_string_lossy().to_string()` double-allocation in transport | `mcp_transport.rs:25` |
| L-6 | `json_data()` silently returns `{ "data": null }` on serialization failure | `handlers/response.rs:15` |
| L-7 | `_color = agentColor(agent)` computed but unused every render | `AgentTerminal.tsx:349` |
| L-8 | Raw `api` import instead of `useApi()` hook in `ContentBlockView.tsx` | `ContentBlockView.tsx:6` |
| L-9 | `vrmWindowRef: any` should be `WebviewWindow \| null` | `lib/tauri.ts:54` |
| L-10 | Array index used as React `key` in 4 list renders | `ContentBlockView.tsx:201`, `AgentTerminal.tsx:542`, `LogSection.tsx:49`, `CommandApprovalCard.tsx:101` |
| L-11 | API key in SSE query string (known `EventSource` limitation); inconsistent param names (`token` vs `api_key`) | `useEventStream.ts:32`, `InstallDialog.tsx:74`, `SetupWizard.tsx:267` |
| L-12 | LLM API keys stored plaintext in `llm_providers` SQLite table | `db/llm.rs:11` |
| L-13 | LLM proxy error messages leak provider config details | `managers/llm_proxy.rs:163` |
| L-14 | SearXNG `secret_key` hardcoded in checked-in config (localhost-only deployment) | `infra/searxng/settings.yml:4` |
| L-15 | Node 20 in CI vs Node 22 locally; Node 20 EOL April 2026 | CI workflow files |
| L-16 | Dead `TARGET_BIN` variable in `start_cloto.sh` | `scripts/start_cloto.sh:29` |
| L-17 | `pkill -9` (SIGKILL) in startup script skips graceful DB shutdown | `scripts/start_cloto.sh:21-22` |

---

## Test Coverage Analysis

### Current State

**Claimed:** 90 Rust tests (baseline)
**Actual `#[test]`/`#[tokio::test]` attributes found:** 234 (189 unit + 45 integration)
**Frontend:** 3 test files, 14 `it()` cases (utility functions only)

### Modules With Zero Test Coverage

| Module | Lines | Criticality |
|--------|-------|-------------|
| `handlers/system.rs` | 1,865 | Agentic loop, memory, tools, consensus |
| `managers/mcp_kernel_tool.rs` | 1,914 | All 25 MGP kernel tools |
| `handlers/marketplace.rs` | 1,746 | Install flow, catalog |
| `handlers/setup.rs` | 814 | Setup wizard backend |
| `handlers/agents.rs` | 780 | Agent CRUD |
| `managers/mcp_client.rs` | 407 | JSON-RPC client |
| `managers/agents.rs` | 402 | Agent manager |
| `managers/registry.rs` | 477 | Plugin registry / event dispatch |
| `consensus.rs` | ~300 | Multi-engine orchestration |
| `db/api_keys.rs` | — | Key revocation |
| `db/chat.rs` | — | Chat persistence |
| `db/cron.rs` | — | CRON persistence |
| `db/mcp.rs` | — | MCP server state |
| `db/permissions.rs` | — | Permission requests |
| All React components | — | 0 component/hook tests |

### Test Isolation Issues

- `CLOTO_DEBUG_SKIP_AUTH` environment variable set in `handlers_http_test.rs:164` without cleanup; bleeds into parallel unit tests
- `CLOTO_SEAL_KEY` and `TEST_CLOTO_VAR` also leak across tests
- `cargo test` runs unit tests in parallel within each crate, making env var mutations a documented flakiness source

### Open Issue Registry

17 open issues remain in `qa/issue-registry.json`:
- 1 CRITICAL (bug-287: kernel-native tool names bypass access control)
- 12 HIGH (architecture violations: hard-coded tool names, metadata interpretation in kernel)
- 2 MEDIUM (StdioTransport Drop, shutdown drain)
- 1 LOW (restart policy defaults)
- None of the 17 open issues have associated test cases

---

## Recommended Priority Actions

### Immediate (This Sprint)

1. **Rotate API keys** (C-1) — `.env` keys should be treated as compromised
2. **Replace `DefaultHasher` with SHA-256** (C-2) — `sha2` crate already available
3. **Gate `CLOTO_DEBUG_SKIP_AUTH` behind `#[cfg(debug_assertions)]`** (H-1)
4. **Default `allow_unsigned` to `false`** (H-2)
5. ~~**Add auth to LLM proxy** (H-4)~~ — Closed: By Design (P5)

### Short-Term (Next 2 Sprints)

6. ~~**Add rate limiting to LLM proxy** (H-5)~~ — Closed: By Design
7. **Tighten Tauri CSP** — remove `unsafe-eval` if possible (H-6)
8. **Scope `shell:allow-execute`** per window (H-7)
9. **Eliminate `"output.avatar"` hard-coding** via CapabilityType extension (H-8)
10. **Define enums for access control fields** (H-9)
11. **Pin all CI actions to SHA** (H-18, H-19)
12. **Add tests for `handlers/system.rs`** (H-10) — highest-risk untested code
13. **Fix env var leakage in tests** (M-21)

### Medium-Term (Next Quarter)

14. Add tests for `mcp_kernel_tool.rs`, `marketplace.rs`, `consensus.rs`, and DB modules
15. Add frontend component tests with Vitest + Testing Library
16. Implement `aria-label` across all interactive elements
17. Add `React.memo` to expensive components; split monolithic components
18. Unify dependency versions and workspace inheritance (M-23 through M-27)
19. Implement Phase 2 OS-level isolation enforcement (M-18)
20. Add Unicode/boundary input tests (M-22)

---

*This audit was conducted via static analysis. Runtime testing may reveal additional issues not captured here.*

*Previous audit: CODE_QUALITY_AUDIT.md (2026-02-13, Score: 65/100 → post-remediation: 90+/100)*
