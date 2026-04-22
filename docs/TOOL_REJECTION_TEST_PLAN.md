# Tool Rejection Test Plan (Phase 0)

**Status:** Active — ground truth for Phase A–F. Archive after Phase F lands.
**Scope:** Structured `ToolFailure::Rejection` protocol (Option C). Covers YOLO-gated tool migration and delegation rejection handling.

This document defines the **exact behavior** expected from each phase before implementation begins. Each subsequent phase is accepted only when the relevant section here is satisfied.

---

## 1. Rejection Code Matrix

All `RejectionCode` variants and their trigger sites. The **Phase** column indicates when migration to `ToolFailure::Rejection` happens. Codes marked **deferred** keep their current behavior until Phase G (separate issue).

| Code | Phase | Trigger site | Retryable | Details payload |
|---|---|---|---|---|
| `YoloRequired` | B | `execute_create_mcp_server` (mcp_kernel_tool.rs:263) | `true` | `None` |
| `YoloRequired` | B | `execute_access_query` (mcp_kernel_tool.rs:513) | `true` | `None` |
| `YoloRequired` | B | `execute_access_grant` (mcp_kernel_tool.rs:569) | `true` | `None` |
| `YoloRequired` | B | `execute_access_revoke` (mcp_kernel_tool.rs:654) | `true` | `None` |
| `YoloRequired` | B | `execute_audit_replay` (mcp_kernel_tool.rs:715) | `true` | `None` |
| `YoloRequired` | B | `execute_mgp_agent_ask` (mcp_kernel_tool.rs:1290) | `true` | `None` |
| `YoloRequired` | B | `execute_discovery_register` (mcp_discovery.rs:247) | `true` | `None` |
| `SelfDelegation` | B | `execute_mgp_agent_ask` (mcp_kernel_tool.rs:1319) | `false` | `None` |
| `DelegationDepth` | B | `execute_mgp_agent_ask` (mcp_kernel_tool.rs:1332) | `false` | `{"chain": [agent_id, ...]}` |
| `DelegationCycle` | B | `execute_mgp_agent_ask` (mcp_kernel_tool.rs:1345) | `false` | `{"chain": [agent_id, ...]}` |
| `AccessDenied` | deferred (G) | `registry.rs:341-346` (MCP access-control Deny) | `true` | `None` |
| `SealUnsigned` | deferred (G) | `mcp.rs:910, 923-929` (Seal verification) | `true` | `None` |
| `CodeUnsafe` | deferred (G) | `execute_create_mcp_server` validator (mcp_kernel_tool.rs:309) | `true` | `{"violations": [...]}` |
| `RiskUnapproved` | dormant | (no enforcer exists) | `true` | — |
| `Unknown` | E | External MCP server response w/ `isError:true` + sentinel | `true` | `None` |

**Total Phase B migration sites: 10** (7 YoloRequired + 3 delegation).

---

## 2. Reason & Remediation Text

All texts are self-contained English that does **not** depend on the LLM understanding the code name (e.g., "YOLO"). Keywords like "privileged mode", "currently disabled", "will reject identical" are designed to guide even weak local models (< 14B params).

### `YoloRequired`
- **reason:** `"This tool is restricted to privileged (YOLO) mode, which is currently disabled by the operator. The kernel will reject identical requests until the operator re-enables privileged mode in the dashboard."`
- **remediation_hint:** `"Ask the operator to enable YOLO mode in Settings → Security."`

### `SelfDelegation`
- **reason:** `"Delegation target and caller are the same agent. An agent cannot delegate to itself; this is a hard logical constraint enforced by the kernel."`
- **remediation_hint:** `None` (no user action fixes this)

### `DelegationDepth`
- **reason:** `"Inter-agent delegation chain exceeded the maximum depth limit. This is a hard logical constraint enforced by the kernel to prevent runaway delegation."`
- **remediation_hint:** `None` (redesign required at agent logic level)

### `DelegationCycle`
- **reason:** `"Inter-agent delegation would form a cycle — the target agent is already in the current delegation chain. This is a hard logical constraint enforced by the kernel."`
- **remediation_hint:** `None`

### `AccessDenied` (Phase G — template reserved)
- **reason:** `"Access to this tool is denied by the access control policy. The operator has explicitly restricted this agent from using this tool."`
- **remediation_hint:** `"Ask the operator to grant access in the Agent Workspace."`

### `SealUnsigned` (Phase G — template reserved)
- **reason:** `"The MCP server providing this tool is not signed and cannot execute at the current trust level. The kernel blocks unsigned servers when trust level is elevated."`
- **remediation_hint:** `"Ask the operator to sign the MCP server or lower its trust level."`

### `CodeUnsafe` (Phase G — template reserved)
- **reason:** `"Generated MCP server code failed safety validation. The kernel refuses to execute code containing blocked imports or dangerous patterns."`
- **remediation_hint:** `"Review the violations in the details payload and regenerate the code without the flagged patterns."`

### `RiskUnapproved` (dormant — template reserved)
- **reason:** `"This tool is classified as dangerous and has not been approved for autonomous execution."`
- **remediation_hint:** `"Ask the operator to approve the risk level in the dashboard."`

