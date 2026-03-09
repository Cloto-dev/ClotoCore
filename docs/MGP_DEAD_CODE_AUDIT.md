# MGP Dead Code Audit

**Date**: 2026-03-08
**Scope**: MGP (Model General Protocol) Tier 1-4 implementation
**Base Version**: v0.6.0-beta.3
**Method**: Static analysis (grep cross-reference of all definitions vs. usages) + `cargo check`
**Status**: **RESOLVED** — All 37 items integrated or removed. Zero `dead_code` warnings.

## Resolution Summary

| Phase | Description | Items Resolved |
|-------|-------------|---------------:|
| 0 | Trivial cleanups (delete/remove annotations) | 9 |
| 1 | MGP error code integration (`MgpError` type + `AppError::Mgp`) | 22+ |
| 2 | Lifecycle notification expansion | 2 |
| 3 | Event delivery integration | 5 |
| 4 | Session cache completion (`touch()`, `set_pinned()`) | 2 |
| 5 | StreamAssembler integration | 3 |
| Fix | Residual warnings (filter, pending_callbacks, is_duplicate) | 3 |
| **Total** | | **37+ items** |

### Verification

- `cargo build` — zero `dead_code` warnings
- `cargo test` — 134 unit + 45 integration tests pass
- `cargo clippy` — no new warnings from MGP changes
- `scripts/verify-issues.sh` — all issues verified

---

## Original Audit (37 Dead Items)

### 1. Unused Constants (23) — RESOLVED

**1.1 MGP Error Codes (22)** — All 22 `MGP_ERR_*` constants now used via `MgpError` struct
constructors in `mcp_mgp.rs`. Error handling paths in `mcp.rs`, `mcp_discovery.rs`, and
`mcp_kernel_tool.rs` replaced `anyhow::anyhow!()` with typed `MgpError` variants. Added
`AppError::Mgp` to `lib.rs` with HTTP status code mapping.

**1.2 `TIER1_CLIENT_EXTENSIONS`** — Deleted (superseded by `CLIENT_EXTENSIONS`).

### 2. Unused Functions (5) — RESOLVED

**2.1 `build_mgp_error_data()`** — Now called by `MgpError::to_json_rpc_error()`.

**2.2 `send_gap_notification()`** — Called by `McpClientManager::handle_stream_chunk()`.

**2.3 `send_pace()`** — Exposed as `mgp.stream.pace` kernel tool.

**2.4 `deliver_event()`** — Called at 6 sites: 5 lifecycle transitions + 1 tool execution.

**2.5 `tool_discovery_schemas()`** — Deleted (redundant wrapper).

### 3. Unused Methods (4) — RESOLVED

**3.1 `get_recorded_response()`** — Called in `handle_callback_request()` duplicate path.
Returns recorded response for `CallbackHandleResult::DuplicateWithResponse`.

**3.2 `is_llm_completion()`** — Called in `respond_to_callback()` to detect LLM completion
callbacks before sending response.

**3.3 `set_pinned()`** — Wired in `execute_tools_session()` via new `pinned` parameter.

**3.4 `touch()`** — Called in `execute_tool()` after successful tool execution for LRU update.

### 4. Unused Structs and Fields (5) — RESOLVED

**4.1 `MgpErrorRecovery`** — Used by `MgpError::with_recovery()`. Added `Default` derive.

**4.2 `StreamAssembler`** — Added as field on `McpClientManager`. `handle_stream_chunk()`
processes incoming chunks with gap detection. Stream chunk interception added to `lib.rs`
notification listener.

**4.3 `ToolIndexEntry.tool_id`** — `#[allow(dead_code)]` removed. Field is read by
the tool index search.

**4.4 `CachedTool.tool_id`** — Deleted (never read, HashMap key used instead).

**4.5 `AgentSession.pinned_reserve`** — Deleted (unused in budget enforcement).

### 5. False `#[allow(dead_code)]` Annotations (5) — RESOLVED

All annotations removed:

| File | Item |
|------|------|
| `mcp.rs` | `lifecycle` field |
| `mcp_lifecycle.rs` | `RestartCounter`, `LifecycleManager` struct + impl |
| `mcp_discovery.rs` | `discovery_tool_schemas()` |
| `mcp_events.rs` | `EventManager` struct + all 7 annotations |
| `mcp_streaming.rs` | All `#[allow(dead_code)]` annotations |

### 6. Additional Items Wired During Integration

| File | Item | Integration |
|------|------|-------------|
| `mcp_events.rs` | `EventSubscription.filter` | Filter matching in `deliver_event()` |
| `mcp_events.rs` | `PendingCallback.{message,options,created_at}` | `pending_callbacks()` + `cleanup_stale_callbacks()` |
| `mcp_streaming.rs` | `StreamAssembler::is_duplicate()` | Dedup check in `handle_stream_chunk()` |

---

## Files Modified

| File | Changes |
|------|---------|
| `mcp_mgp.rs` | Deleted `TIER1_CLIENT_EXTENSIONS`, added `MgpError` type, `Default` for `MgpErrorRecovery` |
| `lib.rs` | `AppError::Mgp` variant, HTTP status mapping, `From<MgpError>`, stream chunk interception, callback handling with `CallbackHandleResult` |
| `handlers/mcp.rs` | `MgpError` downcast in error handling |
| `mcp.rs` | Lifecycle/event emit, error replacement, `stream_assembler` field, `handle_stream_chunk()`, `touch()`, `pending_callbacks` dispatch |
| `mcp_kernel_tool.rs` | MGP error replacement, `stream_pace` tool, `pending_callbacks` tool, `stream_assembler.remove()` in cancel |
| `mcp_events.rs` | `CallbackHandleResult` enum, filter matching, `pending_callbacks()`, `cleanup_stale_callbacks()`, `#[allow(dead_code)]` removal |
| `mcp_health.rs` | Lifecycle emit, event delivery, stale callback cleanup |
| `mcp_lifecycle.rs` | `#[allow(dead_code)]` removal |
| `mcp_streaming.rs` | `#[allow(dead_code)]` removal |
| `mcp_discovery.rs` | `#[allow(dead_code)]` removal, MGP error replacement |
| `mcp_tool_discovery.rs` | Deleted dead fields/function, `set_pinned`/`touch` wiring |

All file paths relative to `crates/core/src/managers/` except `lib.rs` (`crates/core/src/lib.rs`)
and `handlers/mcp.rs` (`crates/core/src/handlers/mcp.rs`).
