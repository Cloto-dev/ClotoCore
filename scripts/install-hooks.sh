#!/usr/bin/env bash
# install-hooks.sh - Activate .githooks/ for this clone.
#
# Git hooks under .git/hooks/ are per-clone and not version-controlled.
# This script points git at the version-controlled .githooks/ directory so
# every contributor gets the same pre-commit / pre-push etc.
#
# Usage: bash scripts/install-hooks.sh
# Idempotent: re-running only re-confirms the setting.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null)"
if [[ -z "$REPO_ROOT" ]]; then
  echo "[install-hooks] Not inside a git repository." >&2
  exit 1
fi
cd "$REPO_ROOT"

if [[ ! -d .githooks ]]; then
  echo "[install-hooks] .githooks/ directory missing — nothing to install." >&2
  exit 1
fi

git config core.hooksPath .githooks
echo "[install-hooks] core.hooksPath set to .githooks"
echo "[install-hooks] Installed hooks:"
for h in .githooks/*; do
  [[ -f "$h" ]] || continue
  [[ -x "$h" ]] || chmod +x "$h"
  echo "  - $(basename "$h")"
done

# Baseline check: warn the contributor if the registry already has stale/unfixed
# entries. The pre-commit hook will block commits that touch qa/issue-registry.json
# until those are resolved (or bypass with --no-verify).
if [[ -f scripts/verify-issues.sh && -f qa/issue-registry.json ]]; then
  echo ""
  echo "[install-hooks] Running baseline check..."
  if bash scripts/verify-issues.sh >/dev/null 2>&1; then
    echo "[install-hooks] Baseline is clean — pre-commit will allow registry-touching commits."
  else
    echo ""
    echo "[install-hooks] WARNING: qa/issue-registry.json baseline has [STALE] / [UNFIXED] / [ERROR] entries."
    echo "[install-hooks] Until those are resolved, any commit that touches the registry will be blocked by pre-commit."
    echo "[install-hooks] Review with: bash scripts/verify-issues.sh"
    echo "[install-hooks] To override a single commit intentionally: git commit --no-verify (discouraged)."
  fi
fi

echo ""
echo "[install-hooks] Done. To disable: git config --unset core.hooksPath"
