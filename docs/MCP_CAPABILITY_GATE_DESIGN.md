# MCP Capability Gate — Unified Per-Agent Access Enforcement

Status: **Accepted** — implementing (§6 policy decided 2026-06-30: global opt-in)
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

## 3. Design: a shared gate at the (two) execution chokepoints

> **Correction (2026-07-01 investigation, code-verified):** there is **no
> single** lowest chokepoint. Exactly **three** transport call sites reach
> `McpClient::call_tool[_streaming]`, funneling into **two** manager-level
> paths:
> - **PATH 1** — `call_server_tool` (`mcp.rs:2191`) + `call_server_tool_streaming`
>   (`mcp.rs:2218`), both via `resolve_tool_call_target` (`mcp.rs:2053`).
> - **PATH 2** — `execute_tool_internal` (`mcp.rs:1808` → `client.call_tool` at
>   `mcp.rs:1951`), an independent sibling that does **not** go through
>   `call_server_tool` or `resolve_tool_call_target`.
>
> Critically, the **main agentic loop tool execution** (`system.rs:2516-2527`)
> and **tool_hint** use PATH 2. A gate only on `call_server_tool` would miss the
> highest-value agent paths. The fix is therefore a single shared gate function
> installed at **both** chokepoints (see §3.1), not one point.

Both paths already hold the resolved `server_id` — the exact key
`resolve_tool_access` needs — so a uniform gate is still achievable; it just
needs two insertion points sharing one enforcement function.

The shared gate `enforce_caller_grant(caller, server_id, tool_name)` is installed:

- **PATH 1**: at `resolve_tool_call_target` entry. `server_id` is final on entry
  (no rewrite occurs across `mcp.rs:2053-2179`), so it covers streaming and
  non-streaming identically. The §5.6.1 delegation intersection
  (`mcp.rs:2099-2120`) remains a **separate ordered stage after** the caller gate.
- **PATH 2**: inside `execute_tool_internal`, **after** the kernel-native match
  (`mcp.rs:1814-1898`) and `server_id` resolution (`mcp.rs:1901-1922`) but
  **before** the validator (`mcp.rs:1925`) / `call_tool` (`mcp.rs:1951`). This
  preserves the kernel-native early-return, the TOOL_EXECUTED / TOOL_ERROR /
  TOOL_BLOCKED audit (`mcp.rs:1927-1987`), and the `CallToolResult → Value`
  flattening that `call_server_tool` lacks. (Re-pointing PATH 2 onto
  `call_server_tool` is therefore **rejected** — it would lose audit + flattening.)

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

- **Agentic loop / normal message path / tool_hint / consensus proposal engines**:
  `Caller::Agent(agent.id)`. Derive once from the in-scope `AgentMetadata`.
- **Consensus synthesizer**: `Caller::System`. **Discriminator MUST be
  `agent.agent_type == "system"`** (`system.rs:1929`), **not** the id — the
  synthesizer's `id` is `"agent.synthesizer"` (`system.rs:1920`) while
  `cfg.synthetic_agent_id` is `"system.consensus"` (`consensus.rs:23`, used only
  for the session_key at `system.rs:1936`); they do not match, so keying on an id
  would misclassify.
- **Kernel-native tools** (`mgp.*`, `gui.*`): keep the dedicated
  `server_id="kernel"` RBAC, evaluated on the kernel-native early-return path
  **before** the central gate; do not route through `resolve_tool_access` server
  grants. **Decision 2026-07-01: the kernel gate special-cases the default
  to Allow (Deny-only RBAC).** This fixes a **confirmed live defect** — the live
  `kernel` row is `opt-in`, so `resolve_tool_access(...,"kernel",...)` currently
  returns Deny for every agent without an explicit kernel grant; today only
  `agent.cloto_default` can use `mgp.agent.ask`. No migration row is added (the
  global opt-in flip in §6 must **not** make kernel deny-by-default for everyone).
- **`/api/mcp/call` coordinator endpoint** (`handlers/mcp.rs:877/889`): already
  admin-auth-gated; map to `Caller::System`. Per-agent scoping for this endpoint
  flows only through `_mgp.delegation` (`original_actor`), not a body agent_id.
- **switch_model relay** (`handlers/llm.rs:119`), **ollama post-connect sync**
  (`mcp.rs:1480`), **DeleteAgentData** (`agents.rs:339`): `Caller::System`.
- **Background memory ops** (`system.rs:703/1113/1294/1506/3550+`):
  `Caller::Agent(agent_id)`.
- **Default-proceed (AI, 2026-07-01, reversible by a later override)**: media preprocessing
  (`AnalyzeImage` `system.rs:3092`, `Transcribe` `system.rs:3202`) → `System`
  (kernel preprocessing, not agent-initiated); episode-archival summarizer
  (`system.rs:3863`) → `Agent(message-agent)`.

## 6. Decision: `default_policy = opt-in` everywhere (deny-by-default)

Migration `20260301000000_default_policy_opt_out.sql` had set all servers'
`default_policy` to `opt-out` (allow-all). With that default, **revoking a grant
(deleting the `server_grant` row) falls back to allow** — so revoke does not
actually deny, and an agent that was *never* granted an engine still runs it
(the LM Studio repro). This is a **policy default**, separate from the
enforcement unification.

