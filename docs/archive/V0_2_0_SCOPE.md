# v0.2.0 (β2) Release Scope

> **Theme:** Bug fixes, security hardening, performance improvements, documentation, and refinements
> **Target:** Demo video release

---

## 1. Bug Fixes

- Audit `qa/issue-registry.json` for open/unverified issues
- Run `bash scripts/verify-issues.sh` to confirm current bug status
- Fix remaining open bugs, prioritizing CRITICAL > HIGH > MEDIUM
- Verify all fixes with the issue verification workflow

## 2. Security Hardening

- Dependency audit (`cargo audit`, review RUSTSEC advisories)
- CORS configuration review
- Input validation coverage check
- Environment variable handling review (secrets, defaults)

## 3. Performance Improvements

- Identify build time bottlenecks
- Review database query patterns (unnecessary full-table scans, missing indices)
- Frontend bundle size audit (unused dependencies, lazy loading opportunities)
- Startup time profiling

## 4. Documentation

- Rewrite CHANGELOG with version-based format (done: v0.1.0 section)
- Verify docs/ accuracy against current codebase
- Remove stale references to deleted components (Evolution Engine, Rust plugins, etc.)
- Ensure CLAUDE.md, ARCHITECTURE.md, DEVELOPMENT.md are consistent

## 5. Refinements (Core Priority & Code Reduction)

- Remove dead code and unused files
- Clean up `archive/` directory (confirm completeness)
- Eliminate redundant dependencies in Cargo.toml
- Review `.gitignore` coverage
- Remove stale `.dev-notes/` content if applicable

---

## Release Checklist

- [ ] All open bugs in issue-registry.json resolved or documented
- [ ] `cargo audit` clean (or advisories acknowledged)
- [ ] `cargo clippy` clean
- [ ] `cargo test` all passing
- [ ] Dashboard builds without warnings
- [ ] CHANGELOG v0.2.0 section finalized
- [ ] Version bumped in `Cargo.toml` (root) and `dashboard/package.json`
- [ ] Tag `v0.2.0` created
