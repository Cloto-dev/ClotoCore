# ClotoCore Development Rules

## Mandatory Reads

Read these before making changes. Do not summarize — read the actual files.

- **`docs/PROJECT_VISION.md`** — Core identity, competitive positioning, target users
- **`docs/ARCHITECTURE.md`** — System architecture, security framework, design principles
- **`docs/MGP_SPEC.md`** — MGP protocol (strict MCP superset). Servers: [clotohub-servers](https://github.com/Cloto-dev/clotohub-servers) (private; formerly cloto-mcp-servers)
- **`docs/DEVELOPMENT.md`** — 8 critical guardrails (security, cascading, state, storage, UI/UX, physical safety, external processes, privacy)

If a proposed change conflicts with any of these, flag it before proceeding.

## Commands

- Rust lint: `cargo clippy --workspace --exclude app --all-targets -- -D warnings`
- Rust format: `cargo fmt --all -- --check`
- Rust test: `cargo test --workspace --exclude app`
- Dashboard lint: `cd dashboard && npx biome check src/`
- Dashboard format: `cd dashboard && npx biome format --write src/`
- Dashboard build: `cd dashboard && npm run build`
- Dev launch: `cd dashboard && npx tauri dev` (starts Vite + Tauri together. Do NOT run `app.exe` directly — the debug build's devUrl points to the Vite dev server)
- Release build: `cd dashboard && npx tauri build` (`cargo build --release -p app` is prohibited)
- Bug verify: `bash scripts/verify-issues.sh`
- Test ratchet: `bash scripts/check-test-count.sh`
- Language packs: `python3 scripts/check-language-packs.py` (blocking CI gate — every key in `dashboard/src/locales/en/` must exist in each pack under `dashboard/src-tauri/resources/`. Adding UI strings means adding the ja values in the same PR; `fallbackLng` hides the gap at runtime and the component tests only see English)

**MUST (pre-push lint for Rust changes):** before pushing any change under `crates/`, run **both** `cargo fmt --all -- --check` **and** clippy locally — the CI **Lint** job gates on both and a formatting/clippy diff fails the PR. Running the dashboard `biome` check alone does **not** cover the Rust Lint job. The clippy command above (`--all-targets`) is stricter than CI; to reproduce CI exactly use the flags in `.github/workflows/ci.yml` (Lint job: no `--all-targets`, plus an `-A clippy::*` allowlist). `.github/workflows/ci.yml` is the authoritative gate.

## SQLx Migration Rules (CRITICAL)

`.gitattributes` enforces **CRLF** line endings for `crates/core/migrations/*.sql`.
sqlx hashes each migration file and stores the checksum in `_sqlx_migrations` on first
apply, then rejects a modified file on the next startup with
`migration ... was previously applied but has been modified` (FATAL — kernel won't boot).

Claude's `Write` tool produces **LF** line endings. If a migration is written with LF,
applied once by `cargo run`/`tauri dev`, then later normalized to CRLF (by git, an IDE
save, or manual conversion), the checksum mismatches on the next build and the kernel
refuses to start.

**Always convert a new migration to CRLF before any `cargo build` / `tauri dev`:**

```
perl -i -pe 's/\r?\n/\r\n/' crates/core/migrations/YYYYMMDDHHMMSS_name.sql
```

If you already hit the FATAL (checksum mismatch) in a dev DB, recover with:

```
sqlite3 target/debug/data/cloto_memories.db \
  "DELETE FROM _sqlx_migrations WHERE version=<version>; \
   ALTER TABLE <table> DROP COLUMN <column_added_by_migration>;"
```

Then restart the kernel — sqlx will re-apply the migration and record the current
checksum. Only needed in dev; users installing via release builds never hit this
because the migration file is embedded once at package time.

### Do NOT delete the dev DB

`cloto_memories.db`, `cloto.db`, `*.db-wal`, and `*.db-shm` must not be `rm`'d
or truncated — even preemptively, even in the name of avoiding a potential
checksum mismatch. Deletion destroys chat history, episode memory, registered
MCP servers (including custom/dynamic ones that cannot be re-derived from
`registry.json`), `mcp_access_control` grants, local embedding namespaces,
cron jobs, audit logs, and every piece of agent-side state that has no other
persistent source.

A freshly authored migration that has never been `cargo run`'d cannot trigger
a checksum mismatch — the `_sqlx_migrations` row it would conflict with
doesn't exist yet. Preemptive `rm` "to be safe" is therefore never correct.

If recovery is unavoidable, use the targeted partial-repair form above
(`DELETE FROM _sqlx_migrations WHERE version=X;` + `ALTER TABLE t DROP COLUMN`)
— it surgically rolls back exactly one migration while preserving every
other row. Never escalate to `rm` without explicit user confirmation.

Incident: 2026-04-21, beta.13 quirks-column session. A preemptive delete
destroyed ~30 minutes of state (mcp_access grants, custom `x-browser` +
`github-bridge` registrations, `x_style_reference` vectors) that had to be
rebuilt via Setup Wizard + API re-registration + grants union.

## Bug Verification (Anti-Hallucination)

`qa/issue-registry.json` + `scripts/verify-issues.sh` mechanically detect hallucinated AI bug reports. MUST run `bash scripts/verify-issues.sh` in each of these cases:

1. **After adding a new bug entry** — expect `[VERIFIED]` (grep confirms the pattern actually exists in the file)
2. **After marking an entry fixed** (`"status": "fixed"`, `"expected": "absent"`) — expect `[FIXED]`
3. **Before claiming a fix** in a summary / commit message / review reply — never write "this bug is fixed" unless this session's verify output shows `[FIXED]` for that issue

Reading the output (exit 0 = all OK / 1 = stale or error): `[VERIFIED]` pattern present ✅ / `[FIXED]` pattern absent ✅ / `[STALE]` expected present but gone (update the registry) / `[UNFIXED]` expected absent but still present (fix incomplete) / `[ERROR]` missing file or broken JSON (investigate).

Automation: a PostToolUse hook auto-runs cases (1) and (2) whenever `qa/issue-registry.json` is edited and injects the verify output; case (3) cannot be hook-detected, so it remains a text rule. `.githooks/pre-commit` blocks registry-touching commits on `[STALE]` / `[UNFIXED]` / `[ERROR]`; `--no-verify` only after deliberate review.

`scripts/verify-issues.sh` is **read-only infrastructure** — do not modify it without explicit user approval.

- Source of truth: `qa/issue-registry.json`
- Scope: bugs where code-level evidence is needed (e.g., AI-discovered bugs that could be false positives). Not every fix needs an entry.
- **Enable pre-commit blocker (once per clone)**: `bash scripts/install-hooks.sh` — sets `core.hooksPath=.githooks`. Baseline: currently has pre-existing `[ERROR]` / `[STALE]` entries (bug-265, bug-333, bug-349, bug-368 as of 2026-04-24) so registry-touching commits will block until those are resolved or `--no-verify` is used intentionally.

## Agent Config Rules

- All agent config operations MUST be deferred (pending state → apply on Save)
- Direct mutation API calls (upload, delete, update) are PROHIBITED outside `handleSave`
- Cancel/Back MUST discard all pending changes without API calls
- Pattern: event handler → set pending state only, `handleSave` → execute all pending
- Reference implementation: `AgentPluginWorkspace.tsx`

### Exception: Confirm-modal destructive actions

Destructive actions that are already gated by a dedicated Confirm modal
(optionally password-protected) are exempt from the deferred pattern and
MAY execute immediately on confirm. Current exempted handlers:

- `AgentTerminal.tsx` — Delete agent (`handleDeleteConfirm`)
- `SecuritySection.tsx` — Invalidate API key (`handleInvalidate`)
- `PowerToggleModal.tsx` — Toggle agent power (`handleConfirm`)

Rationale:

- The modal itself provides the cancellation opportunity, so the pending
  state would be redundant.
- A pending Delete would introduce a "cancel then actually delete" flow
  that is more error-prone than a direct confirm.

Rule scope: **non-destructive** config edits (rename, persona, engine,
MCP access, avatar, VRM) still MUST follow the deferred pattern.

## Dashboard UI Rules

- **Min text size**: `text-[9px]`. Never `text-[8px]` or smaller.
- **Min text color**: `text-content-tertiary`. Never `text-content-muted` for readable text.
- **Hover borders**: `hover:border-brand` (interactive), `hover:border-red-500` (destructive). Full opacity.
- **Tailwind CSS**: The dashboard uses pre-compiled CSS (`src/compiled-tailwind.css`), NOT JIT. When adding or changing Tailwind utility classes in JSX, you MUST regenerate: `cd dashboard && npx tailwindcss -i src/index.css -o src/compiled-tailwind.css`. New classes will not take effect without this step.

### Glass / Card Surface Policy

The dashboard has two distinct surface patterns. Pick the right one for the role.

- **Primary content cards** (agent cards, memory cards, marketplace cards, chat header controls, anything the user directly interacts with as a "tile"):
  Use the `card-solid` component class (defined in `src/index.css` `@layer components`).
  Expands to: `bg-surface-primary/50 shadow-sm hover:shadow-md transition-all duration-300`.
  Callers add `border border-edge`, padding, `rounded-*`, and hover color on top.
  Reference: `AgentTerminal.tsx:362`.

- **Functional UI surfaces** (panels, inputs, dropdowns, bars, sidebars, modals, nav buttons, empty-state containers):
  Use the existing `bg-glass*` + `backdrop-blur-*` utilities.
  - `bg-glass` (60% alpha): default panel background.
  - `bg-glass-subtle` (80% alpha, lighter): prominent glass buttons and nav bars.
  - `bg-glass-strong` (80% alpha, darker): input fields, hover states over solid containers.
  Reference: `AgentPluginWorkspace.tsx:250` (glass button), `KernelMonitor.tsx:16` (glass panel).

- **Do not mix** the two. `bg-surface-primary/50` must not appear on functional UI, and `bg-glass*` must not appear on primary content cards. If in doubt, grep for a nearby equivalent use and follow its pattern.

## Git Rules

> Inherits: `../CLAUDE.md` — shared Git Rules section (author, English commits, push = explicit instruction only).

- Do NOT create git tags manually — use `gh release create`
- Binaries distributed exclusively via [GitHub Releases](https://github.com/Cloto-dev/ClotoCore/releases)

## Release Rules

- Bump version in `Cargo.toml`, `dashboard/package.json`, `dashboard/src-tauri/tauri.conf.json`
- Release notes: cumulative from previous release (`gh release list` to find it)

### VM verify policy (every release, adopted 2026-07-13)

`scripts/proxmox-windows-verify.sh` (Proxmox VM 104, ~6 min automated NSIS
upgrade verify) is gated on the **release event**, not on NSIS-touching diffs —
the diff detector (`nsis-touching-detect.yml`) remains as an extra per-PR
signal, but a quiet detector never waives this policy:

- **Stable release (`X.Y.Z`)**: VM verify is **MUST** before publish, no
  approval needed to run it. A PASS report is a publish precondition. On the
  first stable after updater-path changes (feed / signing / channel
  resolution), additionally run a one-off live auto-update smoke in the VM
  (install previous stable → let the real feed drive the update).
- **Pre-release (`alpha.n` / `beta.n` / `rc.n`)**: when requesting publish
  approval, **explicitly ask the maintainer whether to run VM verify** for
  this release. If the maintainer waives it, record the skip decision in one
  line (release notes or commit message) so the audit trail shows a deliberate
  choice, never a silent fall-through.

Rationale: 0.6.8-beta.2 (2026-07-12) shipped a full distribution-pipeline
overhaul with zero VM verification because the old trigger (NSIS-touching
diff) never fired — condition-based triggers silently miss risk that moves
elsewhere; event-based triggers cannot be missed.

### opverify quality gate (stable cut, adopted 2026-07-14)

`scripts/opverify/` drives a broad catalog of **real operations to success**
against a live headless `clotocore` kernel over its HTTP admin API (an earlier decision) — the operation-coverage complement to the boot-only `--smoke` and the
installer-diff VM verify above. See `scripts/opverify/README.md`.

- **Before a stable cut (`X.Y.Z`)**: opverify is **MUST**. At minimum the
  zero-secret local tier must pass —
  `python3 -m scripts.opverify.run --target local --slice phase0` (exit 0 is the
  gate). As the VM tiers land (phases 2–3), additionally run `--target
  linux-vm` / `--target windows-vm` and the full `--slice all` (real LLM
  providers) for that cut.
- `opverify-nightly.yml` runs the local `phase0` tier on a schedule so
  regressions surface between releases, not only at cut time.
- A new kernel route added without a catalog operation trips the coverage
  ratchet (report mode today; `enforce` once the catalog is complete) — extend
  the catalog, never widen the ignore-list to hide a gap.
- **MUST: every run is recorded.** Pass `--ledger`, so the run appends a row to
  `qa/opverify/history.jsonl` and is compared against the prior same-target
  baseline. That file is the only durable answer to "how much is this machinery
  actually used?" — a question that on 2026-07-27 could only be answered by
  cross-referencing CI run lists against hypervisor snapshot dates, because the
  ledger existed in code and had never been wired to anything. A verification
  tier nobody can count is a tier nobody can defend keeping.

### Pre-release verification (draft_only)

To produce installer artifacts on any branch without creating a tag or
publishing a GitHub Release (e.g. for hands-on Windows NSIS smoke that cannot
be reproduced on macOS / Linux), dispatch `release.yml` with `draft_only=true`:

```bash
gh workflow run release.yml \
  --ref <branch-name> \
  -f version=<x.y.z> \
  -f draft_only=true
```

The `Create tag` and `Create GitHub Release` steps are skipped; NSIS / `.dmg` /
`.deb` / `.AppImage` artifacts are uploaded to the workflow run and retrieved
with:

```bash
gh run list --workflow=release.yml --branch=<branch-name> --limit=1
gh run download <run-id>
```

The `version` input is still required by the workflow but the produced tag is
neither created nor published, so any throwaway placeholder is acceptable for
verification builds. Use real `x.y.z` matching `Cargo.toml` when iterating
toward an actual release so artifact filenames are coherent.

### User emulation (opverify visual apex)

**User emulation** is the top verification tier: drive the *real installed
GUI* through a *real user journey* with a visual agent — only the user is
automated (emulating real usage, not simulating it). opverify's confidence
pyramid, low to high: kernel emulation (headless daemon via the admin HTTP
API) < install + OS emulation (real installer on a real-OS VM) < **user
emulation (apex)** (real installed app, GUI driven as a user).

Each apex run cross-checks two oracles — a **visual** assert (a multimodal
agent reads a screenshot) against a **kernel** assert (the admin HTTP API);
agreement is the signal, disagreement localizes the fault (e.g. kernel OK but
nothing rendered = a frontend bug). This catches GUI-only defects a headless
probe never sees (e.g. a fatal startup modal), and lets a fix be *re-verified
on the real installed app*, closing capture → fix → re-verify. Pair it with a
`draft_only` build (above) to stage a fixed, unpublished installer on the VM.

Tooling + runbook: `scripts/opverify/visual/` (`FIRST_RUN.md` results log,
`VM_EXECUTOR_RUNBOOK.md` two-tier orchestrator ↔ VM-executor runbook + VM
access), landing with the opverify feature line.

**MUST — when an apex run is owed.** The pyramid described a top without saying
when to climb it, and the result was measurable: between 2026-07-14 and
2026-07-27 the apex ran exactly once, while the changes that most needed it (the
entire Lifecycle Defender install/uninstall line) shipped without a single frame
of the real GUI being looked at. A run is owed once a change lands on `master`
that touches what only a real installed GUI can exercise:

- the install / uninstall path (NSIS, `defender::purge*`,
  `POST /api/system/uninstall`);
- anything that renders *before or instead of* the main window (fatal startup
  modal, first-run setup, updater dialog);
- a destructive or irreversible action reachable from the GUI (Settings →
  Danger Zone).

It is owed **by the end of the next working session on this repo**, not inside
the landing PR: an apex run needs a `draft_only` build staged on the VM, which
is a human-triggered step, and blocking a PR on it would only teach everyone to
route around the rule. Two escapes, both explicit — fold the run into the
release-time VM verify when a cut is imminent, or write one line saying it was
skipped and why. A *silent* skip is the exact failure this rule removes.

Every apex run (`python -m scripts.opverify.visual.run_vm <journey>`) is
recorded to `qa/opverify/history.jsonl` by default — the ledger flag flipped
to default-on after a run shipped unrecorded because `--ledger` was forgotten
(2026-07-30). Skipping is `--no-ledger`, a deliberate visible choice. Apex
rows carry their own target label, so they are only ever compared against
prior apex rows.

The VM tiers of the *kernel* emulation (`--target linux-vm` / `windows-vm`) are
still unimplemented — `run.py` exits with "not yet implemented (phase 2/3)".
Until they land, the apex is the only tier that touches a real VM, which is
another reason the rule above is not optional.
