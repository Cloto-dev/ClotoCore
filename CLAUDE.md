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
- Bug verify: `bash scripts/verify-issues.sh`
- Test ratchet: `bash scripts/check-test-count.sh`

## Bug Verification

- Source of truth: `qa/issue-registry.json`
- Discovery: add entry → `bash scripts/verify-issues.sh` → must return `[VERIFIED]`
- Fix: update `expected`→`"absent"`, `status`→`"fixed"` → re-verify → must return `[FIXED]`
- `scripts/verify-issues.sh` is **read-only infrastructure** — never modify without user approval

## Agent Config Rules

- All agent config operations MUST be deferred (pending state → apply on Save)
- Direct mutation API calls (upload, delete, update) are PROHIBITED outside `handleSave`
- Cancel/Back MUST discard all pending changes without API calls
- Pattern: event handler → set pending state only, `handleSave` → execute all pending
- Reference implementation: `AgentPluginWorkspace.tsx`

## Dashboard UI Rules

- **Min text size**: `text-[9px]`. Never `text-[8px]` or smaller.
- **Min text color**: `text-content-tertiary`. Never `text-content-muted` for readable text.
- **Hover borders**: `hover:border-brand` (interactive), `hover:border-red-500` (destructive). Full opacity.

## Git Rules

- Commit messages in English
- Git author: `ClotoCore Project <ClotoCore@proton.me>`
- Do NOT push without explicit user permission
- Do NOT create git tags manually — use `gh release create`
- Binaries distributed exclusively via [GitHub Releases](https://github.com/Cloto-dev/ClotoCore/releases)

## Release Rules

- Bump version in `Cargo.toml`, `dashboard/package.json`, `dashboard/src-tauri/tauri.conf.json`
- Release notes: cumulative from previous release (`gh release list` to find it)
