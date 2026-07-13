# opverify — operation-to-success verification harness

A standing, dependency-free verification flow that boots a live `clotocore`
kernel daemon and drives a broad catalog of **real operations to success**
over its HTTP admin API — asserting that each operation *actually took
effect*, not merely that the process did not crash. It complements the
existing quality layers (`--smoke` boot check; `proxmox-windows-verify.sh`
installer/upgrade diff) with the missing layer: *does the running app hold
up under many kinds of real use?*

Design authority: CSC Goal #169. This is a **permanent** capability
(committed package + coverage ratchet + — phase 4 — nightly CI + result
ledger), deliberately built so it cannot rot into an unused pipeline.

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
* **Phase 1** — full 12-domain catalog incl. chat across DeepSeek / Cerebras
  / Groq (real LLM, cheap/free providers), marketplace install, cron,
  permissions, llm; plus the MCP register→call→stop→reap lifecycle and the
  result ledger (`qa/opverify/history.jsonl`) with regression detection.
  Requires provider API keys via a gitignored `.env` / CI secrets.
* **Phases 2–4** — Linux VM, Windows VM (VM 104), and permanence wiring
  (CLAUDE.md standard-procedure rule, nightly CI, ledger commit).
