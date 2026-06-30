# Consensus Revival — In-Kernel Proposal Generation (Design)

Status: **Accepted** (2026-06-30). an earlier decision / an earlier decision / an earlier decision.
Supersedes the reverted standalone bug-408 patch (`481279c`).

Ratified decisions (see §9): **(a) Option A** — kernel-driven synchronous
orchestration. **(b)** `ConsensusRequested` / `ThoughtRequested` enum variants
are **kept** in `ClotoEventData` (protocol stability) but no longer
produced/consumed on the consensus path. **(c)** proposal `agent_id` stamping
per §5.1 (`agent_id = agent.id`, distinguished by `engine_id`).

## 1. Motivation

Consensus is a multi-engine deliberation mode: a single user task is sent to N
LLM engines, each returns an independent *proposal*, and a designated
*synthesizer* engine merges them into one answer. It is a flagship "multiple
LLMs deliberating" capability and a marketing asset. Today it is **dormant** —
a consensus request collects nothing and silently times out.

This document specifies how to make consensus functional again by generating
proposals **in-kernel**, and shows why that redesign subsumes bug-408
(consensus aggregation race) as a correct-by-construction property rather than a
patch.

## 2. Current (dormant) architecture and the exact gap

### 2.1 What fires today

When an inbound message starts with the consensus prefix (default `consensus:`),
`handlers/system.rs` (`handle_message`, the consensus branch ~L758) emits:

1. one `ConsensusRequested { task, engine_ids }` envelope (depth 0), and
2. one `ThoughtRequested { agent, engine_id, message, context }` envelope per
   configured engine (depth 1, `correlation_id = trace_id`).

In `events.rs` the event loop then:

- routes `ConsensusRequested` → `ConsensusOrchestrator::handle_event` →
  `on_consensus_requested`, which inserts a `Collecting` session and returns
  `None`;
- routes each `ThoughtRequested` → `registry.dispatch_event` (MCP plugins only,
  empty set) → `consensus.handle_event` (its `match` has no `ThoughtRequested`
  arm → `None`) → the post-processing `match` in `events.rs` (no
  `ThoughtRequested` arm).

### 2.2 The gap

**Nothing consumes `ThoughtRequested`.** No component runs the engine for a
`ThoughtRequested` event and emits a matching `ThoughtResponse`. Historically
`plugins/moderator` (a Rust plugin) listened for `ThoughtRequested` in its
`on_event` and called the engine; that plugin layer was removed:

- `742ee61` — state machine absorbed into the kernel (`consensus.rs`),
- `dcc6820` — plugin layer + consensus dashboard UI trimmed,
- `207b39e` — consensus dropped from the advertised use cases.

The engine-running half of the contract was never re-homed. Consequently:

- proposals are never produced → `ConsensusOrchestrator`'s `on_thought_response`
  never fires → the `Collecting` session reaches `min_proposals` never →
- the background cleanup task evicts the session after
  `session_timeout_secs` (default 60s). The user sees nothing.

Note the asymmetry with the **normal** message path: a non-consensus message is
handled *synchronously inside* `handle_message`, which calls
`run_agentic_loop(...) -> anyhow::Result<String>` and then emits the
`ThoughtResponse` directly (system.rs ~L1254). The normal path never relies on
an external `ThoughtRequested` consumer. **Consensus is the only path that emits
work-request events expecting an external worker that no longer exists.**

## 3. Design goals and constraints

- **G1 — Functional consensus**: a `consensus:` message must collect proposals
  from ≥2 real engines, synthesize, and return one unified `ThoughtResponse`.
- **G2 — Reuse, don't fork**: proposals and synthesis must run through the same
  `run_agentic_loop` the normal path uses (same engine resolution, MCP/plugin
  fallback, tool access, error semantics). No second engine-invocation codepath.
- **G3 — Correct identity (subsumes bug-408)**: the kernel must stamp the
  `agent_id` on every proposal and on the synthesis, so identity is never
  inferred from event ordering.
- **G4 — Fail-safe**: an engine erroring, fewer than `min_proposals`
  succeeding, or the synthesizer failing/timing out must surface a definite
  result (error `ThoughtResponse`), never a silent 60s timeout.
- **G5 — No reference cycle**: `EventProcessor` already owns both the
  `SystemHandler` and the `ConsensusOrchestrator`; the design must not require
  the orchestrator to call back into the handler.
- **G6 — Additive default**: with `CONSENSUS_ENGINES` empty (current default),
  behavior is unchanged. Nothing turns on until a deployment configures engines.

## 4. Options considered

