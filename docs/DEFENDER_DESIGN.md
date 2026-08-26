# Lifecycle Defender — Unified Health, Repair, and Clean-Uninstall Subsystem

**Status**: Approved (2026-07-17) — Phase 1 (receipt / check registry / doctor / updater-guard / advisories) and Phase 2 (repair verb / clean-update first-boot) implemented; Phase 3 pending
**Related**: `ONBOARDING_MODERNIZATION_DESIGN.md` (admin-key handover is the prerequisite for the sudo-mode gate in §7), `RELEASE_PIPELINE_DESIGN.md` (signed manifest carries the advisory feed), `INSTALLER_DISTRIBUTION.md`

---

## 1. Motivation

ClotoCore currently has no single component that knows the full footprint of an
installation. This produces four user-visible gaps, each observed in practice:

| Gap | Evidence |
| --- | --- |
| Uninstall is never clean | CLI `uninstall` removes only the prefix (`installer.rs`); NSIS deliberately preserves `cloto-system` (bug-386); the user-data dir (DBs, `seal.key`, `mcp-servers/` + venv, `models/`, `voicevox/`) survives every uninstall path; WebView data (`EBWebView`, WebKit caches) is cleaned by **no** path |
| Health checks stop at DB + venv | `db/health.rs` covers 8 checks; nothing covers MCP liveness, port conflicts, `.env` validity, DB file integrity, or data-dir drift |
| Boot-fatal corruption is undiagnosable | A non-SQLite file at the data path kills the kernel with `code: 26, file is not a database` **before** any pool-based health scan can run (observed on VM verification, 2026-07-14) |
| Update preconditions are unchecked | Interrupted swaps leave `.old`/`.new` remnants; exe-location write permission is only discovered at swap time; a stale legacy data dir can make a successful update *look* broken at next boot |

The two originally separate proposals — clean uninstall and kernel health check —
are two views over the same missing data structure: **a ledger of everything
ClotoCore has placed on the machine**. Health is the read/verify pass over that
ledger; clean uninstall is the destructive pass over the same ledger. Keeping
them unified makes "health knows about it but uninstall forgets it" drift
structurally impossible (the 2026-06-10 uninstall silent no-op bug was exactly
this class: two divergent enumeration logics).

## 2. Architecture — three canonical sources, three verbs

```
crates/core/src/defender/
  footprint.rs   — install receipt: the ledger (canonical source 1)
  advisories.rs  — known-issue feed evaluation (canonical source 2)
  checks.rs      — check registry (canonical source 3)
  repair.rs      — the repair verb (non-destructive fixes)
  purge.rs       — purge-plan generation; execution is plan-bound and separate
```

| Verb | Semantics | Destructive? |
| --- | --- | --- |
| `scan` | Evaluate every registered check against the receipt; report | Never |
| `repair` | Fix `fix_capable` findings **non-destructively** (regenerate, rename-quarantine, deduplicate registrations) | Never deletes user data |
| `purge` | Execute an explicit, user-approved purge plan | Yes — only via the gates in §7 |

The separation between `repair` and `purge` is an invariant, not a convention:
`repair` has no code path that deletes user data, and `purge` is reachable only
from the uninstall flow. This exports the asymmetric-safety principle (repair
is always safe to run; destruction always requires explicit human intent).

## 3. Canonical source 1 — install receipt

`<data_dir>/installed.json`: written by `clotocore install`, the desktop
first-run, and **every operation that mutates the footprint** (MCP server
install/uninstall, model download, update swap, service registration).

