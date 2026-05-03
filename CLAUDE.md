# ClotoCore Development Rules

## Mandatory Reads

Read these before making changes. Do not summarize — read the actual files.

- **`docs/PROJECT_VISION.md`** — Core identity, competitive positioning, target users
- **`docs/ARCHITECTURE.md`** — System architecture, security framework, design principles
- **`docs/MGP_SPEC.md`** — MGP protocol (strict MCP superset). Servers: [cloto-mcp-servers](https://github.com/Cloto-dev/cloto-mcp-servers)
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

> Inherits: `../CLAUDE.md` — "Mandatory: Issue Registry Verification" section.
> MUST run `bash scripts/verify-issues.sh` when adding / fixing / claiming a fix for
> entries in `qa/issue-registry.json`. PostToolUse hook auto-runs on edits;
> `.githooks/pre-commit` blocks commits with `[STALE]` / `[UNFIXED]`.

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
