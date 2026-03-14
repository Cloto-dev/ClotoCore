# ClotoCore Development Rules

## Project Vision (MANDATORY)

**You MUST read `docs/PROJECT_VISION.md` at the start of every session.**

This document defines the core identity, competitive positioning, target users,
and strategic direction of the ClotoCore Project. All development decisions — feature
additions, architectural changes, plugin development, UI work — must align with
the vision described in this document.

If a proposed change conflicts with the project vision, flag it to the user before proceeding.

## Architecture & Design Principles (MANDATORY)

**You MUST read `docs/ARCHITECTURE.md` before making any structural or code-level changes.**

This document defines the system architecture, security framework, plugin communication
protocols, and design principles of ClotoCore. Any code modification — new features, refactoring,
plugin development, API changes — must conform to the architectural constraints described here.

If a proposed change violates an architectural principle, flag it to the user before proceeding.

## MGP (Model General Protocol) — MANDATORY

MGP (MCPの厳格なスーパーセット) について理解して、MCPではなくMGPを積極的に実装すること。
MGP仕様は `docs/MGP_SPEC.md` を参照。詳細ドキュメント (security, communication, discovery,
guide, isolation design) およびサーバー実装は
[cloto-mcp-servers](https://github.com/Cloto-dev/cloto-mcp-servers) リポジトリに移管済み。

## Development Guardrails (MANDATORY)

**You MUST read `docs/DEVELOPMENT.md` before making any code changes.**

This document defines 8 critical guardrails (security, cascading, state management,
storage, UI/UX, physical safety, external processes, privacy/biometrics) that constrain
what code changes are safe to make. Violating a guardrail can cause security vulnerabilities,
infinite loops, data corruption, or physical safety issues.

If a proposed change touches a guardrail-protected area, flag it to the user before proceeding.

## Bug Verification Workflow (MANDATORY)

All bug investigation and fixing work MUST follow this verification workflow.
This applies to ALL severity levels (CRITICAL, HIGH, MEDIUM, LOW).

### Source of Truth

**`qa/issue-registry.json`** is the version-controlled registry for all documented issues.
This file is the single source of truth for bug verification. `.dev-notes/*.md` files are
supplementary human-readable notes (gitignored, not authoritative).

### Discovery Phase

When a bug is found, BEFORE attempting any fix:

1. **Add an entry to `qa/issue-registry.json`** with all required fields
2. **Run `bash scripts/verify-issues.sh`** to confirm `[VERIFIED]` (proves the bug exists)
3. Optionally add human-readable notes to `.dev-notes/*.md`

If the verification script does not return `[VERIFIED]` for the new entry,
the bug documentation is invalid and must be corrected before proceeding.

### Registry Entry Format

Each entry in `qa/issue-registry.json` → `issues[]`:

```json
{
  "id": "bug-NNN",
  "summary": "Short description of the bug",
  "severity": "CRITICAL|HIGH|MEDIUM|LOW",
  "discovered": "ISO-8601-timestamp",
  "version": "cargo-toml-version",
  "commit": "short-git-hash",
  "file": "path/relative/to/project/root",
  "pattern": "grep-P-compatible-regex",
  "expected": "present",
  "status": "open",
  "github_issue": 123
}
```

- `github_issue` is **optional** — do NOT set it manually. It is auto-populated by the
  GitHub Actions workflow (`issue-sync.yml`) when the PR adding the entry is merged.

### GitHub Issue Sync

The `issue-sync` workflow (`scripts/sync-issues-to-github.sh`) runs on PR merge and performs:

1. **Auto-create**: New registry entries without `github_issue` → creates a GitHub Issue
   with `bug`, severity label, and `auto-created` label, then commits the issue number back.
2. **Auto-close**: Entries that changed to `status: "fixed"` with a `github_issue` →
   closes the linked GitHub Issue with a resolution comment.

### Fix Phase

After fixing the bug:

1. Update the registry entry: `"expected": "present"` -> `"expected": "absent"`
2. Update the registry entry: `"status": "open"` -> `"status": "fixed"`
3. Run `bash scripts/verify-issues.sh` to confirm `[FIXED]` status
4. Commit both the code fix AND the updated `qa/issue-registry.json`

### Key Rules

- **No fix without verification**: Every bug must have a `[VERIFIED]` entry before work begins
- **No commit without re-verification**: Run `bash scripts/verify-issues.sh` before committing fixes
- **Anti-hallucination**: The pattern-based verification proves bugs exist in the actual codebase
- **Traceability**: Each entry links to the exact version, commit, and file where the bug was found
- **Registry is sacred**: Do NOT remove or modify existing entries without running verification. When in doubt, run `bash scripts/verify-issues.sh` to check integrity

### Verification Script Protection

**`scripts/verify-issues.sh` is a critical infrastructure component. NEVER modify it without explicit user approval.**

This script is the mechanical verification engine that prevents hallucination and ensures bug tracking integrity.

**Protected Status:**
- **Read-only by default**: Treat as infrastructure, not application code
- **Modification requires approval**: If you identify a bug or improvement, report it to the user FIRST
- **No refactoring without discussion**: Even "improvements" can break verification integrity
- **Test changes thoroughly**: If approved to modify, run full verification before committing

**When you discover an issue with the script:**
1. Do NOT fix it immediately
2. Report the issue to the user with:
   - What you found (bug description)
   - Why it's problematic (impact analysis)
   - Proposed fix (code diff)
3. Wait for user approval before making changes
4. After approval, modify and verify with: `bash scripts/verify-issues.sh`

**Rationale:** This script is the foundation of the anti-hallucination system. Unintended changes could:
- Break bug verification
- Invalidate historical tracking data
- Introduce false positives/negatives
- Compromise audit trail integrity

## Dashboard UI Rules

### Text Readability Minimums

- **Minimum text size**: `text-[9px]` (9px). Never use `text-[8px]` or smaller.
- **Minimum text color**: `text-content-tertiary`. Never use `text-content-muted` for readable text.
  - `text-content-muted` is permitted only for decorative elements (borders, disabled icons, dividers).
  - All user-readable text must use `text-content-tertiary` or higher (`text-content-secondary`, `text-content-primary`).

### Hover Border Patterns

- **Interactive card/row hover**: `hover:border-brand` — brand color outline on hover.
- **Destructive/deactivate hover**: `hover:border-red-500` — red outline for remove/revoke actions.
- Always use full opacity (not `/30` or `/50`) for hover borders to ensure visibility.

## Project Structure

- **Language**: Rust (workspace with multiple crates)
- **Core**: `crates/core/` - kernel, handlers, database, middleware
- **MCP Servers**: [cloto-mcp-servers](https://github.com/Cloto-dev/cloto-mcp-servers) (separate repo)
- **Dashboard**: `dashboard/` - React/TypeScript web UI
- **Scripts**: `scripts/` - build, verification, and utility scripts
- **QA**: `qa/` - issue registry and quality assurance data

## GitHub Policy

- **NEVER link to binary/executable files directly from README.md or other documentation files.** This includes `.exe`, `.msi`, `.dmg`, `.AppImage`, installer scripts (`curl | bash`, `irm | iex`), and any other downloadable executables.
- Binary and executable files MUST be distributed exclusively through the [GitHub Releases](https://github.com/Cloto-dev/ClotoCore/releases) page.
- README.md may link to the Releases page itself (e.g., `[Releases](https://github.com/Cloto-dev/ClotoCore/releases/latest)`), but MUST NOT contain direct download URLs for binaries or piped install commands.

## Git Rules

- Commit messages in English
- Git author: `ClotoCore Project <ClotoCore@proton.me>`
- Do NOT push without explicit user permission

## Release Rules

### Version Bump

- Bump version in `Cargo.toml`, `dashboard/package.json`, `dashboard/src-tauri/tauri.conf.json` on every version-incrementing commit (even if that version will not be released).
- Unstable versions may be skipped for release. The version number is "consumed" and not reused.

### Tags

- Do NOT create git tags manually (`git tag` is prohibited).
- Tags are created exclusively by `gh release create`, which auto-creates the tag on the release commit.

### Release Notes (Cumulative Changelog)

- Release notes MUST include all changes since the **previous released version** (not just the current version's commit).
- Use `gh release list` to identify the previous release, then gather all commits between that tag and HEAD.
- When intermediate versions were skipped, organize the changelog by **version sections** (e.g., `### v0.5.5`, `### v0.5.4`, `### v0.5.3 → v0.5.4`).
- Each section should describe features, bug fixes, and other changes for that version.
