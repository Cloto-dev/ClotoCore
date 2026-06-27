# Recall Session Scope — v2 Design (channel-axis default unification)

**Status:** Deferred design, for next-session pickup. A/B-gated **default change** — not an additive knob. Follows knob 2 v1 (per-agent `session_scope`, ClotoCore PR #193) under Goal #120.

This document is self-contained: it is the entry point for implementing v2 in a later session.

## 1. Background — what knob 2 v1 landed (PR #193)

`SessionScope { PerUser, Channel, Thread }` lives in `agent.metadata["session_scope"]`. The kernel resolves it into the two **long-term** recall axes it assembles for the CPersona `recall` call — `source_id` (user axis) and `channel` (episode axis) — in one place (`derive_recall_scope` in `crates/core/src/handlers/system.rs`), so the two axes can no longer disagree.

v1 is **purely additive**: the default `PerUser` reproduces the historical scoping byte-for-byte:

| scope | `source_id` (user axis) | `channel` (episode axis) |
| --- | --- | --- |
| `per_user` (default) | the user (`MessageSource::User.id`) | the bridge **type** (`external_source`, e.g. `"discord"`) |
| `channel` | `""` (channel-shared) | concrete `external_channel_id` |
| `thread` | `""` | concrete `external_channel_id` |

## 2. The v1 limitation v2 addresses

Under the default (`PerUser`), the CPersona episode `channel` axis is the **bridge type** (`"discord"`) for *every* Discord channel. Long-term episodes are therefore **not separated per concrete channel** — only the user axis (`source_id`) separates them. A user active in several Discord channels has all their episodes filed under one `"discord"` channel, so a recall in channel A can surface an episode from unrelated channel B.

The kernel already receives the concrete channel id as `external_channel_id` (`events.rs:531-533` forwards `channel_id` → `external_channel_id`), but the `PerUser` default deliberately ignores it to preserve behavior. (Note: `external_source` is the bridge *type*, set from the message `source` field in `events.rs:522` — it is **not** a channel id. This was the key discovery while scoping v1.)

## 3. v2 proposal

Make the default `PerUser` also scope the episode `channel` axis to `external_channel_id` (the concrete channel) instead of the bridge type, giving **per-channel episode separation even without opting into `Channel`/`Thread` scope**.

This is a **default behavior change** — every existing agent's long-term recall channel axis moves from `"discord"` to the concrete channel id — so it is **gated on A/B validation**, not shipped additively. Same discipline as knob 1's deferred default flip (`always` → `session_start+active`).

## 4. Hypothesis

Per-channel episode separation reduces cross-channel topic-drift **contamination** (the Scope #17 root line) without recall loss — the channel-axis analogue of how per-user `source_id` separation already works. Expected: fewer off-topic recalls sourced from unrelated channels, same on-topic hit-rate.

## 5. A/B plan

- **Harness:** `scripts/recall_ab/` (the precision/contamination A/B harness already used on the recall-contamination line).
- **Variants:** A = current default (`channel = "discord"`), B = `channel = external_channel_id`.
- **Corpus:** a multi-channel transcript where the same user/agent is active across several channels (this is where the two variants diverge).
- **Metric:** cross-channel contamination rate (off-topic recalls from other channels) vs on-topic recall hit-rate.
- **Decision gate:** ship B as the default only if contamination drops with **no** recall-quality regression. Otherwise keep B available only via the opt-in `channel`/`thread` scopes (v1) and leave the default unchanged.

## 6. Implementation sketch

- Minimal change in `derive_recall_scope` (`crates/core/src/handlers/system.rs`): the `PerUser` branch uses `channel_id` (with `base_channel` fallback) for the channel axis instead of `base_channel`, while keeping `source_id = base_source_id` (per-user).
- During the A/B window, gate the change behind a flag (env or per-deployment) so A and B can be compared on the same build before flipping the hardcoded default.
- Tests: update the `per_user_preserves_historical_scoping` unit test to reflect the new channel axis once the default flips; no new derivation surface.

### 6.1 What landed (kernel + harness plumbing) — flip still pending

The gated change is implemented; **the hardcoded default has NOT flipped** — it is opt-in via env during the A/B window:

- **Flag:** `CLOTO_RECALL_PERUSER_CHANNEL_AXIS` (`config.rs` → `Config.recall_per_user_channel_axis` → `SystemHandler`, same wiring as `CLOTO_MCP_STREAMING_ENABLED`). Off by default. Accepts `true`/`1`/`yes`/`on`.
- **Kernel:** `derive_recall_scope` takes `per_user_channel_axis: bool`. Arm A (flag off) reproduces v1 byte-for-byte; arm B (flag on) scopes the `PerUser` episode channel axis to `external_channel_id` (with `base_channel` fallback), keeping per-user `source_id`. `Channel` / `Thread` are unaffected. Unit tests cover both arms (`per_user_preserves_historical_scoping` = arm A, `per_user_channel_axis_scopes_episode_to_concrete_channel` + `..._falls_back_to_base_channel_without_concrete_id` = arm B).
- **Harness plumbing** (`scripts/recall_ab/run_ab.py`): per-message `external_channel_id` emission — without it the flag's divergence point is never exercised. New flags `--channel-id` (global), per-entry `channel_id` in the corpus / query-set JSON (overrides global), and `--corpus` / `--query-set` to point at per-channel fixtures. All additive; absent `channel_id` → identical to the pre-v2 harness.

### 6.2 The store/recall asymmetry fix (REQUIRED before the A/B is valid)

The §6.1 change above gated only the **recall** channel. The **store / archive** path
(`mem.store` for the user message at `system.rs` ~1466 and the agent reply at ~1109)
unconditionally filed every memory under the bridge type (`external_source`,
`"discord"`/`"chat"`), ignoring `session_scope` and the flag entirely. CPersona's
channel filter is an **exact match** (`vector.py` `WHERE … AND channel = ?`;
`_search_episodes_fts` exact), so:

- **Arm B would have produced a false win via recall *collapse*** — memories filed under
  `"chat"` but recalled under the concrete `channel_id` never match → zero recall, which
  the harness scores as "0 % drift" while actually returning nothing.
- The same asymmetry **latently broke knob 2 **v1**'s opt-in `Channel` / `Thread` scopes**:
  recall queried the concrete channel while the store filed under the bridge type, so they
  too recalled nothing. (Untested because the kernel default is `PerUser`.)

**Fix (this PR):** both sides now derive the channel from one helper,
`SessionScope::derive_channel(base_channel, channel_id, per_user_channel_axis)`, which
`derive_recall_scope` also calls — so store and recall can never disagree. `PerUser` with
the flag off returns `base_channel` (the bridge type) → **arm A / the default is
byte-for-byte unchanged**; `PerUser` + flag, and `Channel`/`Thread`, file under the concrete
channel. The invariant is locked by `derive_channel_matches_recall_channel_for_every_scope_and_flag`.

- **Scope:** **memory** store channels only. The **episode** archive (`maybe_archive_episode`,
  `archive_args` has no `channel`) is intentionally left filing under `''` (unscoped) — adding
  a channel there would change the *default* episode-recall behaviour (today `''` episodes are
  excluded by any channel filter), which is out of scope for a behaviour-preserving gate. Per-channel
  **episode** filing is a follow-up (it also needs to decide an episode's channel when it summarizes
  memories spanning several channels). The A/B is unaffected: its signal is the seeded memories, and
  any unscoped episodes are filtered out of both arms identically.

**Run arms on the same build** by toggling the env var per deployment: arm A = flag unset, arm B = `CLOTO_RECALL_PERUSER_CHANNEL_AXIS=true`. (The harness's `--arm` only labels output; behavior is whichever build/env is running — see `scripts/recall_ab/README.md`.) **Because the store channel is now flag-dependent, the corpus must be re-seeded inside each arm** (reset the test agent with `delete_agent_data` between arms), not seeded once — see the README "Ready-made v2 fixtures".

**Still pending (the actual gate, 博士-environment):** author a multi-channel corpus (§5) where the same user/agent is active across ≥2 concrete channels, run arm A vs arm B, and flip the hardcoded default in `from_metadata`'s `PerUser` branch only if cross-channel contamination drops with no recall-quality regression. The corpus is best authored against real responses during the run rather than blind.

## 7. Relationship to other deferred slices (independent of v2)

- **Short-term `session_key` scoping** (bridge-owned; chunk lifecycle stays with the bridge for now) — independent of v2.
- **Channel-vs-Thread parent rollup** — folding a thread into its parent channel under `Channel` scope needs the bridge to forward `parent_channel_id` (currently inside `thread_info`, not forwarded to the kernel). Independent of v2; until it lands, `Channel` and `Thread` coincide.
- v2 is **only** the long-term channel-axis default.

## 8. Next-session entry point

The kernel change + harness plumbing have landed (§6.1); the remaining work is the A/B run and the default flip:

- **Flip point:** `from_metadata`'s `PerUser` branch / the `recall_per_user_channel_axis` default in `config.rs` — flip only after the gate passes, then remove the flag.
- **A/B run:** deploy the same build twice (arm A flag off, arm B `CLOTO_RECALL_PERUSER_CHANNEL_AXIS=true`); author a multi-channel corpus and run `scripts/recall_ab/` with `--channel-id` / per-entry `channel_id` (see its README "Cross-channel (knob2 v2) A/B").
- **Context:** CPersona memory `goal120-knob2-v1-pr193-20260627`; this doc; Goal #120.
