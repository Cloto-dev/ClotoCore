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

## Cost

One LLM call per turn: ~19 (seed, once) + 14 × `trials` per arm. At `trials=3`
that is ~42 calls/arm, ~84 across both arms, plus the one-time seed — on the
order of the original report's ~$2 run, engine-dependent.