**Decision 2026-06-30: flip every server back to `opt-in`
(deny-by-default) globally.** Rationale — the simplest, most predictable mental
model: `grant ⇒ allow`, `no grant ⇒ deny`, `revoke = delete ⇒ deny` falls out
for free. This makes both the zero-grant case (LM Studio) and the
revoke-staleness case deny correctly with no special-casing.

The accepted cost is the **largest blast radius**: every server an agent was
implicitly relying on under `opt-out` now denies unless it has an explicit
`server_grant`. This **mandates a comprehensive backfill** (§7) so well-formed
agents — those configured through SetupWizard / AgentConsole, which already
write `server_grant` rows — keep working. Agents with **no** matching grant
(the bug being fixed) correctly lose access.

Considered and rejected: (a) opt-in for engines only — leaves non-engine
revoke-staleness under opt-out; (b) keep opt-out, write an explicit `Deny` on
revoke — does not fix the zero-grant (never-granted) engine case, so the
headline bug survives. The global opt-in flip subsumes both.

## 7. Rollout / behavior change

Two coupled behavior changes ship together:

1. **The gate** enforces `resolve_tool_access` at the chokepoint for every
   `Caller::Agent` path (engine, tool, delegation, tool_hint).
2. **The opt-in flip** (§6) makes any server without an explicit grant deny.

Net effect: an agent with no granted engine stops responding, and an agent can
no longer reach a server it was never granted. Agents created through
SetupWizard / AgentConsole already write `server_grant` rows for their engine
and assigned servers, so well-formed agents are unaffected.

**Real blast radius (code-verified):** the flip is small in live data — only
`github-bridge`, `mind.local`, `x-browser` are `opt-out` (3/18). `20260301000000`
ran its `UPDATE ... WHERE default_policy='opt-in'` against a near-empty table, and
servers registered afterward already default to `opt-in` (`db/mcp.rs:128`). The
flip's real value is making revoke-staleness deny correctly + converting
implicit engine access to explicit, auditable grants.

**Migration (one file, timestamp after the real tail `20260524010000`; CRLF-convert
before any build per the SQLx rule).** Hard constraints (verified):
`mcp_access_control.server_id REFERENCES mcp_servers(name)` and the kernel opens
SQLite with `PRAGMA foreign_keys=ON` (`lib.rs:442-447`) — a dangling-engine INSERT
**aborts the migration → FATAL boot**, so the `EXISTS(mcp_servers)` guard is
mandatory; `granted_at` is `NOT NULL` with no default and must be supplied.

1. **Global opt-in flip:** `UPDATE mcp_servers SET default_policy='opt-in' WHERE
   default_policy='opt-out'`. Will **not** touch the `kernel` row (already opt-in;
   its default is special-cased in code per §5).
2. **Engine backfill (FK-safe, non-clobbering):** grant each agent's
   `default_engine_id` as a `server_grant` **only** where the engine row exists
   and no prior server_grant exists:
   ```sql
   INSERT INTO mcp_access_control (entry_type, agent_id, server_id, permission, granted_by, granted_at)
   SELECT 'server_grant', a.id, a.default_engine_id, 'allow', 'migration:capability-gate', datetime('now')
   FROM agents a
   WHERE a.default_engine_id IS NOT NULL AND a.default_engine_id != ''
     AND a.id != 'agent.テスト用'                                    -- §decision 3: repro agent excluded
     AND EXISTS (SELECT 1 FROM mcp_servers s WHERE s.name = a.default_engine_id)
     AND NOT EXISTS (SELECT 1 FROM mcp_access_control ac
                     WHERE ac.agent_id = a.id AND ac.server_id = a.default_engine_id
                       AND ac.entry_type = 'server_grant' AND ac.tool_name IS NULL);
   ```
   The `NOT EXISTS` also matches `permission='deny'` rows, so existing explicit
   DENYs (e.g. `cpersona_bench`) are preserved.

**Decisions 2026-07-01 — strict enforcement, consistent with global opt-in:**

- **Engine backfill grants only `default_engine_id`** (not routing /
  `escalate_to` / `fallback` / `engine_override` — 0 live rows; reached engines
  surface the "engine not granted" error below). It deliberately grants only what
  an agent demonstrably owns; it does **not** re-grant every active server.
- **Repro agent `agent.テスト用` ("for testing") is EXCLUDED** from the backfill (the `a.id !=`
  guard above), so it is genuinely denied after the flip — this validates the
  reported "grant-0 agent still responds" bug on the real agent. (For all other
  agents the bug is fixed for future never-granted agents + revoke semantics.)
- **Opt-out tool servers `x-browser` / `github-bridge` are NOT preserved**
  (intentional drop). They lose implicit allow-all access; agents that need them
  must be re-granted explicitly.

**UX (required):** surface a clear chat error instead of a silent non-response —
**"no engine assigned"** when an agent's engine resolves to `Deny`, **and** a
per-tool **"tool not granted"** signal when a tool server resolves to `Deny`
(the latter is required because the tool-server drop above is otherwise invisible).

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
