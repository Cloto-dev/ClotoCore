//! The kernel's integration tests, compiled into one test binary.
//!
//! Each file directly under `tests/` is its own test binary, and every one of
//! them links the whole kernel. With 37 of them, a one-line change to the kernel
//! re-linked 37 near-identical binaries, and linking was about a third of the
//! test rebuild in CI. Here they are modules of a single binary instead.
//!
//! Five files stay under `tests/` as binaries of their own, because they need a
//! process to themselves. Do not move them in here:
//!
//! - `api_key_env_precedence_test.rs` and `api_key_env_precedence_absent_test.rs`
//!   check the API-key sample the kernel takes once at boot. The sample is a
//!   `OnceLock`, so a process can observe only one answer.
//! - `handlers_http_test.rs` sets `CLOTO_DEBUG_SKIP_AUTH`, which turns off the
//!   auth check for every request in the process. Sharing a process with the
//!   security tests would let them pass without the check they exist to test.
//! - `marketplace_install_test.rs` and `marketplace_fetch_test.rs` both point
//!   `CLOTO_CATALOG_URL` at their own mock servers, each behind its own lock.
//!   The locks only serialize tests within one binary.
//!
//! A new test file that changes process-wide state (environment variables, the
//! working directory, a global subscriber) belongs beside those five, not here.

#[path = "../common/mod.rs"]
mod common;
#[path = "../common/kernel_spawn.rs"]
mod kernel_spawn;

mod agent_power_password_test;
mod agent_token_caller_test;
mod capability_gate_test;
mod chat_search_test;
mod concurrent_events_test;
mod consensus_test;
mod conversation_title_test;
mod conversations_test;
mod corrupt_db_recovery_test;
mod e2e_workflows_test;
mod event_cascading_test;
mod event_memory_management_test;
mod kernel_bind_policy_test;
mod kernel_integration_test;
mod kernel_stop_signal_test;
mod kernel_stop_with_open_stream_test;
mod mcp_server_listing_test;
mod migration_test;
mod notification_api_test;
mod notification_store_test;
mod operator_ask_test;
mod permission_elevation_test;
mod permission_workflow_test;
mod plugin_lifecycle_test;
mod published_state_test;
mod response_stop_test;
mod seal_parity_test;
mod security_forging_test;
mod sse_streaming_test;
mod system_loop_test;
mod tool_hint_access_test;
mod tool_rejection_smoke;
