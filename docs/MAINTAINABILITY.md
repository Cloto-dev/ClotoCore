# ClotoCore Maintainability Report

A document recording the results of a comprehensive maintainability scan and a list of improvement actions.
Please update this document continuously to support development decision-making and technical debt management.

**Last Updated**: 2026-03-06
**Target Version**: 0.6.0-alpha.3

---

## 1. Project Structure Overview

```
ClotoCore/
├── crates/
│   ├── core/        # Kernel: handlers/, db/, managers/, events, middleware
│   └── shared/      # Shared trait definitions
├── mcp-servers/     # 5 MCP servers (cerebras, cpersona, deepseek, embedding, terminal)
├── dashboard/       # React + TypeScript + Tauri 2.x
├── scripts/         # Utility scripts
├── qa/              # issue-registry.json (source of truth for bug verification)
└── .dev-notes/      # Maintenance notes (gitignored, supplementary materials)
```

**Tech Stack**: Rust (workspace with 2 crates) / TypeScript + React / Python

---

## 2. Code Size Metrics

### Major File Sizes (Line Count)

| File | Lines | Status |
|---|---|---|
| `crates/core/src/handlers/system.rs` | 1829 | ⚠️ Under monitoring (agentic loop) |
| `crates/core/src/handlers/mcp.rs` | 966 | Acceptable |
| `crates/core/src/managers/mcp.rs` | 1122 | ⚠️ Under monitoring |
| `crates/core/src/lib.rs` | 769 | Acceptable |
| `crates/core/src/handlers.rs` | 580 | ✅ Split from 1668 → routing module + 12 sub-handlers |
| `crates/shared/src/lib.rs` | 560 | Acceptable |
| `crates/core/src/handlers/agents.rs` | 508 | Good |
| `crates/core/src/capabilities.rs` | 464 | Good |
| `crates/core/src/db/mod.rs` | 438 | ✅ Split from 1658 → routing module + 8 sub-modules |
| `crates/core/src/managers/` | (directory) | 15 modules, ~3,800 total |
| `crates/core/src/handlers/` | (directory) | 12 modules, ~4,940 total |
| `crates/core/src/db/` | (directory) | 9 modules, ~1,750 total |

### Test Scale

| Item | Count |
|---|---|
| Total test functions (`#[test]` + `#[tokio::test]`) | ~90 |
| Files with `#[cfg(test)]` blocks | 15 |
| Integration test files (`crates/core/tests/`) | 16 |
| Estimated coverage | ~35% |

---

## 3. Issues and Concerns

### 🔴 Requires Immediate Action

#### A. bug-017 in `qa/issue-registry.json` is a false positive

`bug-017` ("CI/CD: cargo audit security check missing") has `"status": "open"`, but
`cargo audit` already exists at `ci.yml:63-64`. **The registry is inconsistent with reality**.

---

### ✅ Resolved (For Reference)

- **`managers.rs` bloat**: Split into `managers/` directory completed (15 modules)
- **`evolution.rs` bloat**: Archived to `archive/evolution/`, file deleted
- **`db.rs` bloat** (v0.6.0-alpha): Split from 1658 lines into `db/` directory (9 modules: mod.rs, mcp.rs, chat.rs, permissions.rs, cron.rs, audit.rs, api_keys.rs, llm.rs, trusted_commands.rs)
- **`handlers.rs` bloat** (v0.6.0-alpha): Split from 1668 lines into `handlers/` directory (12 modules: system.rs, mcp.rs, agents.rs, chat.rs, llm.rs, utils.rs, commands.rs, permissions.rs, events.rs, assets.rs, response.rs, cron.rs)
- **`mcp.rs` refactoring** (bug-144): Extracted mcp_client.rs, mcp_types.rs, mcp_kernel_tool.rs, mcp_health.rs, mcp_tool_validator.rs from monolithic mcp.rs

---

### 🟠 High Priority (Within 1 Month)

#### B. Insufficient Test Coverage (~35%)

The following critical paths are untested:

| Untested Scenario | Priority |
|---|---|
| DB migration rollback | HIGH |
| Plugin initialization failure handling | HIGH |
| Concurrent event storm (100+ events/sec) | HIGH |
| Memory exhaustion (large event history) | MEDIUM |
| Rate limiter cleanup race condition | MEDIUM |
| Permission approval workflow | MEDIUM |

Mock plugins are not implemented, so tests depend on real plugins.
A `MockPlugin` implementation (`crates/core/tests/mocks/mod.rs`) is needed.

---

### 🟡 Medium Priority (Within 3 Months)

#### ~~C. Lack of Pagination for DB Full Retrieval (M-03)~~ ✅ Resolved

Implemented `DEFAULT_MAX_RESULTS = 1_000` + overflow detection + `db_timeout()` in `db::get_all_json()`.

#### ~~D. Graceful Shutdown Not Implemented~~ ✅ Resolved

- Fixed `shutdown.notify_one()` to `notify_waiters()` (notifies all tasks)
- Added shutdown signal to MCP notification listener
- All background tasks monitor shutdown via `tokio::select!`

#### E. Unclear Error Messages (M-05)

DB operation errors and configuration errors lack context information,
reducing debugging efficiency. Add "which agent" and "which operation" to errors.

#### F. No Upper Limit on Event History