### Option A — Kernel-driven, synchronous orchestration (RECOMMENDED)

Move *all* engine execution into the `system.rs` consensus branch. The branch
becomes a self-contained async routine:

1. Resolve `consensus_engines`; if `< min_proposals` are configured, emit an
   error `ThoughtResponse` immediately (G4).
2. Run the N engines **concurrently** via `run_agentic_loop`, one task per
   engine, each bounded by a per-engine timeout (`tokio::select!` over the
   existing `session_timeout_secs`/a dedicated proposal timeout). Collect the
   `Ok(String)` proposals; log and drop `Err`/timed-out engines.
3. If `collected.len() < min_proposals` → error `ThoughtResponse` (G4).
4. Build the combined-views prompt and run the **synthesizer engine** in-kernel
   via the same `run_agentic_loop`, bounded by a synthesis timeout.
5. Emit exactly one final `ThoughtResponse` with `agent_id =
   synthetic_agent_id`, `engine_id = "consensus"`.

`ConsensusOrchestrator` (consensus.rs) is **retired as an event handler**. Its
value — the prompt-combination logic, `ConsensusConfig` (min_proposals,
synthesizer_engine, timeout, synthetic_agent_id) — is kept, either as a small
synchronous helper module or folded into the consensus branch. The
`ConsensusRequested` event and the per-engine `ThoughtRequested` emissions are
**deleted** (they have no consumer and no other producer).

