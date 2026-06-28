# Recall-contamination A/B harness (Phase 5)

Measures topic-drift ("pan → raspberry pie") severity on a running ClotoCore
kernel so the **OLD** recall behaviour (long-term recall every turn) can be
compared against the **NEW** behaviour from the Discord-recall redesign
(session-start-gated recall + per-channel episodic loop + per-agent recall
instructions).

It is the redesign-era successor to the one-off harness described in
[`docs/RECALL_CONTAMINATION_AB_2026-04-24.md`](../../docs/RECALL_CONTAMINATION_AB_2026-04-24.md),
which found a persistent ~23% severe-drift rate that the redesign aims to cut.

## Why one accumulating session matters

The NEW gate only suppresses recall on **continuing** turns (the in-flight T1
transcript is non-empty). So the benefit only shows up across a multi-turn
conversation: turn 1 is a session-start (recall fires), turns 2–N do not
auto-recall. The harness therefore sends the 14 probe queries as a single
**accumulating** session, repeated `--trials` times. Running each query in its
own fresh session would make every query a session-start and erase the gate's
effect (≈ the OLD behaviour).

## Files

| File | Purpose |
| --- | --- |
| `query_set.json` | 14 probe queries (6 categories) + per-query contaminant keywords |
| `seed_corpus.json` | ~19 memories that reproduce the bread/raspberry-pie contamination |
| `run_ab.py` | seeds, runs the probe via `/api/chat` + SSE, classifies, writes `results_<arm>.json` |
| `compare.py` | side-by-side OLD vs NEW + aggregate severe-drift delta |

## Prerequisites

- A running ClotoCore kernel reachable over HTTP (default `http://127.0.0.1:8081`).
- A **dedicated test agent** (recommended) so production memories are untouched,
  with a reasoning engine granted (`mind.*`) and a memory server granted
  (`memory.cpersona`). The embedding server must be up for vector recall.
- The kernel's `X-API-Key` if `admin_api_key` is configured.
- Python with `httpx` (e.g. `uv run --with httpx python run_ab.py ...`).
  Use Python 3.13 if running under `uv` — `uv`'s default 3.14 has hung
  pytest-asyncio in this monorepo (unrelated to this script, but pin to be safe:
  `uv run --python 3.13 --with httpx python run_ab.py ...`).

Configuration is via flags or env vars:

| flag | env | default |
| --- | --- | --- |
| `--base-url` | `CLOTO_BASE_URL` | `http://127.0.0.1:8081` |
| `--api-key` | `CLOTO_API_KEY` | (none) |
| `--agent-id` | `CLOTO_AB_AGENT` | `agent.cloto_default` |
| `--arm` | `CLOTO_AB_ARM` | `unset` (output label only) |
| `--source-id` | `CLOTO_AB_SOURCE_ID` | `abtest:user1` |
| `--channel` | `CLOTO_AB_CHANNEL` | `chat` (use `discord` to mimic the bridge) |
| `--trials` | `CLOTO_AB_TRIALS` | `3` |
| `--response-timeout` | `CLOTO_AB_RESPONSE_TIMEOUT` | `90` (seconds) |

`--arm` does **not** change behaviour — the arm is whichever kernel build is
running. It only labels the output file.

## Procedure

> The script performs **no** destructive DB operations. You snapshot/restore the
> test agent's CPersona rows between arms so each arm sees the same seed corpus
> (probe turns are themselves stored as memories and would otherwise leak into
> the next arm). Follow the project's destructive-DB rules — never `rm` the DB;
> snapshot the file or use the CPersona export/`delete_agent_data` tooling on the
> **test** agent only.

1. **Build & run the kernel on the OLD branch** (e.g. `main`), with the test agent.
2. **Seed once** (additive):
   ```bash
   uv run --python 3.13 --with httpx python run_ab.py \
     --agent-id agent.abtest --seed
   ```
   Then snapshot the test agent's CPersona rows (the clean post-seed state).