### `Unknown` (Phase E — external MCP server rejection)
- **reason:** `<content text from external server>` (passed through)
- **remediation_hint:** `None` (kernel cannot infer)

---

## 3. Agentic Loop Behavior

### 3.1 tool_history injection format (Phase C)

When a tool call returns `Err(ToolFailure::Rejection(r))`, the agentic loop pushes this entry to `tool_history`:

```
{
  "role": "tool",
  "tool_call_id": "<call.id>",
  "content": "Error: <r.reason>\nREMEDIATION: <r.remediation_hint or 'This rejection cannot be resolved by operator action.'>\nDo not retry this tool call; report the situation to the user."
}
```

The trailing "Do not retry this tool call" directive is **mandatory**. It shapes LLM behavior even when the model doesn't understand the specific code.

### 3.2 Break conditions

The agentic loop exits early (before `MAX_ITERATIONS`) when:

1. **Consecutive same-code rule (β):** If the most recent 2 tool calls both return `ToolFailure::Rejection` with the same `RejectionCode`, break.
2. **Hard rejection rule:** If any tool call returns `ToolFailure::Rejection` with `retryable: false`, break immediately after that call.

Break precedence: hard rule fires first (even on the first rejection), consecutive rule is a fallback for retryable codes.

### 3.3 Mechanical final response (Phase C)

When the loop breaks due to rejection, the kernel **does not call the LLM for the final turn**. Instead, it synthesizes a fixed template per-code. Templates use `{tools}` for comma-joined attempted tool names.

| Code | Template |
|---|---|
| `YoloRequired` | `"The requested operation (tool(s): {tools}) was rejected because privileged (YOLO) mode is currently disabled. Please enable it in Settings → Security to allow these operations."` |
| `SelfDelegation` | `"Inter-agent delegation was aborted: the delegation target is the same as the caller. This is a logical constraint and cannot be resolved."` |
| `DelegationDepth` | `"Inter-agent delegation was aborted: the delegation chain exceeded the maximum depth. This is a logical constraint — the agent logic needs to be redesigned to avoid deep delegation."` |
| `DelegationCycle` | `"Inter-agent delegation was aborted: the requested chain would form a cycle. This is a logical constraint and cannot be resolved without changing the delegation target."` |
| `AccessDenied` | `"Access to tool(s) {tools} was denied by the access control policy. Please review the agent's permissions in the Agent Workspace."` |
| `SealUnsigned` | `"Tool(s) {tools} could not be executed because the providing MCP server is not signed at the required trust level. Please sign the server or lower its trust level."` |
| `CodeUnsafe` | `"Generated MCP server code failed safety validation. Please review the violations reported in the rejection details."` |
| `RiskUnapproved` | `"Tool(s) {tools} are classified as dangerous and have not been approved for autonomous execution. Please approve them in the dashboard."` |
| `Unknown` | `"Tool(s) {tools} were rejected by the MCP server. Reason: {reason_of_last_rejection}"` |

The mechanical response is emitted as `ClotoEventData::AgentFinalResponse { content: <template>, agent_id, trace_id, ... }` — identical envelope to LLM-generated responses.

### 3.4 SSE event order (golden sequence)

#### Normal iteration (success)
```
AgentThinking (optional)
ToolInvoked { success: true, ... }
```

#### Rejected iteration
```
AgentThinking (optional)
ToolInvoked { success: false, ... }
ToolRejected { code, reason, remediation_hint, iteration, ... }
```

#### Break by consecutive same-code (2 rejected iterations)
```
iter 1: AgentThinking → ToolInvoked(false) → ToolRejected(YoloRequired)
iter 2: AgentThinking → ToolInvoked(false) → ToolRejected(YoloRequired)  // same code → break
AgentFinalResponse { content: "<mechanical template>", ... }
```

#### Break by hard rejection (1 iteration)
```
iter 1: AgentThinking → ToolInvoked(false) → ToolRejected(DelegationCycle, retryable:false)  // → break
AgentFinalResponse { content: "<mechanical template>", ... }
```

---

## 4. Audit Log

New event_type `TOOL_REJECTED` added in Phase C. Schema:

```json
{
  "event_type": "TOOL_REJECTED",
  "actor_id": "<agent_id>",
  "target_id": "<tool_name>",
  "permission": null,
  "result": "rejected",
  "reason": "<code>: <reason>",
  "metadata": {
    "call_id": "<...>",
    "code": "<RejectionCode>",
    "iteration": <u8>,
    "retryable": <bool>,
    "details": <json or null>
  },
  "trace_id": "<...>"
}
```

---

## 5. Test Cases (must be green before merge)

### Phase A unit tests (shared)
- [ ] `ToolRejection` serde round-trip (all 9 codes)
- [ ] `From<anyhow::Error> for ToolFailure` produces `ToolFailure::Error` variant
- [ ] `ClotoEventData::ToolRejected` serializes to expected JSON shape
- [ ] `ToolFailure` exhaustive match compiles (guards against enum drift)