- **bug-408 disposition**: the race ("a late proposal `ThoughtResponse` is
  mistaken for the synthesis result") **cannot occur** — collection and
  synthesis are sequential awaits in one task, not events matched by arrival
  order. The reverted guard (`481279c`, `synthesizer_agent_id` matching) is
  unnecessary; correctness is structural (G3).
- **Concurrency** (G2): `tokio::task::JoinSet` or `futures::join_all` over
  per-engine `run_agentic_loop` futures, each wrapped in `tokio::time::timeout`.
- **Trace/session** (an earlier decisionc): one `trace_id` for the whole consensus
  session; each proposal loop uses a per-engine derived `session_key` so engines
  don't share conversational state, while the synthesis loop uses its own.
- **Pros**: simplest data flow; no event-bus round trips; no reference cycle
  (G5); kernel stamps every `agent_id` (G3); fail-safe is plain `Result`
  handling (G4); deletes dead code.
- **Cons**: retires the event-driven `ConsensusOrchestrator`. (It is currently
  dead, so this removes liability rather than function.) Consensus no longer
  emits intermediate per-proposal events on the bus — if the dashboard wants to
  visualize each engine's proposal live (an earlier decision), the branch must emit
  explicit progress events for that purpose (see §7).

### Option B — Event-driven, add an in-kernel `ThoughtRequested` runner

Keep `ConsensusRequested` and the orchestrator. Add a kernel handler that
consumes `ThoughtRequested` events generally: run `run_agentic_loop`, emit a
`ThoughtResponse`. Now both the per-engine proposal requests (from system.rs)
and the synthesizer request (from the orchestrator's `NeedsSynthesis`) get
executed. The orchestrator stays event-driven; bug-408's `synthesizer_agent_id`
guard remains load-bearing.

- **Pros**: minimal change to consensus.rs; preserves the event-driven model; a
  general `ThoughtRequested → engine → ThoughtResponse` mechanism could be
  reused by other features.
- **Cons**: more moving parts and bus round trips; requires loop-guards so a
  proposal's `ThoughtResponse` is not re-dispatched as new work; bug-408 race is
  **real and must be guarded** (identity matching by `synthesizer_agent_id`)
  rather than eliminated; identity is split across two stampers (proposal runner
  vs orchestrator). Higher surface for the same outcome.

### Decision

**Option A.** It matches the direction recorded at goal creation (kernel
generates proposals and stamps `ThoughtResponse.agent_id` directly), eliminates
bug-408 instead of guarding it, avoids the reference cycle, and removes dead
code. Option B's only unique upside — a general `ThoughtRequested` worker — is
speculative (YAGNI); no other feature needs it today.

## 5. Recommended design in detail (Option A)

### 5.1 Agent-id stamping convention (an earlier decisionb)

- **Proposal**: stamped with the proposing engine's identity. Use the inbound
  `agent` as the base and tag the engine, so each proposal is attributable
  (e.g. `agent_id = agent.id`, distinguished by `engine_id`). The kernel sets
  this; engines never self-declare it (consistent with the anti-spoofing rule at
  system.rs ~L819).
- **Synthesis / final answer**: `agent_id = config.synthetic_agent_id` (default
  `system.consensus`), `engine_id = "consensus"`. This is the only
  `ThoughtResponse` delivered to the user/chat.

### 5.2 Fail-safe matrix (an earlier decisiond, G4)

| Condition                              | Behavior                                             |
| -------------------------------------- | ---------------------------------------------------- |
| `consensus_engines.len() < min`        | Immediate error `ThoughtResponse`, no engine run.    |
| An engine returns `Err` / times out    | Log, drop that engine, continue with the rest.       |
| Collected `< min_proposals`            | Error `ThoughtResponse` ("not enough proposals").    |
| Synthesizer `Err` / times out          | Error `ThoughtResponse` ("synthesis failed"); MAY    |
|                                        | fall back to returning the proposals concatenated.   |
| All good                               | One synthesized `ThoughtResponse`.                   |

Every terminal branch emits exactly one `ThoughtResponse` (success or error) so
the user never waits on a silent timeout. The background cleanup task in
consensus.rs is removed along with the session map (no long-lived sessions
remain).

### 5.3 Reuse boundary (an earlier decisione)

Reused unchanged: `run_agentic_loop` (engine resolution, `mind.` prefix
fallback, MCP/plugin dispatch, tool grants, retriable-error fallback). New code
is limited to: the concurrent fan-out, per-engine timeout wrapping, the
combined-views prompt builder (lifted from `consensus.rs` `on_thought_response`),
and the fail-safe branching. No new engine-call primitive is introduced (G2).

### 5.4 What is deleted vs kept

- **Deleted**: `ConsensusRequested` event emission and its `events.rs` routing;
  the per-engine `ThoughtRequested` fan-out in system.rs; `ConsensusOrchestrator`
  event handling, the `sessions` map, and the cleanup task.
- **Kept**: `ConsensusConfig` and its env wiring (`CONSENSUS_ENGINES`,
  `CONSENSUS_PREFIX`, `CONSENSUS_SYNTHESIZER`, `CONSENSUS_MIN_PROPOSALS`,
  `CONSENSUS_SESSION_TIMEOUT_SECS`, `CONSENSUS_AGENT_ID`); the combined-views
  prompt; the synthesizer prompt template.
- **Open sub-decision for ratification**: whether `ConsensusRequested` /
  `ThoughtRequested` variants stay in the `ClotoEventData` enum (kept for
  protocol stability / future Option-B reuse) or are removed. Default
  recommendation: **keep the enum variants** (cheap, avoids a breaking protocol
  change) but stop producing/consuming them in the consensus path.

## 6. Config and defaults

No new config keys. Defaults already additive: `CONSENSUS_ENGINES` empty →
consensus never triggers (G6). Documentation of the keys and a recommended
multi-engine example belongs to an earlier decision (distribution/docs).

## 7. Dashboard visibility hook (forward note for an earlier decision)

Because Option A no longer puts per-proposal events on the bus, live
visualization (each engine's proposal, Collecting/Synthesizing state) needs the
consensus branch to emit explicit progress events (e.g. a `ConsensusProgress`
SSE payload) at: session start, each proposal collected, synthesis start, and
completion. This is out of scope for the implementation task (#112) but the
branch should be structured so these emit points are trivial to add.

## 8. Test plan (for the implementation an earlier decision)

- **Unit**: combined-views builder formatting; fail-safe branches
  (`< min` engines configured; an engine erroring; `< min` collected; synthesizer
  erroring/timing out) each yield the expected single `ThoughtResponse`.
- **Integration (mock engines)**: a `consensus:` message with 2+ mock engines →
  proposals collected concurrently → synthesis → one unified `ThoughtResponse`
  with `synthetic_agent_id`. Assert no `ConsensusRequested`/`ThoughtRequested`
  is emitted. Assert a slow/late engine past the timeout is dropped, not merged.
- **Regression**: with `CONSENSUS_ENGINES` empty, a `consensus:`-prefixed
  message is treated as a normal message (or a defined no-op) — confirm the
  additive default.
- **Registry**: flip bug-408 `open → fixed` with the rationale "eliminated by
  in-kernel synchronous orchestration"; run `scripts/verify-issues.sh` → expect
  `[FIXED]`.

## 9. Done criteria for this design task (#111)

This doc lands (Tier A — PR + review) and is ratified by the maintainer. The
ratification points are: (a) Option A vs B, (b) the §5.4 enum-variants
sub-decision, (c) the proposal `agent_id` stamping convention in §5.1.
Implementation (#112) does not start until these are confirmed.
