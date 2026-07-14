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
- **PowerShell-on-VM helper:** `scratchpad/vmps.sh` — `printf '%s' "$PS" | bash
  vmps.sh` runs `$PS` on the guest via `ssh PC@192.0.2.252 powershell
  -EncodedCommand` (UTF-16LE base64, quote-immune). Guest stdout returns on
  stdout.
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
- **Grab→read:** PS `[Convert]::ToBase64String($r.Content)` for `/grab`, then
  `| tr -d '\r\n' | base64 -d > frame.png`, then Read the PNG.
- Chain several `/act` + a final `/grab` in one PS (with `Start-Sleep
  -Milliseconds 1500` to settle) to save round-trips.

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
