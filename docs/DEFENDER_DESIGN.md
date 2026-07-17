# Lifecycle Defender — Unified Health, Repair, and Clean-Uninstall Subsystem

**Status**: Draft (design approved 2026-07-17)
**Tracking**: an earlier decision (an earlier decision, 0.6.8 line)
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
  plan.rs        — purge-plan generation & execution
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

## 7. Complete uninstall

### Enumeration and scope tiers

The purge plan is generated from the receipt (plus legacy-scan findings),
shown to the user as a concrete list with real paths and sizes — not an
abstract warning. Scope tiers, conservative by default:

1. Application only (binary / app bundle, service, autostart) — default
2. \+ user data (`data_dir`: DBs, seal.key, attachments, avatars)
3. \+ heavy assets (models, voicevox) and MCP servers + venv
4. \+ everything (WebView data, registry keys, receipt itself)

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
(`clotocore uninstall --purge-data --dry-run`) is a thin wrapper over the same
plan generator, covering headless installs. NSIS gains at most a checkbox that
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
kernel: stop MCP servers → close DB pool → write purge plan to temp
      → copy own binary to temp → spawn detached:
        clotocore purge-exec --plan <file> --pid <parent>   (hidden subcommand)
      → clean app exit
helper (from temp): wait for parent pid → execute plan
      → UAC elevation on Windows where required (Program Files, ProgramData)
      → remove service/autostart/uninstall keys → remove everything in plan
```

The helper executes **only** what the plan file lists — the plan is the
capability boundary; the helper has no enumeration logic of its own.

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

## 9. Phasing

| Phase | Deliverable | Value if it ships alone |
| --- | --- | --- |
| 1 | receipt + check registry + `clotocore doctor` (read-only) + updater-guard + advisory evaluation | pre-boot diagnosis of boot-fatal states; update-failure prevention; known-issue recommendations |
| 2 | `repair` verb (absorbs existing repair; adds quarantine-based fixes) + clean-update first-boot phase | self-healing installs |
| 3 | purge plan + `purge-exec` helper + Danger Zone UI + CLI/NSIS wrappers | complete uninstall on all platforms |

Each phase consumes the previous phase's ledger unchanged; no rework between
phases. opverify (an earlier decision) gains an E2E scenario per phase (doctor
verdict on a seeded-corruption VM; GUI-driven full uninstall → zero-residue
assertion).

## 10. Open questions

- Naming: "defender" is the working name; `doctor` is the CLI surface. Final
  user-facing naming to be settled at Phase 1 review.
- Receipt schema versioning / migration policy (start at `receipt_version: 1`,
  additive-only until Phase 3).
- Whether advisory evaluation should also run scheduled (daily) or only on
  scan/boot — default: on scan + boot, no background network.
