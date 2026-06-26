# Turnkey runbook — recall-contamination A/B (Phase 5)

Pre-filled, copy-paste steps for running the OLD-vs-NEW A/B. Read `README.md`
for the design rationale; this file is the operational checklist.

The irreducible manual parts are: (1) build & run the kernel on each arm's
branch, (2) provide a reasoning engine (this spends LLM credits, ~$2/run), and
(3) snapshot/restore the test agent between arms. Everything else is one command.

## 0. One-time setup

Create a **dedicated test agent** so production memories are never touched. It
needs three grants and the embedding server up:

- a reasoning engine (`mind.*`)
- `memory.cpersona`
- the embedding server running (vector recall)

> **bug-396 — read before you run.** The kernel resolves an agent's engine by
> **exact MCP server-name match**. The DeepSeek connector installs under the
> catalog id **`deepseek`**, so its server name is `deepseek` — but an agent set
> to `default_engine_id = "mind.deepseek"` will **not** resolve and every turn
> returns `[Error] Engine 'mind.deepseek' not found`. The classifier scores that
> as `coherent`, i.e. a **false 0% drift** that silently corrupts the run (this
> is what wrecked the first Phase-5 pass). Set the test agent's
> `default_engine_id` to the resolvable server-name (e.g. `deepseek`).

Config (flags or env) — defaults shown, override as needed:

| flag | env | turnkey value |
| --- | --- | --- |
| `--base-url` | `CLOTO_BASE_URL` | `http://127.0.0.1:8081` |
| `--api-key` | `CLOTO_API_KEY` | set if the kernel has `admin_api_key` |
| `--agent-id` | `CLOTO_AB_AGENT` | `agent.abtest` |
| `--trials` | `CLOTO_AB_TRIALS` | `3` |

`UV = uv run --python 3.13 --with httpx python` is used below (pin 3.13 — uv's
default 3.14 has a hung pytest-asyncio in this monorepo).

## 1. Pre-flight (do this FIRST — it costs 1 call and catches bug-396)

With the kernel running and the test agent configured:

```bash
cd scripts/recall_ab
uv run --python 3.13 --with httpx python preflight.py \
  --agent-id agent.abtest --api-key "$CLOTO_API_KEY"
```

- `PREFLIGHT OK` → the engine resolves; proceed.
- `PREFLIGHT FAIL (3)` → engine did not resolve (bug-396); fix `default_engine_id`.
- `PREFLIGHT FAIL (2)` → no reply; kernel / engine / embedding server is down.

**Do not run the full A/B until pre-flight passes** — otherwise you spend ~84
LLM calls on a silently-corrupted result.

## 2. Seed once (additive), then snapshot

On the OLD-branch kernel:

```bash
uv run --python 3.13 --with httpx python run_ab.py --agent-id agent.abtest --seed
```

Then **snapshot the test agent's CPersona rows** (the clean post-seed state).
Follow the destructive-DB rules — never `rm` the DB. Snapshot the DB file
(`cp ~/.claude/cpersona.db cpersona.db.postseed`) or use CPersona
export / `delete_agent_data` scoped to **`agent.abtest` only**.

## 3. OLD arm

```bash
uv run --python 3.13 --with httpx python run_ab.py --agent-id agent.abtest --arm old
# → results/results_old.json
```

## 4. Restore the post-seed snapshot

Probe turns are themselves stored as memories and would leak into the NEW arm.
Restore the snapshot from step 2 (test agent only) so both arms start identical.

## 5. NEW arm

Rebuild & run the kernel on the NEW branches (ClotoCore
`feat/discord-recall-gating`, clotohub-servers `feat/discord-per-channel-session`,
cpersona `feat/episode-channel-scoping`), same seeded test agent, then:

```bash
uv run --python 3.13 --with httpx python run_ab.py --agent-id agent.abtest --arm new
# → results/results_new.json
```

## 6. Compare

```bash
python compare.py results/results_old.json results/results_new.json
```

The redesign is working if the NEW arm's `severe_pct_of_completed` drops
materially vs OLD **without** a completion-rate (timeout) regression — the
original L2 experiment failed by cutting drift but spiking timeouts.

## Notes

- Cost: ~19 seed + 14 × `trials` per arm (~42 at trials=3) ≈ ~84 calls both
  arms, ~$2, engine-dependent.
- `classify()` is a heuristic; it is good for relative OLD-vs-NEW comparison but
  not a substitute for human review on borderline cases (see README "Classifier
  caveat"). For higher fidelity, wire an LLM judge against the §2.3 rubric.
