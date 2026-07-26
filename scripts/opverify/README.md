# opverify — operation-to-success verification harness

A standing, dependency-free verification flow that boots a live `clotocore`
kernel daemon and drives a broad catalog of **real operations to success**
over its HTTP admin API — asserting that each operation *actually took
effect*, not merely that the process did not crash. It complements the
existing quality layers (`--smoke` boot check; `proxmox-windows-verify.sh`
installer/upgrade diff) with the missing layer: *does the running app hold
up under many kinds of real use?*

Design authority: CSC Goal #169. This is meant to be a **permanent**
capability — committed package, coverage ratchet, nightly CI, and a result
ledger that is actually written and committed (see Status: the ledger existed
in code but was wired to nothing until 2026-07-27, which is exactly the rot
this design claims to prevent).

## Quick start

```sh
# from the ClotoCore repo root
cargo build --bin clotocore                 # if not already built
python3 -m scripts.opverify.run --target local --slice phase0
```

Exit code is the verdict: `0` = pass, `1` = fail — usable as a CI gate and
as a pre-stable-cut manual check. Add `--report out.json` for the full
machine-readable report.

```
python3 -m scripts.opverify.run \
    --target local \          # local | linux-vm (phase 2) | windows-vm (phase 3)
    --slice all \             # all | phase0 (LLM-free spine subset)
    --ratchet report \        # report (list gaps) | enforce (uncovered route => fail)
    --binary path/to/clotocore \   # optional; defaults to target/{debug,release}/clotocore
    --ledger \                # record this run in qa/opverify/history.jsonl + check
                              #   for regressions vs the prior same-target baseline
                              #   (nightly/VM runs; omit for local iteration)
    --report out.json
```

## How it works

* **deploy/** stands a daemon up somewhere and hands the harness an HTTP
  endpoint + admin key. `local` boots an isolated child process (throwaway
  `DATABASE_URL` + `CLOTO_SANDBOX_DIR`, captured stderr, teardown via the
  authenticated `POST /api/system/shutdown` since the kernel installs no
  SIGTERM handler). `linux-vm` / `windows-vm` (phases 2/3) deploy into a
  Proxmox guest via snapshot rollback.
* **catalog/** is the set of operations, one module per domain. Each
  `Operation` declares the routes it `covers`, a `drive()` that performs it,
  and an `assert_success()` that proves it took effect.
* **oracle.py** runs the cross-cutting checks between operations and at the
  end: liveness (`/api/system/health`), integrity (`/api/health/scan`),
  resource footprint (RSS / open FDs / child-process count — catches MCP
  orphans), stderr panic/ERROR scraping, and a post-teardown
  `PRAGMA integrity_check` on the throwaway DB.
* **coverage.py** is the ratchet: it parses the kernel route table out of
  `crates/core/src/lib.rs` and fails (in `enforce` mode) if any meaningful
  route is not claimed by some operation — so "comprehensive" stays true as
  the API grows.
* **harness.py** wires it together into a structured report; **run.py** is
  the CLI.

## Requirements

Python standard library only (no `pip install`). The corruption oracle uses
the `sqlite3` CLI and resource sampling uses `ps` / `pgrep` / `lsof` when
present; each degrades to `None` rather than failing where unavailable.

## Status

* **Phase 0 (done)** — local spine: health, agents (list + full lifecycle),
  memory (read), events (publish→history), mcp (list); oracles; coverage
  ratchet; JSON report.
* **Phase 1 (in progress)** — the headline **real-LLM chat** verification is
  live: `chat.deepseek` drives a genuine round trip (engine subprocess → LLM
  proxy → DeepSeek → `ThoughtResponse` correlated to our exact message id) in
  an **isolated copy** of the operator's DB. Isolation is machine-proven by
  the `isolation` oracle (real `cpersona.db` / `cscheduler.db` are byte-for-byte
  unchanged across a run). The result **ledger** (`qa/opverify/history.jsonl`,
  via `--ledger`) records each run and flags regressions vs the prior
  same-target baseline. Domains landed so far: health, agents, memory, events,
  mcp (list **+ register→call→stop→reap** lifecycle — the Goal #145 orphan
  target), chat (deepseek), **llm** (inspect + configure/real-connection test),
  **cron** (full job lifecycle), **marketplace** (catalog), **permissions**
  (approve/deny against staged pending requests), **system**, **plugins**, and
  **setup** (onboarding gate) — every locally-drivable domain is now covered.
  Still to add: `chat.cerebras` (needs its mind engine installed) and
  `chat.groq` (needs a provider row + key), which land with the full-real
  keys/engines available on the VM and nightly tiers.
* **Phases 2–3 (not started)** — the kernel tiers on a real VM: `--target
  linux-vm` / `--target windows-vm` both still exit with "not yet implemented
  (phase 2/3)". The only tier that touches a VM today is the visual apex
  (`visual/`), which is driven by hand.
* **Phase 4 (partially wired)** — permanence. What is wired:
  * the nightly (`opverify-nightly.yml`) passes `--ledger`, so each of its
    three OS jobs appends a row and turns a regression (a coverage drop, or an
    operation that passed last night and fails now) into a **red nightly**;
  * a single `commit-ledger` job collects those rows and commits
    `qa/opverify/history.jsonl` back to master, so the trend is durable rather
    than living in a 14-day artifact;
  * the apex records to the same file — `python -m
    scripts.opverify.visual.run_vm <journey> --ledger`. Apex rows carry their
    own `target_kind` (`apex`), so they are only ever compared against prior
    apex rows and never trip the route-coverage check;
  * `ledger_selftest.py` gates that distillation in CI (Lint job), because
    nothing else exercises it on a PR.

  What is **not** wired, and what this therefore does not claim: recording a
  run is not the same as making runs happen. The apex is still hand-driven, its
  obligation lives in `CLAUDE.md` (a text rule, not a mechanism), and the
  ledger cannot record a tier that does not run — which is precisely how the
  apex managed to run exactly once between 2026-07-14 and 2026-07-27 while the
  machinery designed to notice that sat unwired.

### Chat isolation (how a real key is used without ever being read)

`bootstrap.py` copies the operator's dev DB to a throwaway (the LLM key rides
along inside the copy and is **never** selected or logged — copy+use is
permitted, disclosure is not), then on the copy: deactivates every MCP engine
except the pure-HTTP reasoning engine (`is_active=0`, so the DB-touching
`cpersona`/`cscheduler`/`embedding` subprocesses never spawn), rewrites the
engine's LLM-proxy port to a private one, points the target agent at it, and
disables all other agents (so the heartbeat fires no stray billable calls).
The install's `seal.key` is copied into the throwaway so Magic Seals verify for
real. The isolation oracle fingerprints the real DBs before boot and asserts
they are untouched after teardown.