### Phase B unit tests (kernel)
- [ ] `execute_create_mcp_server` + YOLO OFF → `Err(ToolFailure::Rejection(YoloRequired, retryable:true))`
- [ ] `execute_create_mcp_server` + YOLO ON + valid args → `Ok(Value)` (existing happy path unchanged)
- [ ] `execute_access_query` / `execute_access_grant` / `execute_access_revoke` + YOLO OFF → `YoloRequired`
- [ ] `execute_audit_replay` + YOLO OFF → `YoloRequired`
- [ ] `execute_mgp_agent_ask` + YOLO OFF → `YoloRequired`
- [ ] `execute_mgp_agent_ask` + self-target → `SelfDelegation, retryable:false`
- [ ] `execute_mgp_agent_ask` + depth >= MAX → `DelegationDepth, retryable:false, details:{chain}`
- [ ] `execute_mgp_agent_ask` + target in chain → `DelegationCycle, retryable:false, details:{chain}`
- [ ] `execute_discovery_register` + YOLO OFF → `YoloRequired`
- [ ] `execute_tool_for_agent` passes `ToolFailure::Error` through unchanged for existing errors (compatibility)

### Phase C agentic loop tests (handlers/system.rs)
- [ ] Single rejection (retryable:true) → tool_history gets "Error: ... Do not retry ..." entry, no break
- [ ] 2 consecutive same-code rejections → break triggered, AgentFinalResponse emitted with correct template
- [ ] 2 rejections with different codes → no break (continues to MAX_ITERATIONS or success)
- [ ] 1 retryable:false rejection → immediate break, AgentFinalResponse emitted
- [ ] Break response content matches template exactly for each of 9 codes
- [ ] `TOOL_REJECTED` audit log row created for each rejection
- [ ] `ToolInvoked { success: false }` AND `ToolRejected` both emitted per rejected call
- [ ] Rejection does not corrupt tool_history drain / MAX_TOOL_HISTORY behavior

### Phase D dashboard tests
- [ ] `biome check src/` passes
- [ ] `npm run build` succeeds
- [ ] Smoke test: YOLO OFF + trigger rejection → `ToolRejectionCard` visible with correct reason + hint
- [ ] Dismiss button removes card from DOM without errors
- [ ] `compiled-tailwind.css` regenerated (check mtime after run)
- [ ] Multiple concurrent rejections render as stacked cards (each dismissable independently)

### Phase E fallback tests (external MCP)
- [ ] Mock MCP server returns `CallToolResult { isError: true, content: [{text: "{\"status\":\"rejected\",\"reason\":\"...\"}"}] }` → promoted to `ToolFailure::Rejection { code: Unknown, reason: <text>, retryable: true }`
- [ ] Mock MCP server returns `CallToolResult { isError: true, content: [{text: "plain error"}] }` (no sentinel) → `ToolFailure::Error` (existing behavior)
- [ ] Mock MCP server returns `CallToolResult { isError: false }` with `status:"rejected"` in content → **not promoted** (isError:false takes precedence)

### Phase F (cloto-mcp-servers) docs tests
- [ ] `MGP_SPEC.md` §14.7 passes repo's markdown linter (if any)
- [ ] JSON Schema example in §14.7 validates against actual `ToolRejection` serde output

---

## 6. Per-Phase Acceptance Checklist

Copy this into each phase's PR description:

```
□ cargo clippy --workspace --exclude app --all-targets -- -D warnings
□ cargo fmt --all -- --check
□ cargo test --workspace --exclude app
□ cd dashboard && npx biome check src/
□ cd dashboard && npm run build
□ bash scripts/check-test-count.sh
□ bash scripts/verify-issues.sh
□ Phase-specific test cases in §5 above
□ No DB migration added (Phase A–F are pure Rust type + UI changes)
□ No version bump (awaiting explicit user instruction)
□ Commit message uses conventional format with "(Phase X)" suffix
```

---

## 7. End-to-End Smoke Test (post-Phase D)

1. `cd ClotoCore/dashboard && npx tauri dev` with YOLO OFF
2. Create/select an agent, instruct it: "Please grant my agent access to the file MCP server"
3. Agent calls `mgp.access.grant` → rejection fires
4. Observe in dashboard:
   - Tool call row shows red/failed indicator (not green checkmark)
   - `ToolRejectionCard` appears with text containing "privileged (YOLO) mode" and "currently disabled"
   - Card has a dismiss button
5. Agent's final message contains the mechanical template starting with "The requested operation"
6. Click dismiss → card disappears, no console errors
7. Repeat instruction → 2nd rejection → loop breaks after 2nd iteration (not MAX_ITERATIONS)
8. Query `sqlite3 target/debug/data/cloto_memories.db "SELECT event_type, reason FROM audit_log WHERE event_type='TOOL_REJECTED' ORDER BY id DESC LIMIT 5"` → 2 rows present

---

## 8. Archive Policy

This doc lives in `docs/` during the Phase A–F migration. Once Phase F lands (MGP_SPEC §14.7 draft accepted), archive this file to `docs/archive/TOOL_REJECTION_TEST_PLAN_<phase-f-merge-date>.md` and remove from active docs.