3. **Run the OLD arm:**
   ```bash
   uv run --python 3.13 --with httpx python run_ab.py \
     --agent-id agent.abtest --arm old
   ```
   → writes `results/results_old.json`.
4. **Restore** the post-seed snapshot (undo the probe-turn memories).
5. **Build & run the kernel on the NEW branch** (the feature branches:
   ClotoCore `feat/discord-recall-gating`, clotohub-servers
   `feat/discord-per-channel-session`, cpersona `feat/episode-channel-scoping`),
   restart against the same seeded test agent.
6. **Run the NEW arm:**
   ```bash
   uv run --python 3.13 --with httpx python run_ab.py \
     --agent-id agent.abtest --arm new
   ```
   → writes `results/results_new.json`.
7. **Compare:**
   ```bash
   python compare.py results/results_old.json results/results_new.json
   ```

To also exercise the per-agent recall-instructions knob or the Discord channel
path, set `--channel discord` and/or configure the test agent's
`recall_instructions` in the agent settings, then add more arms.

## Cross-channel (knob2 v2) A/B

> **Historical (2026-06-28):** this A/B has run and the default flipped — the
> `CLOTO_RECALL_PERUSER_CHANNEL_AXIS` flag below is **removed** from the kernel,
> so arm A / arm B can no longer be toggled on one build. This section is kept as
> the record of how the gate was validated; see
> `docs/RECALL_SESSION_SCOPE_V2_DESIGN.md` §8 for the outcome.

The default topic-drift experiment above is single-channel. The knob2 v2 change
(`docs/RECALL_SESSION_SCOPE_V2_DESIGN.md`) only diverges when the **same user is
active across several concrete channels**, so it needs a multi-channel corpus and
the concrete-channel plumbing:

- **Arms are the same build, toggled by env** — arm A = `CLOTO_RECALL_PERUSER_CHANNEL_AXIS`
  unset, arm B = `CLOTO_RECALL_PERUSER_CHANNEL_AXIS=true`. (`--arm` still only labels
  output.)
- **Concrete channel id** is sent as `external_channel_id` via `--channel-id`, or
  per-entry `channel_id` in the corpus / query-set JSON (which overrides the global
  flag). Absent → omitted, i.e. the pre-v2 behavior. `--corpus` / `--query-set`
  point at per-channel fixtures.

Sketch (author the multi-channel corpus against real responses):

1. Seed a corpus where the same `--source-id` has memories in ≥2 channels — either
   one corpus with per-entry `channel_id`, or two `--seed` passes with different
   `--channel-id`. Snapshot the post-seed CPersona rows.
2. **Arm A:** run the kernel with the flag unset; probe in one channel
   (`--channel-id <A>` or per-query `channel_id`) and measure how often memories
   from the *other* channel contaminate. Restore the snapshot.
3. **Arm B:** restart the same build with `CLOTO_RECALL_PERUSER_CHANNEL_AXIS=true`;
   run the identical probe.
4. Compare: v2 wins if cross-channel contamination drops with no recall-quality
   (completion-rate) regression — then flip the hardcoded default.

### Ready-made v2 fixtures

`seed_corpus_v2.json` / `query_set_v2.json` implement the sketch above so the run
is copy-paste. The scenario: one user active in two channels — `cook-general`
(#料理, bread & desserts) and `maker-denshi` (#電子工作, Raspberry Pi the
single-board computer). The homophone ラズベリーパイ (dessert vs. SBC) is the
contamination bridge; every probe runs in `cook-general`, and drift is measured
by **unambiguous electronics vocabulary** (GPIO / SDカード / はんだ / Raspberry Pi
OS …) that can only have come from the *other* channel. Arm A files every memory
under one channel (cross-channel mixing → leak); arm B files each under its
concrete channel (per-channel separation → no leak).

> **Seed once per arm — NOT once total.** The channel a memory is *filed under*
> is itself flag-dependent (the v2 fix makes store and recall use the same
> `derive_channel`): arm A files everything under the bridge channel, arm B files
> per concrete `channel_id`. So the corpus must be re-seeded inside each arm, and
> the test agent reset between them — `delete_agent_data` scoped to `agent.abtest`
> (NOT a whole-DB snapshot/restore; CPersona is agent-keyed, so agent-scoped
> delete is the clean, production-safe reset).