```json
{
  "receipt_version": 1,
  "app_version": "0.6.8",
  "installed_at": "2026-07-17T00:00:00Z",
  "entries": [
    { "id": "binary",        "kind": "file",    "path": "/opt/cloto/clotocore" },
    { "id": "env",           "kind": "file",    "path": "/opt/cloto/.env", "secret": true },
    { "id": "data_dir",      "kind": "dir",     "path": "~/.local/share/cloto-system" },
    { "id": "mcp:cscheduler","kind": "dir",     "path": ".../mcp-servers/cscheduler" },
    { "id": "webview_data",  "kind": "dir",     "path": "%LOCALAPPDATA%/<bundle-id>/EBWebView" },
    { "id": "service",       "kind": "service", "name": "cloto" }
  ]
}
```

Design points:

- **Receipt is authoritative; scanning is fallback.** Heuristic discovery of
  well-known locations (legacy `cloto-system` dirs, old NSIS uninstall keys,
  `.old`/`.new` swap remnants, duplicate service registrations) exists only to
  cover installs that predate the receipt. Once a receipt exists, enumeration
  is deterministic.
- Receipt updates are best-effort and non-fatal: a failed receipt write must
  never fail the operation it records. `doctor` reports receipt staleness
  (entries whose paths no longer exist, paths present but unrecorded).
- The receipt itself is listed in the receipt (it is part of the footprint).

## 4. Canonical source 3 — check registry and `clotocore doctor`

Registry-based checks, following the pattern proven in CPersona v2.4.37:

```rust
struct Check {
    name: &'static str,
    scope: CheckScope,      // Db | Files | Update | Runtime
    base_severity: Severity,
    fix_capable: bool,
    run: fn(&CheckCtx) -> CheckResult,
}
```

- The existing 8 checks in `db/health.rs` (db_connection, 3× orphaned rows,
  audit_chain, venv_exists, venv_python_version, venv_repair) migrate into the
  registry unchanged. `GET /api/health/scan` and `POST /api/health/repair`
  become thin wrappers over the registry — API shape is preserved.
- **New checks (Phase 1 set)**: DB file integrity (SQLite header +
  `PRAGMA integrity_check` — *file-level, no pool required*), legacy data-dir
  drift (`cloto-system` remnants that the current binary would misread),
  `.env` validity (key present, referenced `${VAR}`s resolvable), port 8081
  availability, receipt staleness, and the updater-guard set (§6).
- **`clotocore doctor` runs pool-free.** The single most valuable diagnosis —
  "the DB is corrupt and the kernel will fatal on boot" (code 26 class) —
  must work when the kernel *cannot* boot. `doctor` therefore opens nothing
  through sqlx; file-level checks come first, pool-dependent checks are
  skipped with an explicit "kernel not reachable" marker.

## 5. Canonical source 2 — advisory feed (known-issue → update recommendation)

The defender does **not** discover unknown bugs (that is CI / opverify /
release-pipeline territory). It maps *known* issues to the installed version
and corroborates them locally:

- `qa/issue-registry.json` already records verified bugs. The release pipeline
  extracts entries into an `advisories` block in the signed updater-feed
  manifest (the same manifest `install.sh` and the updater already fetch —
  no new endpoint, tamper-resistant by existing signing).

```json
"advisories": [{
  "bug_id": "bug-386",
  "severity": "high",
  "affected": ">=0.6.0 <0.6.7",
  "fixed_in": "0.6.7",
  "symptom_check": "legacy_data_dir_drift",
  "summary": "legacy cloto-system install can break boot (code 26)"
}]
```

- Layer 1 (deterministic): installed version ∈ `affected` semver range →
  advisory applies.
- Layer 2 (local corroboration): `symptom_check` names a registry check;
  if it fires, the advisory is reported as *manifesting*, not just *possible*.
