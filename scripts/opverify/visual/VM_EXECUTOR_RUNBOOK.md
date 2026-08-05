# Visual apex — two-tier orchestration (orchestrator ↔ VM executor)

The visual apex is driven by **two cooperating models**, not one. This split is a
permanent part of the opverify pipeline (adopted 2026-07-14): it keeps the
orchestrator's context lean while a cheaper multimodal model does the heavy,
repetitive VM I/O — and it *is* the "live multimodal assessor" the bootstrap
`RecordedVision` was always meant to become (see `run_vm.py`).

```
┌─ Orchestrator (Opus, main loop) ──────────────────────────────────────┐
│  • designs the journey + the dual-oracle assertions                    │
│  • writes the executor's task from THIS runbook                        │
│  • VERIFIES the executor's verdict (re-runs the deterministic kernel   │
│    correlation itself + eyeballs the critical frame(s))                │
│  • records the result (CSC / ledger) and decides next steps            │
└───────────────────────────────────────────────────────────────────────┘
                    │ spawns (Agent, model=sonnet, ~mid effort)
                    ▼
┌─ VM executor (Sonnet, subagent) ──────────────────────────────────────┐
│  • actuates the GUI (click/type/key via the session-1 agent)          │
│  • perceives (grabs frames, READS the pixels — first-pass visual       │
│    assessment: rendered? defect? spinner stuck?)                       │
│  • probes the kernel (health / chat / history)                        │
│  • returns a COMPACT structured verdict + key frame paths (never the   │
│    raw PNG bytes or full history) so the orchestrator stays lean       │
└───────────────────────────────────────────────────────────────────────┘
```

Why two tiers: every `/grab` is a ~40 KB PNG the visual oracle must *read*, and a
journey is many grab→read→act cycles. Doing that in the orchestrator's own
context bloats it fast. Delegating the cycle to a Sonnet subagent isolates that
cost; the orchestrator pays only for the design + the compact verdict + a
spot-check. Cost pools also differ, and the executor's visual read is a genuine
intelligent perceiver — exactly the apex's design (a VLM in the human perception
loop), just hosted in a subagent.

## Two ways the executor drives

- **Manual VM I/O** (the flow detailed below): the subagent curl/tunnel-grabs,
  Reads the PNG, judges, curl-acts, curl-probes — assembling the dual-oracle
  verdict itself. Maximum flexibility for exploratory / ad-hoc journeys.