```bash
UV="uv run --python 3.13 --with httpx python"
COMMON="--agent-id agent.abtest --source-id abtest:user1 \
  --corpus seed_corpus_v2.json --query-set query_set_v2.json"

# ---- Arm A: flag UNSET (historical default; all memories filed under one channel) ----
# (kernel running with CLOTO_RECALL_PERUSER_CHANNEL_AXIS unset)
$UV run_ab.py $COMMON --seed                 # seed under arm A's filing
$UV run_ab.py $COMMON --arm old              # → results/results_old.json
#   reset the test agent: delete_agent_data(agent.abtest) via the kernel's CPersona

# ---- Arm B: flag SET (memories filed per concrete external_channel_id) ----
# (restart the SAME build with CLOTO_RECALL_PERUSER_CHANNEL_AXIS=true)
$UV run_ab.py $COMMON --seed                 # re-seed under arm B's filing
$UV run_ab.py $COMMON --arm new              # → results/results_new.json
#   reset again when done: delete_agent_data(agent.abtest)

python compare.py results/results_old.json results/results_new.json
```

The `X*` (cross-channel-magnet) and `W*` (open-meta) probes are where A and B
diverge most — a ラズベリーパイ question in the cooking channel should not return
GPIO/SDカード advice. The `Y*` cooking probes guard against recall loss (arm B
must keep recalling `cook-general`'s own memories — they are filed under the same
channel the probe queries). `F*` are false-positives (any seeded topic in a
weather/weekend chat is drift).

## Reading the results

Each `results_<arm>.json` has per-query verdicts (`coherent` / `mild` /
`severe` / `timeout` / `error`) across the trials and an aggregate
`severe_pct_of_completed` (severe ÷ non-timeout/non-error trials, matching the
original report's metric). The redesign is working if the NEW arm's severe%
drops materially vs OLD, **without** a completion-rate (timeout) regression —
the original L2 experiment failed exactly because it cut drift but spiked
timeouts.

## Classifier caveat

`classify()` is a heuristic approximation of the report's §2.3 rubric
(unrelated-topic keyword present + elaboration → severe; with a disclaimer →
mild; otherwise coherent). It is good for relative OLD-vs-NEW comparison but is
not a substitute for human judgement on borderline cases. The raw responses are
not stored by default; add a dump if you want manual re-grading, or wire an
LLM judge that scores each response against the rubric for higher fidelity.

**Known false-positive blind spot (`F*` probes).** A `false_positive` query
scores against the *entire* `memory_topics` list (`classify()` line ~228), which
includes everyday cooking words (パン / クロワッサン / ケーキ / 発酵 / オーブン …),
not just the cross-channel electronics terms. `elaborated` also trips on a single
`?` / `？` (line ~238). Because the probe runs as one accumulating session with
`F11`/`F12` last — right after the cooking conversation — a perfectly coherent
reply such as "週末はパンでも焼いてみては？" hits `パン` + `？` and is graded
**severe**. This is a measurement artifact, not memory contamination: it is
channel-axis-independent (both arms score `F12` 3/3 severe), whereas genuine
cross-channel drift is captured by the `X*`/`W*` probes via electronics-only
vocabulary (0/9 on the per-channel arm). The historical "~23% severe baseline"
was produced by the same classifier and likely carried this artifact, so treat
it as an upper bound. To tighten: drop everyday words from `memory_topics` for
`false_positive` probes, require an electronics-specific hit, or make the `?`
elaboration test stricter — or replace the heuristic with an LLM judge.

## Cost

One LLM call per turn: ~19 (seed, once) + 14 × `trials` per arm. At `trials=3`
that is ~42 calls/arm, ~84 across both arms, plus the one-time seed — on the
order of the original report's ~$2 run, engine-dependent.
