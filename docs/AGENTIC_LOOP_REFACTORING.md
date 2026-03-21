# Agentic Loop Refactoring Plan

## Status: Planned (Phase 2-3)

Phase 1 (ask_agent mini loop) is implemented. This document outlines the remaining refactoring phases.

## Current Architecture

The agentic loop is embedded directly in `handlers/system.rs::run_agentic_loop()` (~300 lines). It is tightly coupled with:

- Memory recall/store (CPersona MCP)
- Chat persistence (SQLite)
- Auto-speak (output.avatar TTS)
- Command approval gate (PendingApprovals, SessionTrustedCommands)
- Engine selection (CFR, escalation, fallback)
- Event emission (ToolInvoked, AgentThinking, AgenticLoopCompleted)

A second, smaller agentic loop now exists in `managers/mcp_kernel_tool.rs::run_ask_agent_loop()` for `ask_agent` inter-agent delegation. This creates code duplication in:

- ThinkResult parsing (`parse_think_result` vs `parse_mcp_think_result`)
- Tool execution and result formatting
- Tool history management
- Anti-spoofing injection

## Target Architecture

### Phase 2: Extract AgenticLoopRunner

Create a reusable component:

```rust
// crates/core/src/agentic_loop.rs (new module)

pub struct AgenticLoopRunner {
    mcp: Arc<McpClientManager>,
    pool: SqlitePool,
    event_tx: mpsc::Sender<EnvelopedEvent>,
}

pub struct AgenticLoopOptions {
    pub max_iterations: u8,           // SystemHandler: 16, ask_agent: 5
    pub tool_timeout_secs: u64,       // default: 30
    pub command_approval: bool,       // SystemHandler: true, ask_agent: depends
    pub emit_tool_events: bool,       // both: true
    pub agent_id_injection: String,   // anti-spoofing: which agent_id to inject
}

pub struct AgenticLoopResult {
    pub content: String,
    pub total_iterations: u8,
    pub total_tool_calls: u32,
}

impl AgenticLoopRunner {
    pub async fn run(
        &self,
        agent: &AgentMetadata,
        engine_id: &str,
        message: &Value,        // think_with_tools args
        tools: &[Value],
        options: AgenticLoopOptions,
    ) -> anyhow::Result<AgenticLoopResult>
}
```

### Phase 3: Migrate SystemHandler

1. Replace `run_agentic_loop()` in `system.rs` with `AgenticLoopRunner::run()`
2. Keep SystemHandler responsible for:
   - Memory recall/store (before/after loop)
   - Chat persistence (after loop)
   - Auto-speak (after loop)
   - ThoughtResponse emission (after loop)
   - Engine selection (before loop)
3. Remove duplicated `parse_think_result` from `mcp_kernel_tool.rs`
4. Remove `run_ask_agent_loop` from `mcp_kernel_tool.rs`, replace with `AgenticLoopRunner::run()`

## Migration Steps

1. Create `crates/core/src/agentic_loop.rs`
2. Move `parse_think_result` to the new module (shared)
3. Implement `AgenticLoopRunner::run()` based on existing `run_agentic_loop`
4. Update `ask_agent` to use `AgenticLoopRunner`
5. Update `SystemHandler` to use `AgenticLoopRunner`
6. Remove old duplicated code
7. Add unit tests for `AgenticLoopRunner`

## Risks

- SystemHandler's loop has edge cases (CFR tiers, escalation, consensus) that must be preserved
- Command approval gate requires access to `PendingApprovals` and `SessionTrustedCommands`
- Auto-speak side-effects are timing-sensitive
- Extensive regression testing needed

## Files Affected

- `crates/core/src/agentic_loop.rs` (new)
- `crates/core/src/handlers/system.rs` (major refactor)
- `crates/core/src/managers/mcp_kernel_tool.rs` (simplify)
- `crates/core/src/lib.rs` (add module)