- **Structured handshake** (`OPV_ASSESSOR=handshake`, #237): the subagent runs
  `python -m scripts.opverify.visual.run_vm <journey>` in the background and only
  supplies the *visual* verdict per frame — the driver owns the loop, journey
  structure, dual-oracle cross-check, tiering and forensic capture. Prefer this
  to run a *defined* journey with less hand-rolling. Protocol: the driver writes
  `req_NNN.json` `{seq, question, frame}` into `OPV_EXCHANGE_DIR`; you Read the
  referenced PNG (your live eyes) and Write `resp_NNN.json`
  `{visible: bool, detail: str, defects: [str]}`; repeat until `done.flag`
  appears, then read the run's JSON report. This is the live replacement for the
  `RecordedVision` bootstrap — the executor's read *is* the oracle.

## Orchestrator responsibilities (do NOT delegate these)

1. **Journey + oracle design.** Decide the steps, the visual question at each
   checkpoint, and the kernel hard-gate it cross-checks against.
2. **Verification of the verdict (mandatory).** The executor's report is a
   *claim*. Before accepting AGREE_PASS:
   - Re-run the **deterministic** kernel oracle yourself (e.g. `GET /api/history`
     and confirm the correlated `ThoughtResponse` — engine id + content). This is
     cheap and non-negotiable (delegation-verification discipline).
   - **Eyeball the critical frame(s)** the executor saved (the response render at
     minimum). The visual oracle is the apex's whole point; don't take it on
     faith.
3. **Record + decide.** Log the outcome (an earlier decision / ledger), file any real
   defect, choose the next journey.

## VM executor task template (hand this, filled in, to the subagent)

Spawn with `Agent(subagent_type="general-purpose", model="sonnet")`. The prompt
must contain, verbatim, the operational runbook below plus the journey-specific
goal + dual-oracle spec.

### VM access (stable facts)
- **Standing up the agent (reproducible deploy, #238):** the session-1 actuator
  agent must be running before any transport works. Stand it up with the
  committed, idempotent deployer — no hand-rolled scp/schtasks:
  `python -m scripts.opverify.visual.deploy_agent` (full: ensures `C:\opv`,
  Python + `mss`/`pyautogui`, copies `vm_agent.py`, registers the
  Interactive/RunLevel-Highest/AtLogOn task, starts it, and asserts `/health`
  reports the committed protocol version). `--redeploy` re-copies + restarts
  only (after editing `vm_agent.py`); `--status` is a read-only version-match
  probe. Use this on a pristine re-take or a brand-new VM. Config via the same
  `OPV_VM_USER`/`OPV_VM_IP`/… env as the backends.
- **Canonical automated transport:** the committed harness
  (`python -m scripts.opverify.visual.run_vm <journey>`, backend
  `backends_vm.py`) drives the guest via bare `curl.exe` over an SSH
  connection kept warm with `ControlMaster`/`ControlPersist` — only the first
  call pays the TCP+auth handshake (measured ~0.76s→~0.4s per round trip). Grab
  bytes come back raw (no base64), POST bodies ride ssh stdin. Prefer this for
  anything scripted.
- **Manual round trips:** run `curl.exe` directly over a multiplexed ssh —
  `ssh -o ControlMaster=auto -o ControlPersist=60 -o ControlPath=/tmp/opv-ssh-%r@%h:%p
  PC@192.0.2.252 'curl.exe -s http://127.0.0.1:8900/grab' > frame.png`. Reuse
  the same `-o ControlPath` on every call so the handshake is paid once. The
  legacy `scratchpad/vmps.sh` (`ssh … powershell -EncodedCommand`, UTF-16LE
  base64) remains only for ad-hoc PowerShell that isn't a simple HTTP call.
- **Admin token:** `scratchpad/opv_key.txt` = kernel `CLOTO_API_KEY`. Use for the
  `X-API-Key` header; transport base64 (`$ak =
  [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String("<b64>"))`) so it
  never appears in plaintext. **Never print it.**
- **Actuator agent** (session 1, no auth): `http://127.0.0.1:8900` — `GET /grab`
  (image/png), `POST /act` (`{kind:"click",x,y}` / `{kind:"type",text}` /
  `{kind:"key",key:"enter"}` / `hotkey` / `scroll`), `GET /health`, `POST /run`
  (`{path,args,env}` — env merges over the process env, e.g. to inject a known
  `CLOTO_API_KEY` when launching the app).
- **Kernel admin API** (needs `X-API-Key`): `http://127.0.0.1:8081` —
  `/api/system/health`, `/api/chat`, `/api/history`, `/api/llm/providers`,
  `/api/mcp/servers`, `/api/system/version`.
- **Screen** 1280×800.
- **Grab→read:** `ssh <multiplexed-opts> PC@192.0.2.252 'curl.exe -s
  http://127.0.0.1:8900/grab' > frame.png`, then Read the PNG (raw bytes, no
  base64 step). The legacy PS `[Convert]::ToBase64String($r.Content) | tr -d
  '\r\n' | base64 -d` still works if you're already in a PS snippet.
- To settle before a checkpoint, `POST /act` then sleep ~1.5s then `/grab`;
  with a warm `ControlPath` each of those is a cheap reuse, so separate calls
  are fine (no need to cram them into one PS to save handshakes).

### Chat payload / oracle (reference)
- `POST /api/chat` body: `{id, source:{type:"User",id,name}, target_agent:
  "agent.cloto_default", content, timestamp, metadata:{}}`. Returns `{}`
  (accept ≠ success).
- Success = a `ThoughtResponse` in `/api/history` (`{"data":[...]}`) with
  `data.source_message_id == id`, non-empty `data.content`, expected
  `data.engine_id`. When driving via the **GUI** you don't know the frontend's
  msg id — correlate by a unique **nonce** appearing in the response content.

### Security (hard rules)
- MUST NOT read/print/log any **LLM provider API key** contents. Only the boolean
  `has_key` may surface.
- The admin token is auth-only — use, never print.

### Return schema (compact — the only thing the orchestrator ingests)
```json
{
  "reached_target": true,
  "steps_taken": 0,
  "visual": {"rendered": true, "detail": "...", "defects": []},
  "kernel": {"ok": true, "engine_id": "...", "content_snippet": "..."},
  "diagnosis": "AGREE_PASS|FRONTEND_BUG|BACKEND_OR_HIDDEN|AGREE_FAIL",
  "key_frames": {"label": "<path>"},
  "notes": "anything the orchestrator should verify or that went wrong"
}
```
Do not paste PNG bytes or full history. Stop after a reasonable attempt if
blocked and return what you have with the blocker in `notes`.

## Diagnosis codes (shared vocabulary)
- **AGREE_PASS** — visual rendered ✓ and kernel confirms ✓.
- **FRONTEND_BUG** — kernel ✓ but the GUI didn't render it (soft/warn; the class
  the apex exists to catch).
- **BACKEND_OR_HIDDEN** — GUI shows it but the kernel has no record (hard-fail).
- **AGREE_FAIL** — neither.

## Known harness artifacts (not product bugs)
- **Keystroke fidelity:** the session-1 agent types via pyautogui `write()`, which
  mis-maps some shifted characters on a JP keyboard layout (observed 2026-07-14:
  `:` → `*` in the sent text). It does not affect alphanumeric nonces. For exact
  literal text, prefer clipboard paste over simulated keystrokes.

## VM state reset (snapshot rollback) — orchestrator op

Every apex / uninstall measurement starts from a **known clean state** by
rolling the VM back to a `clean-install-*` snapshot instead of reinstalling
(~16 s measured vs ~6 min for a scripted reinstall). This also structurally
removes the "residue of the previous experiment" class of mismeasurement
(observed 2026-07-30). Adopted with an earlier decision (an earlier decision).

```bash
ssh root@192.0.2.2 'qm rollback 104 clean-install-0-6-8-beta-2 && qm start 104'
# vmstate snapshot → the VM resumes the saved live session; qm start is a
# no-op safety for the non-vmstate case. Agent answers /health in ~16 s.
```

Wait for readiness the same way the verify script does — poll the actuator:
`ssh PC@192.0.2.252 'curl.exe -s -m 2 http://127.0.0.1:8900/health'` until
`"ok": true`.

Snapshot lineage on VM 104 (`qm listsnapshot 104` is authoritative):

- `pristine` — blank Win11 + OpenSSH only. For installer-diff verify runs.
- `agent-base-20260727` — OS + Python + opv_agent, ClotoCore UNINSTALLED,
  deliberate `%APPDATA%\cloto-system` residue kept as a Defender purge
  fixture. The durable base; app versions go stale, this does not.
- `clean-install-<version>` — ClotoCore `<version>` installed via NSIS and
  **never launched** (no `%APPDATA%` data), opv_agent listening, vmstate =
  live logged-in session 1. The default reset target for apex / uninstall
  journeys. Proxmox snapshot names cannot contain dots, so the version is
  dash-encoded (`0.6.8-beta.2` → `clean-install-0-6-8-beta-2`); the exact
  version is in the snapshot description.

**On a version change**: roll back to the current `clean-install-*`, silently
install the new installer over it (or uninstall + install for a fresh-path
measurement), re-verify the fingerprint (installed version, no unexpected
`%APPDATA%` state, `/health` ok), then `qm snapshot 104 clean-install-<new>
--vmstate 1` with a description recording the contents. Keep the previous
snapshot as a fallback until the new one has survived one journey.

**Look at the live state before rolling back.** `qm rollback` discards it —
there is no "current" snapshot to return to. On 2026-08-03 a rollback threw
away the only configured (onboarded + engine + provider key) environment,
which existed nowhere else because no `clean-install-*` snapshot carries app
data by definition. Rebuilding it is the recipe below, not a disaster, but it
is twenty minutes that a five-second look would have priced in. If a
configured state is worth keeping, snapshot it *first* under its own name.

## Rebuilding chat preconditions from a clean-install snapshot

`clean-install-*` is deliberately data-free, so any journey needing a working
engine starts with onboarding, no MCP servers and no provider key. All of it
except the wizard clicks is scriptable through the authenticated admin API —
which is also how the provider key stays out of the GUI, the actuator's
request bodies and every frame:

1. Launch with a known admin key — `POST :8900/run` with
   `{"path": "C:\\Program Files\\ClotoCore\\app.exe", "env": {"CLOTO_API_KEY": "…"}}`.
2. Install the engine — `POST :8081/api/marketplace/install`
   `{"server_id": "cerebras", "auto_start": true}`. It comes up `Connected`
   and creates the provider row (`has_key: false`).
3. Set the provider key — `POST :8081/api/llm/providers/cerebras/key`
   `{"api_key": "…"}`. Confirm with `GET /api/llm/providers`: `has_key: true`,
   `configured: true`.
4. Click through the wizard: name → **skip** the assistant preset (its batch
   install is minutes of work that step 2 already did) → tick the admin-key
   acknowledgement, and **do not press the regenerate button** — it would
   invalidate the `CLOTO_API_KEY` every later probe authenticates with.
5. Prove the kernel oracle before driving any GUI: `POST /api/chat` with a
   nonce, then find the `ThoughtResponse` in `/api/history` with the expected
   `engine_id` and the nonce as its content.

**A chat journey also needs a tall thread.** Messages posted to `/api/chat`
land in kernel history but *not* in the console's store (`/chat/{agent}/messages`),
so they do not fill the pane — send the filler turns through the GUI. Without
them the transcript is shorter than the viewport, every new turn is visible
however the scroll logic behaves, and `reply-rendered-without-scrolling`
passes while testing nothing. Click the composer before *each* send: focus is
not retained after Enter, so a loop that clicks once silently drops every turn
after the first.

Two quoting traps on the way in, both costly to rediscover: the guest's default
shell is PowerShell, so `curl.exe -H "…"` inside an `ssh` command line arrives
mangled — put headers in a curl config file (`curl.exe -K C:\opv\auth.cfg`,
one `header = "X-API-Key: …"` line per header, plus a second file adding
`Content-Type: application/json` for POSTs, which the kernel requires). And
`--data-binary @-` over stdin is parsed by PowerShell before curl sees it; copy
the body to the guest and use `-d @C:\opv\body.json`.

## `danger-zone-purge` — the outcome journey (destructive)

`danger-zone` stops at the preview. `danger-zone-purge` presses the button and
then asserts the machine: the app ends, the detached helper writes a clean
report, and a sweep of the OS finds nothing left. It runs at **tier 4**, because
the narrower tiers deliberately keep the ARP entry and the vendor key, so
"residue is zero" only means anything at the widest scope.

Three things it needs, each of which fails the run loudly rather than quietly
passing:

1. **A launch with the debug port open and a known admin key.** Targets are
   resolved live over CDP (`cdp.py`), and gate 3 asks for the key:
   `POST :8900/run` with `{"env": {"CLOTO_API_KEY": "…",
   "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS": "--remote-debugging-port=9222"}}`,
   and `OPV_API_KEY` set to the same key for the harness.
2. **An install with data to remove.** The journey refuses to build on an empty
   tier-4 plan, and `PurgeReportClean` fails a report whose entries are all
   `absent` — a purge with nothing to purge verifies nothing. A *fresh* install
   is not enough on its own: it comes up on the onboarding carousel, which the
   first step rejects by name (the main window is what the rest is written
   against). Walk onboarding first — name → **skip** the assistant preset → tick
   the admin-key acknowledgement, and **do not press regenerate**.
3. **A live visual oracle.** `OPV_ASSESSOR=handshake` is enforced: replayed
   verdicts would agree with anything while the uninstall proceeded.

Restoring afterwards is a reinstall, not only a snapshot rollback — the
installer is already on the guest at `C:\opv\` after the first run.

Two lessons from building it, both about *targeting* rather than the product:

- **The window is not what an element is visible within.** A control scrolled
  just past the bottom of the modal's scroll pane still has a rect inside the
  window; clicking it lands on the backdrop and closes the modal, and the run
  then reads as "the app refused to uninstall". `cdp.py` intersects the
  clipping ancestors and hit-tests with `elementFromPoint`, so a target is only
  actionable when a click would actually reach it.
- **Ask the visual oracle only what the frame can answer.** "Are all four
  checkboxes checked" is a question about the scroll position; "is the widest
  one checked" is a question about the app. The scope really widening is the
  kernel probe's job, and it answers authoritatively.

## Journey preconditions (learned 2026-07-31, danger-zone round 3)

Every committed journey assumes it starts from the **plain main window — no
settings modal open**. A run started with the modal already up is invalid from
step one: the journey's own `open-settings` click lands on the modal backdrop,
which CLOSES the modal, and every later step then fails as `frontend_bug`
against the bare main window (observed as 6 ops flipping to failing at once —
a signature worth recognizing: near-total frontend_bug across successive steps
usually means a dirty starting state, not six real GUI defects).

Two related facts about the SETTINGS modal:
- **Esc does not close it.** Close it by clicking the backdrop (e.g. (640,700),
  below the modal) or its X button — verify with a grab afterwards.
- Reopening the modal resets its pane state (component unmounts on close), so
  a fresh open is a fresh danger-zone card — collapsed plan, top scroll.
