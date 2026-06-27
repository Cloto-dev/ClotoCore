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

## 7. Relationship to other deferred slices (independent of v2)

- **Short-term `session_key` scoping** (bridge-owned; chunk lifecycle stays with the bridge for now) — independent of v2.
- **Channel-vs-Thread parent rollup** — folding a thread into its parent channel under `Channel` scope needs the bridge to forward `parent_channel_id` (currently inside `thread_info`, not forwarded to the kernel). Independent of v2; until it lands, `Channel` and `Thread` coincide.
- v2 is **only** the long-term channel-axis default.

## 8. Next-session entry point

- **Code:** `derive_recall_scope` in `crates/core/src/handlers/system.rs` (after PR #193 merges to `master`).
- **A/B harness:** `scripts/recall_ab/`.
- **Context:** CPersona memory `goal120-knob2-v1-pr193-20260627`; this doc; Goal #120.