- Output is a **recommendation only** ("fixed in 0.6.8 — run
  `clotocore update`"), surfaced in `doctor` output, the Health section, and
  `/api/health/scan`. **The defender never auto-updates** (HITL invariant).
  Recommendations are channel-coherent: stable users are pointed at stable
  `fixed_in` versions only.

## 6. Updater integration

**Updater-guard checks** (registry `scope: Update`): exe-location write-probe
(will the swap succeed?), interrupted-swap remnants (`.new` present → resume
or roll back deterministically; `.old` present → safe cleanup), disk space,
updater config validity, version coherence (running process vs on-disk binary
— "updated but not restarted").

**Clean update — intelligence rides in the incoming binary.** The update path
gains a post-update phase: on first boot of the new version, the defender runs
receipt-driven migration and residue cleanup (legacy dirs quarantined by
rename, swap remnants removed, receipt rewritten). The outgoing version is
never required to cooperate beyond being replaceable. This generalizes the
bug-386 NSIS pattern (new installer cleans up the old install) to all
platforms and all install paths.

**Rescue layering** (honest boundary): a binary too broken to run cannot run
its own doctor. The recovery path for that case is external — `install.sh` /
`install.ps1` re-install a healthy binary (checksum-verified, already
implemented), and *that* binary's first boot performs the cleanup. "Defending
the updater" is therefore a two-layer guarantee: in-binary defender + external
install script.

**Safety**: all migration is backup-first — quarantine by rename
(`<name>.bak-<ts>`), never delete; quarantined items are reported as purge
*candidates* for explicit approval later.

**Phase 2 implementation note (deviation, 2026-07-17)**: legacy data-dir
quarantine is deliberately part of *neither* the automatic first-boot phase
*nor* the repair verb. A stray-looking data dir can be the **active** dir of
a coexisting install (dev layout and a production desktop install on the
same machine), and no local heuristic can distinguish the two reliably.
Legacy drift therefore stays report-only (`legacy_data_dir_drift`) until the
Phase 3 purge plan, where removal is explicit, enumerated, and
user-approved. The first-boot phase runs receipt convergence only; the
`.old` rollback binary next to the exe is likewise left in place at first
boot (it *is* the backup-first quarantine) and is removed by the explicit
repair verb.

## 7. Complete uninstall

### Enumeration and scope tiers

The purge plan is generated from the receipt (plus legacy-scan findings),
shown to the user as a concrete list with real paths and sizes — not an
abstract warning. Scope tiers, conservative by default:

1. Application only (binary / app bundle, service, autostart) — default
2. \+ user data (`data_dir`: DBs, seal.key, attachments, avatars)
3. \+ heavy assets (models, voicevox) and MCP servers + venv
4. \+ everything (WebView data, registry keys, receipt itself)

Tier 4 is where the `data_dir` *container* and the receipt go; tiers 2 and 3
name paths inside it. A plan therefore never lists a path that sits inside
another listed directory — the child is reported as covered by its parent so
the size total counts each tree once.

A plan is a UTF-8 JSON file, and a path is not text: on Linux a file name is
bytes, so a path can exist that the plan cannot write down. Rendering it
lossily is the dangerous option, because the mangled string round-trips to
itself — nothing downstream can tell it apart from a faithful path, the
executor stats it, finds nothing, and reports the entry as already gone while
the directory is still there. Such a path is therefore refused at the two
seams where a path becomes a string (the receipt and the plan's own
candidates), listed among the plan's skipped entries, and stated in the plan's
notes. The uninstall says it did not remove it, which is true, instead of
claiming a success it did not have.

Receipt entries are classified by id, and an id the running binary does not
recognise **falls back to tier 2, never tier 1**. A future version that
records a new kind of footprint is then invisible to the default uninstall
rather than deleted by it: an unclassified entry should survive a scope the
user did not knowingly widen. The classification lives in code
(`defender::purge::classify`), which is where new ids are added; enumerating
every id here would drift.

### UI — Settings → Health → Danger Zone

The Health section (`HealthSection.tsx`) already exists; the Danger Zone is
appended at the bottom (established Settings→Danger-Zone grammar). Three
gates, in order:

1. **Dry-run enumeration** — the resolved purge plan rendered from the receipt
2. **Scope checkboxes** — tier selection, default = tier 1
3. **Sudo mode** — manual admin API key entry (see below)

The dashboard route is the primary implementation for *all* platforms: one
flow, one plan format, one confirmation UI; OS-specific work stays inside the
existing `crate::platform` abstraction. On macOS — which has no uninstaller
artifact at all — this route is the only proper uninstall path. The CLI
(`clotocore uninstall --plan [--tier N] [--prefix P] [--json]` for the dry run;
execution flags land with the executor) is a thin wrapper over the same plan
generator, covering headless installs. NSIS gains at most a checkbox that
invokes the same plan.

### Authentication — two distinct layers

- **Server-side (the real boundary)**: `POST /api/system/uninstall` requires
  `X-API-Key` (admin auth, same class as shutdown). Non-negotiable; kills
  CSRF / foreign-origin invocation.
- **UI-side (sudo mode, a deliberateness gate)**: the dialog requires manually
  typing the admin key — GitHub-sudo-mode pattern. Because the desktop webview
  can fetch the key programmatically (`getAutoApiKey`), this layer is
  explicitly documented as *deliberateness + unattended-session hardening*,
  **not** a security boundary. The dialog links the legitimate retrieval path
  (Settings → Security → reveal key; see `ONBOARDING_MODERNIZATION_DESIGN.md`).

### Self-deletion choreography

A running app cannot delete itself (locked exe on Windows, open DBs, live
WebView profile). The handoff pattern already has an in-repo precedent: the
hidden `SwapExe` subcommand used by the updater ("perform exe swap after
parent exits").

```
dashboard confirm → POST /api/system/uninstall (X-API-Key)
kernel: write purge plan to a fresh temp dir → copy own binary beside it
      → ask for UAC elevation where the plan needs it (Windows)
      → spawn detached:
        clotocore purge-exec --plan <file> --pid <parent> --root <r>…
      → drain MCP servers → close DB pool → clean app exit
helper (from temp): wait for parent pid → execute plan
      → remove service/autostart/uninstall keys → remove everything in plan
      → write the report next to the plan
```

The helper executes **only** what the plan file lists — the plan is the
capability boundary; the helper has no enumeration logic of its own.

Three properties of this sequence are load-bearing, and each of them is a
correction to the sketch it replaces.

**The elevation prompt comes before the exit, not inside the helper.** A prompt
raised after the app is gone appears with nothing on screen to explain it, and a
refusal has nobody left to report to. Asking while the kernel is still up costs
nothing — the helper blocks on the parent pid either way — and turns a declined
prompt into a `409` with the app still running and nothing removed. Whether the
plan needs elevation at all is decided from the plan (a service, an `HKLM`
uninstall key, or any path outside the user's own tree), not from a permission
probe, which would race the state it measures.

**The containment roots travel on the command line.** The plan is a file, and
the helper reads it after the kernel wrote it — elevated, on Windows. If the
plan's contents were the only thing deciding what gets deleted, then plan-file
integrity would be the whole security boundary, and a same-user process that
rewrote the file between write and read would be handing an elevated process a
list of things to remove. The lexical floor (absolute, not a filesystem root, no
`..`) does not close this: `/etc/passwd` satisfies all three. So the kernel
states the allowed directory trees as `--root` arguments, which a process that
can only rewrite files cannot reach, and the executor refuses any path outside
them. A helper invoked with no roots derives them from its own environment
(`data_dir`, home, the platform dirs, the install prefix, `/Applications`) —
never from the plan it was handed. An empty root set refuses everything rather
than allowing everything.

**The report is a file, not stdout.** A detached helper's stdout goes nowhere:
the process that spawned it has exited. The run is written to
`<plan>.report.json` before the exit status is decided, so a partial or refused
uninstall leaves a complete account behind — which is also what makes "fix the
failure and re-run `purge-exec --plan <path>`" a real instruction rather than a
suggestion to start over.

That instruction only holds if the plan outlives the run, and the receipt
outlives the removal that reads it. Deepest-first ordering would take the
data-directory container — and the receipt inside it — before shallower entries
like the install prefix or the app bundle, so a failure on one of those would
leave a second `uninstall --execute` rebuilding its plan from a receipt that no
longer exists, silently naming less than the first attempt did. The entry
holding the receipt is therefore removed last; nesting has already collapsed by
then, so the surviving entries are disjoint and the order is free to say so.
The in-process path saves its plan into a staging directory outside the tree it
is about to remove, before removing anything, and writes its report beside it
afterwards — the same two artifacts the detached path produces, so an
interrupted run of either is resumed the same way.

Removing a *running* installation is a separate hazard from removing a
privileged one. On Unix the deletions succeed against open files, the live
process keeps writing to unlinked inodes, and it recreates the receipt and data
directory on its way out: the uninstall reports success and the installation is
still there. The kernel therefore records its pid in `data_dir/kernel.pid` while
it runs, and the in-process path (`clotocore uninstall --execute`) refuses while
a live kernel holds it, pointing at the detached flow instead. The record is
advisory — a stale file whose pid is gone counts as no holder — because it
answers a question for a human-driven operation, not a concurrency invariant.

On a desktop install the kernel lives inside the GUI binary, so *that* binary is
what gets copied and re-launched as `purge-exec`. It honours the hidden
subcommand before any window is created; otherwise the copy would start a second
app instead of executing the plan.

### Boundaries (stated honestly in user-facing docs)

- Third-party MCP servers may write outside their install dir (own caches,
  configs). ClotoCore cannot track arbitrary side effects; cleanup of declared
  paths only. *Future mitigation*: a `data_paths` declaration field in the
  registry entry, letting well-behaved servers opt into full cleanup.
- OS-level traces (prefetch, event logs, MRU) are out of scope and never
  promised.

## 8. Safety invariants (summary)

1. `scan` and `repair` never delete user data. `repair` fixes by regeneration,
   rename-quarantine, or deregistration only.
2. `purge` is reachable only through the uninstall flow: plan file + admin key
   + explicit multi-gate confirmation. No auto-purge, ever.
3. All migration/cleanup in the update path is backup-first (rename, not
   delete).
4. The defender never applies an update by itself — advisory output is a
   recommendation (HITL).
5. The detached helper is plan-bound: nothing outside the plan file is
   touched.
6. A plan file is UTF-8 JSON. A path that does not survive that encoding is
   refused and reported as refused — never acted on, and never counted as
   already removed.
7. The entry holding the install receipt is removed last, and every run saves
   its plan and its report outside the tree it removes, so a partial uninstall
   can be finished with `purge-exec --plan <file>` rather than re-enumerated
   from a ledger it has already deleted.

## 9. Phasing

| Phase | Deliverable | Value if it ships alone |
| --- | --- | --- |
| 1 | receipt + check registry + `clotocore doctor` (read-only) + updater-guard + advisory evaluation | pre-boot diagnosis of boot-fatal states; update-failure prevention; known-issue recommendations |
| 2 | `repair` verb (absorbs existing repair; adds quarantine-based fixes) + clean-update first-boot phase | self-healing installs |
| 3 | purge plan + `purge-exec` helper + Danger Zone UI + CLI/NSIS wrappers | complete uninstall on all platforms |

Each phase consumes the previous phase's ledger unchanged; no rework between
phases. opverify gains an E2E scenario per phase (doctor
verdict on a seeded-corruption VM; GUI-driven full uninstall → zero-residue
assertion).

## 10. Open questions

- Naming: "defender" is the working name; `doctor` is the CLI surface. Final
  user-facing naming to be settled at Phase 1 review.
- Receipt schema versioning / migration policy (start at `receipt_version: 1`,
  additive-only until Phase 3).
- Whether advisory evaluation should also run scheduled (daily) or only on
  scan/boot — default: on scan + boot, no background network.
