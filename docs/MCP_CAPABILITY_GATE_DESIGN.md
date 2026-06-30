# MCP Capability Gate — Unified Per-Agent Access Enforcement

Status: **Proposed** (design only; not yet implemented)
Scope: ClotoCore kernel access control (`crates/core`)
Related bugs: bug-419 (consensus engines), bug-420 (tool_hint bypass), engine-not-gated (LM Studio repro)

## 1. Problem

Per-agent access control in ClotoCore is enforced at **several scattered
execution sites with inconsistent logic** — some gated, some not. The
permission *data model* is already unified (`mcp_access_control` +
`resolve_tool_access`), but *enforcement* is fragmented, which makes the
effective policy **non-deterministic** (whether an agent may use a server
depends on which code path reaches the server) and **redundant** (the same
"can agent A use server S?" decision is re-implemented per call site).

Observed consequences:

- **Engine is never access-checked.** An agent's reasoning engine
  (`default_engine_id`, a `mind.*` MCP server) is resolved globally via
  `get_engine` / `has_server` with no grant check, so an agent with **zero
  grants still runs its engine and responds** (LM Studio repro). Engines are
  MCP servers but are treated as outside access control.
- **tool_hint bypass** (bug-420, point-fixed): the bridge-supplied direct-tool
  path called `execute_tool_internal` without a grant check.
- **Consensus** (bug-419, point-fixed): ran a global engine list under every
  agent regardless of the agent's granted engines.
- **Delegation** (`run_mgp_agent_ask_loop`): executes engine-requested tools via
  `call_server_tool` with no per-agent check.
- **Empty-grant branch**: the agentic loop falls back to the ungated
  `registry.execute_tool` when `agent_plugin_ids` is empty.

### Current enforcement sites (the fragmentation)

| Path | Current check | State |
| --- | --- | --- |
| Agentic loop — tool presentation | `collect_tool_schemas_for_agent` per-tool `resolve_tool_access` | gate (presentation) |
| Agentic loop — membership | `tool_names.contains` | gate (depends on presented set) |
| Agentic loop — execution | `execute_tool_for_agent` → `check_tool_access` | gate |
| Agentic loop — empty-grant branch | `registry.execute_tool` | **ungated** |
| tool_hint direct execution | (bug-420) `check_tool_access` added | bolt-on gate |
| Delegation (`run_mgp_agent_ask_loop`) | `call_server_tool` direct | **ungated** |
| Engine think / think_with_tools | `get_engine` / `has_server` (global) | **ungated** |
| `/api/mcp/call` | auth only | separate axis |

## 2. Key insight: the data model is already unified

- All capabilities — reasoning **engines** (`mind.*`), **tools**, **memory**
  servers — are MCP servers registered in `mcp_servers` and grantable through
  `mcp_access_control` (`server_grant` / `tool_grant`) with a per-server
  `default_policy`.
- `resolve_tool_access(agent_id, server_id, tool_name)` already resolves the
  full 3-level precedence (`tool_grant > server_grant > default_policy`) **by
  `server_id`** — so it works uniformly for engines, tools, and memory. (Note:
  `check_tool_access` resolves the server *via the global tool_index*, which
  does **not** index `mind.*` engine-internal tools; `resolve_tool_access` takes
  `server_id` directly and therefore covers engines too.)

So unification requires **no new permission concept**. "Engine config" is not a
separate permission system; an engine is just an MCP server that today is not
enforced. The fix is to route **all** execution through one gate that consults
the existing `resolve_tool_access`.

## 3. Design: a single deterministic chokepoint

All execution — tools, engine `think` / `think_with_tools`, delegation,
tool_hint, `/api/mcp/call` — ultimately flows through
`McpClientManager::call_server_tool` (and its streaming sibling
`call_server_tool_streaming`). These are the **single lowest chokepoint** and,
critically, they already hold the resolved `server_id` — the exact key
`resolve_tool_access` needs.

### 3.1 Caller identity

Thread a caller identity into the chokepoint:

```rust
enum Caller {
    /// Subject to per-agent access control.
    Agent(String),         // agent_id
    /// Trusted internal caller — bypasses the grant gate.
    /// (kernel-native tools, the consensus synthesizer, coordinator /api callers)
    System,
}

async fn call_server_tool(
    &self,
    caller: Caller,
    server_id: &str,
    tool_name: &str,
    args: Value,
) -> Result<Value, ToolFailure>;
```

Gate (the only enforcement of the **grant/access** axis):

```
match caller {
    Caller::Agent(id) => match resolve_tool_access(id, server_id, tool_name) {
        Allow   => proceed,
        Deny    => Err(Rejection::access_denied),
        Err(e)  => Err(Rejection::access_check_failed),
    },
    Caller::System => proceed,   // trusted internal
}
```