Under high-frequency events (1000/min), 60-minute retention could accumulate up to 60,000 entries ≈ 60MB.

```rust
// Recommended addition
const MAX_EVENT_HISTORY: usize = 10_000;
history.retain(|e| e.timestamp > cutoff);
if history.len() > MAX_EVENT_HISTORY {
    history.drain(..history.len() - MAX_EVENT_HISTORY);
}
```

---

### 🟢 Low Priority

| Item | Description | Effort |
|---|---|---|
| `clone()` optimization (L-01) | 160+ calls (mostly legitimate Arc clones) | 1-2h |
| Enable `clippy::pedantic` | Get additional lint suggestions | 1h |
| `rustfmt.toml` configuration | Project-specific formatting settings | 30min |
| DB connection pool env variable | Make `SQLX_MAX_CONNECTIONS` configurable | 10min |
| Rate limiter cleanup frequency | 10min to 2min (low impact) | 5min |

---

## 4. Code Quality Summary

### `unwrap()` Usage (Total: 245 Occurrences)

After detailed examination, the impact on production code is limited:

| Category | Count | Action |
|---|---|---|
| Inside `#[cfg(test)]` blocks | ~230 | Acceptable (test use) |
| Benchmarks (`benches/`) | ~10 | Acceptable |
| Safe usage after `is_some()` check | 2 | Acceptable |
| Production code requiring review | ~5 | See below |

**Production code locations requiring review**:
- `dashboard/src-tauri/src/lib.rs:108` — Tauri icon retrieval (acceptable as framework convention)

### `#[allow(dead_code)]` (12 Occurrences)

| Location | Reason | Verdict |
|---|---|---|
| `plugins/moderator/src/lib.rs` | Fields for future UI | ✅ Legitimate |
| `crates/core/src/handlers.rs:797` | Investigation needed | ⚠️ Requires review |

### TODO/FIXME/HACK Comments

**Only 1** (virtually zero). Excellent condition. Thorough management via Issue Tracker.

---

## 5. CI/CD Assessment

| Check | Status | Notes |
|---|---|---|
| `cargo fmt --check` | ✅ | CI required |
| `cargo clippy -D warnings` | ✅ | CI required |
| `cargo test --workspace` | ✅ | CI required |
| `cargo audit` | ✅ | CI required (bug-017 is false positive, fix needed) |
| Dashboard build + test | ✅ | CI required |
| Release: checksum generation | ✅ | SHA256 |
| Release: cosign signing | ✅ | Keyless signing, excellent |
| GitHub Actions pinning | ✅ | All steps pinned to commit hashes |
| Concurrent CI control | ✅ | `concurrency` configured |
| `cargo audit` local | ⚠️ | Not installed in dev environment |
| Windows installer | ✅ | Tauri NSIS (replaced Inno Setup in v0.6.0-alpha.1) |
| Cross-compilation | ✅ | linux-x64/arm64, macOS-x64/arm64, win-x64 |

---

## 6. Overall Assessment

| Aspect | Rating | Comment |
|---|---|---|
| CI/CD | **A** | Robust pipeline, security-focused |
| Security | **B+** | Critical vulnerabilities fixed, ongoing monitoring needed |
| Testing | **C+** | 105 test functions but estimated 35% coverage |
| Code Quality | **B** | No `unwrap` abuse, near-zero TODOs is excellent |
| File Structure | **A-** | `managers/`, `db/`, `handlers/` all split into focused modules |
| Documentation | **A-** | docs/ fully English-unified, architecture and metrics updated for v0.6.0-alpha.3 |
| Dependency Management | **B+** | Workspace unified, lockfile present |

---

## 7. Action List

### Immediate Action

- [x] bug-017 in `qa/issue-registry.json` — Archived (fix confirmed)
- [x] Add `LIMIT` to `db::get_all_json()` — `DEFAULT_MAX_RESULTS = 1_000` implemented
- [x] Graceful shutdown — `notify_waiters()` + all tasks shutdown support

### Within 1 Month

- [ ] Implement `MockPlugin` (`crates/core/tests/mocks/mod.rs`)

### Within 3 Months

- [ ] Test coverage 35% to 60% (prioritize DB migration, plugin init failure, concurrent storm)
- [ ] Error message improvement (M-05): Add context to DB/Config errors
- [ ] Set upper limit on event history (`MAX_EVENT_HISTORY = 10_000`)
- [ ] Enable `clippy::pedantic` and fix warnings

---

## 8. Previously Fixed Bugs (For Reference)

17 bugs were fixed in the recent development cycle (see `qa/issue-registry.json`).
Key items:

| Severity | Description |
|---|---|
| CRITICAL | Panic risk from `.expect()` in SafeHttpClient |
| CRITICAL | Missing panic cleanup for TUI terminal |
| CRITICAL | `socket` module leak in Python sandbox |
| HIGH | DeepSeek/Cerebras plugins silently accepting empty API keys |
| HIGH | CLI SSE URL path mismatch |
| HIGH | Multi-byte character UTF-8 slice panic in `logs.rs` |
| HIGH | `config set` command writing API keys to disk |
| HIGH | TUI scroll position unclamped and direction inverted |

---

*This document records the results of a maintainability scan.*
*Next scan recommended: **2026-06-04** (3 months later)*
