# Visual apex — first real-VM run (2026-07-14)

First end-to-end run of the dual-oracle visual apex against a **real ClotoCore
GUI** on a Windows 11 VM, driving the app through the interactive-desktop
actuator agent (`opv_agent.py`). This is the transition from stub-proven
scaffolding (`selftest.py`) to real backends (`backends_vm.py`).

## Environment

* Windows 11 VM (Proxmox VMID 104), interactive **session 1** active.
* `opv_agent.py` launched via a Task Scheduler **Interactive** principal →
  listens on `127.0.0.1:8900` (`/grab` = mss, `/act` = pyautogui, `/run`).
* Driver host reaches the agent over SSH (session 0) → localhost socket →
  agent (session 1). The network socket bridges the session wall; no VM
  firewall / BIND_ADDRESS change needed.
* App under test: ClotoCore **0.6.8-beta.1** (installed `app.exe`). Kernel
  admin HTTP API on `127.0.0.1:8081`; `/api/system/health` answers
  unauthenticated → the dual oracle's deterministic hard-gate.

## What the apex caught on first contact

On the very first launch the GUI showed a fatal modal — **"Cloto Kernel failed
to start: (code: 26) file is not a database"** — a startup failure that
manifests *only* as a GUI dialog (a headless kernel probe would see "not
listening" without the diagnostic). The visual oracle read the exact error.

* **Root cause (this run):** a 15-byte dummy `cloto_memories.db` (marker
  `seed-…`) left by a prior `proxmox-windows-verify.sh` data-preservation seed,
  in the legacy `%APPDATA%\cloto-system\` data dir that 0.6.8-beta.1 still uses
  (the `config.rs` `data_dir` `cloto-system`→`ClotoCore` unification is not yet
  landed). VM-state artifact, **not** a fresh-install regression.
* **Hardening observation (follow-up):** ClotoCore has no graceful handling of
  a corrupt / non-SQLite DB file — it hard-fails at startup with a fatal modal
  and exits. A real user could hit code-26 from disk corruption, an interrupted
  write, or AV quarantine. Candidate for a backup-and-recreate recovery path.
  Not filed in `qa/issue-registry.json` yet (design observation, not a
  code-pattern regression) — pending review.

After renaming the dummy aside (non-destructive), the kernel created a fresh
valid DB and booted (green status dot, fresh 4096-byte DB, 8081 listening).

## Dual-oracle results (healthy GUI)

| step | actuation | visual oracle | kernel hard-gate | diagnosis |
| --- | --- | --- | --- | --- |
| app-renders | — | onboarding rendered | `status:ok` | AGREE_PASS |
| click-Get-Started | pyautogui click landed | advanced to language-select (page 2/7) | `status:ok` | AGREE_PASS |

`run_vm.py liveness` re-runs the driver end-to-end against the live VM and
reports `verdict: pass` (real grab + real health probe + `cross_check`).

## Proven / deferred

* **Proven:** perception (`/grab`), actuation (`/act`), kernel liveness oracle,
  **operation-level kernel oracle** (see below), `driver.py` + `dual_oracle.py`
  orchestration against real infra, and a real defect-class catch
  (startup-blocking modal).
* **Deferred:** (1) a live multimodal-model vision assessor to replace
  `RecordedVision` (this run's vision is the agent's own recorded read); (2) a
  chat-render journey once onboarding is completed.

## Operation-level kernel oracle (added 2026-07-14)

Liveness (`/api/system/health`) is unauthenticated, but the admin routes
(`/api/agents`, `/api/history`, …) return **403** without the app's
`X-API-Key`. ClotoCore reads `CLOTO_API_KEY` from the environment at boot
(`dashboard/src-tauri/src/lib.rs:721`), so the harness launches the GUI with a
**known** key — the actuator agent's `/run` merges an `env` dict into the
launched process — and then authenticates the kernel probe with it (matching
the opverify daemon flow). `KernelApiProbe` (in `backends_vm.py`, key from
`OPV_API_KEY`) lifts the hard-gate from "is the kernel alive" to "did the
operation take effect": the `agents` journey (`run_vm.py`) cross-checks the
rendered GUI against an authenticated `/api/agents` that confirms the seeded
default agent (`default_engine_id: mind.cerebras`) — **AGREE_PASS**, with the
same route returning 403 unauthenticated (auth is enforced; the key unlocks it).

## Chat-render dual-oracle (added 2026-07-14) — the headline journey

The apex's north-star journey: a real user types into the chat box, and the
assistant's reply **renders** while the kernel **persists** it. Both must agree.

* **Unblocker — the VM was stale.** On beta.1 (2026-06-13) every mind engine
  install hit the *already-fixed* **bug-399** (doubled marketplace path
  `mcp-servers/servers/<id>/servers/<id>/server.py`) plus a Magic Seal failure,
  so no reasoning engine could start and chat was impossible. This was not a new
  defect — the apex faithfully reproduced a **known, since-fixed regression on a
  stale build**. Fix `e5b1b85` (#207, 2026-06-29) ships in beta.2. Updating the
  VM to **0.6.8-beta.2** (verified installer SHA-256, silent NSIS install,
  relaunched with a known `CLOTO_API_KEY`) let the `cerebras` engine connect
  immediately (`/api/mcp/servers` → `cerebras: Connected`, key carried in the
  preserved DB).
* **Kernel oracle proven first.** `POST /api/chat` with a nonce →
  `ThoughtResponse` in `/api/history`, `engine_id=cerebras`, content = the exact
  nonce echoed. `default_engine_id=mind.cerebras` resolves to the bare `cerebras`
  engine (the `mind.` prefix is stripped by migration + runtime; not a mismatch).
* **Visual × kernel = AGREE_PASS.** Driving the GUI (finish onboarding → chat
  input → type nonce → Enter), the response bubble rendered the exact nonce in
  0.8 s (no stuck spinner / greying / overlap), and `/api/history` independently
  showed the correlated `cerebras` `ThoughtResponse`. Cross-check: **AGREE_PASS**.

### Two-tier execution (orchestrator ↔ VM executor)

This run introduced the permanent split now codified in
`VM_EXECUTOR_RUNBOOK.md`: the **orchestrator** (Opus) designs the journey +
dual-oracle and *verifies* the verdict (re-runs the deterministic kernel
correlation itself + eyeballs the response frame); a **Sonnet subagent** does the
GUI actuation, frame grabs + first-pass visual assessment, and kernel probes,
returning a compact structured verdict. This keeps the orchestrator's context
lean and realizes the "live multimodal assessor" that `RecordedVision` was a
placeholder for.

* **Harness artifact caught (not a product bug):** the session-1 agent's
  pyautogui `write()` mis-typed `:` as `*` on the JP keyboard layout (visible in
  the sent message, not the alphanumeric nonce). Prefer clipboard paste for exact
  literal text. See `VM_EXECUTOR_RUNBOOK.md` "Known harness artifacts".
* **Follow-up (low priority):** two extra `MessageReceived` events carrying the
  bare nonce appeared after the `ThoughtResponse` — possible duplicate emission,
  not yet investigated.