Because the gate keys on `server_id`, **engines are covered for free**: engine
execution is routed with `Caller::Agent(agent.id)` and the agent's
`default_engine_id` (a `mind.*` server) is checked via `server_grant` /
`default_policy` exactly like any other server. An agent with no engine grant
no longer responds.

### 3.2 Orthogonal axes stay distinct (policy vs data)

The grant/access axis is **data** (DB rows). The MCP permission model has other,
**orthogonal** axes that must be *composed* at the chokepoint as distinct
ordered stages — **not flattened into the grant check** (mixing policy into the
access-data resolution is the same anti-pattern as baking routing into a data
layer):

| Axis | Question | Nature |
| --- | --- | --- |
| grant / access (`resolve_tool_access`) | May this agent use this server? | **data** — unified here |
| YOLO / SafetyGate | Is this operation dangerous (agent-independent)? | policy |
| Approval (`pending_approvals`) | Does a human need to confirm? | policy (human-in-loop) |
| Delegation intersection | Effective caller = caller ∩ target | caller transform |

The chokepoint becomes a single deterministic **ordered pipeline** with **one
audit point** — but each stage remains a separate evaluator.

### 3.3 What stays

Presentation-layer filtering (`collect_tool_schemas_for_agent`) is **kept** for
UX / token budget (which tools the LLM is shown), but it is **no longer the
enforcement boundary** — "shown" and "allowed" are decoupled and enforcement is
single-sourced.

## 4. Redundancy removed

Once the chokepoint enforces uniformly, these collapse:

- `execute_tool_for_agent`'s inline `check_tool_access` branch → moves to the gate.
- The agentic loop's `if agent_plugin_ids.is_empty() { execute_tool (ungated) } else { execute_tool_for_agent }` two-branch → single path.
- The bolt-on `check_tool_access` in the tool_hint path (bug-420) → **removed** (subsumed by the central gate; the point-fix was interim).
- `tool_names` membership used as *enforcement* → demoted to presentation only.
- Delegation loop's unchecked execution → covered by the same gate.
- Engine resolution's missing check → covered by the same gate.

## 5. Caller threading & exemptions

- **Agentic loop / normal message path / tool_hint / consensus engines**:
  `Caller::Agent(agent.id)`.
- **Consensus synthesizer** (`agent.synthesizer`, system agent): `Caller::System`
  (it is a kernel-internal merge step, not a user agent).
- **Kernel-native tools** (`mgp.*`, `gui.*`): keep the existing `server_id="kernel"`
  RBAC (Deny-only) — represented as a `System`/kernel path or a dedicated kernel
  gate; do not route through `resolve_tool_access` server grants.
- **`/api/mcp/call` coordinator endpoint**: already auth-gated; map the
  authenticated caller to `Caller::System` (or a coordinator agent id if one is
  carried), preserving today's behavior.

## 6. Open decision: `default_policy = opt-out`

Migration `20260301000000_default_policy_opt_out.sql` set all servers'
`default_policy` to `opt-out` (allow-all). With that default, **revoking a grant
(deleting the `server_grant` row) falls back to allow** — so revoke does not
actually deny. This is a **policy default**, separate from the enforcement
unification. To make revoke effective, choose one:

- (a) Engines / sensitive servers default to `opt-in` (deny-by-default), or
- (b) "Reject" writes an explicit `Deny` entry rather than deleting the grant, or
- (c) Both.

This decision is independent of the chokepoint work and should be made
deliberately (it changes existing deployments' effective access).

## 7. Rollout / behavior change

Enforcing the engine grant is a **behavior change**: an agent with no granted
engine stops responding. Agents created through SetupWizard always grant their
chosen engine, so well-formed agents are unaffected; agents created by other
paths (tests, raw API) that set `default_engine_id` without a grant will need a
grant. Consider a one-time backfill (grant each agent's current
`default_engine_id` as a `server_grant`) and/or a clear "no engine assigned"
error surfaced to chat.

## 8. Test plan

- Unit: the gate denies `Caller::Agent` for an ungranted server (engine, tool,
  memory) and allows `Caller::System`.
- Integration: a zero-grant agent cannot get an engine response (the LM Studio
  repro), but the same agent with its engine granted responds.
- Regression: bug-419 / bug-420 scenarios remain closed via the single gate.
- Delegation: a delegated agent cannot execute tools on servers it is not granted.

## 9. Relation to the point-fixes

bug-419 (consensus engine sourcing) and bug-420 (tool_hint gate) are already
fixed as targeted patches. This chokepoint **subsumes and simplifies** them: the
tool_hint inline check is removed, and consensus engine filtering keeps using
the agent's granted engine servers as its source. The point-fixes are safe
interim measures; the chokepoint is the durable, deterministic design.
