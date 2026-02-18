# Exiv Development Rules

## Project Vision (MANDATORY)

**You MUST read `docs/PROJECT_VISION.md` at the start of every session.**

This document defines the core identity, competitive positioning, target users,
and strategic direction of the Exiv Project. All development decisions — feature
additions, architectural changes, plugin development, UI work — must align with
the vision described in this document.

If a proposed change conflicts with the project vision, flag it to the user before proceeding.

## Architecture & Design Principles (MANDATORY)

**You MUST read `docs/ARCHITECTURE.md` before making any structural or code-level changes.**

This document defines the system architecture, security framework, plugin communication
protocols, and design principles of Exiv. Any code modification — new features, refactoring,
plugin development, API changes — must conform to the architectural constraints described here.

If a proposed change violates an architectural principle, flag it to the user before proceeding.

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
  "status": "open"
}
```

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

## Project Structure

- **Language**: Rust (workspace with multiple crates)
- **Core**: `crates/core/` - kernel, handlers, database, middleware
- **Plugins**: `plugins/` - python_bridge, etc.
- **Dashboard**: `dashboard/` - React/TypeScript web UI
- **Scripts**: `scripts/` - build, verification, and utility scripts
- **QA**: `qa/` - issue registry and quality assurance data

## GitHub Policy

- **NEVER link to binary/executable files directly from README.md or other documentation files.** This includes `.exe`, `.msi`, `.dmg`, `.AppImage`, installer scripts (`curl | bash`, `irm | iex`), and any other downloadable executables.
- Binary and executable files MUST be distributed exclusively through the [GitHub Releases](https://github.com/Exiv-ai/Exiv/releases) page.
- README.md may link to the Releases page itself (e.g., `[Releases](https://github.com/Exiv-ai/Exiv/releases/latest)`), but MUST NOT contain direct download URLs for binaries or piped install commands.

## Git Rules

- Commit messages in English
- Git author: `Exiv Project <exiv.project@proton.me>`
- Do NOT push without explicit user permission
