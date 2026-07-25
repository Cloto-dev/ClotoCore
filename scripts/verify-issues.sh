#!/usr/bin/env bash
# verify-issues.sh - Mechanically verify documented issues against codebase
#
# Reads qa/issue-registry.json (version-controlled source of truth) and
# checks if each documented pattern exists in the specified file.
#
# Usage: bash scripts/verify-issues.sh [--filter STATUS]
#   No arguments: verify all issues
#   --filter open:   verify only open issues
#   --filter fixed:  verify only fixed issues
#
# Exit codes:
#   0 - All issues verified successfully
#   1 - One or more issues failed verification
#
# Requires: python3 (for JSON parsing)

set -euo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Resolve project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

REGISTRY="$PROJECT_ROOT/qa/issue-registry.json"

# Parse arguments
FILTER=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --filter) FILTER="$2"; shift 2 ;;
        *) echo -e "${RED}[ERROR]${NC} Unknown argument: $1"; exit 1 ;;
    esac
done

# Check prerequisites
if [[ ! -f "$REGISTRY" ]]; then
    echo -e "${RED}[ERROR]${NC} Registry not found: $REGISTRY"
    exit 1
fi

PYTHON_CMD="python3"
if ! "$PYTHON_CMD" --version &>/dev/null 2>&1; then
    PYTHON_CMD="python"
fi
if ! command -v "$PYTHON_CMD" &>/dev/null; then
    echo -e "${RED}[ERROR]${NC} python3 or python is required but not found"
    exit 1
fi

echo -e "${CYAN}=== Issue Verification Report ===${NC}"
echo -e "Registry: qa/issue-registry.json"
echo -e "Date:     $(date -u +%Y-%m-%dT%H:%M:%SZ)"
[[ -n "$FILTER" ]] && echo -e "Filter:   $FILTER"
echo ""

# Counters
total=0
verified=0
stale=0
fixed=0
errors=0
rows_read=0
declared_count=""

# Extract issues from JSON using python3, into a file rather than a process
# substitution (bug-494). A substitution's exit status is invisible to
# `set -e`, so a registry that failed to parse yielded zero rows and the run
# still reported success — silently disabling verification for EVERY entry
# while the pre-commit hook and the CI Issue Registry job (both of which gate
# only on this script's exit code) went green.
EXTRACT_OUT="$(mktemp)"
EXTRACT_ERR="$(mktemp)"
trap 'rm -f "$EXTRACT_OUT" "$EXTRACT_ERR"' EXIT

# Convert path for native Python on Windows (MSYS /c/ → C:/)
_REGISTRY_PY="$REGISTRY"
if command -v cygpath &>/dev/null; then
    _REGISTRY_PY="$(cygpath -m "$REGISTRY")"
fi

# Output format: id|severity|file|pattern|expected|status|summary
# plus a final `#count=<n>` trailer, so a truncated read cannot masquerade as
# a short registry. `data['issues']` is indexed, not `.get`, so a registry
# missing the key raises instead of verifying nothing.
if ! PYTHONUTF8=1 $PYTHON_CMD -c "
import json, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
with open('$_REGISTRY_PY', encoding='utf-8') as f:
    data = json.load(f)
issues = data['issues']
for issue in issues:
    print('|'.join([
        issue.get('id', ''),
        issue.get('severity', '?'),
        issue.get('file', ''),
        issue.get('pattern', ''),
        issue.get('expected', 'present'),
        issue.get('status', 'unknown'),
        issue.get('summary', ''),
    ]))
print('#count={}'.format(len(issues)))
" > "$EXTRACT_OUT" 2> "$EXTRACT_ERR"; then
    echo -e "  ${RED}[ERROR]${NC} Failed to parse registry: qa/issue-registry.json"
    sed 's/^/           /' "$EXTRACT_ERR"
    echo ""
    echo -e "${RED}Registry could not be read — nothing was verified.${NC}"
    exit 1
fi

while IFS='|' read -r id severity file pattern expected status summary; do
    # Row-count trailer, not an issue
    if [[ "$id" == '#count='* ]]; then
        declared_count="${id#\#count=}"
        continue
    fi
    rows_read=$((rows_read + 1))

    # Apply filter
    if [[ -n "$FILTER" && "$status" != "$FILTER" ]]; then
        continue
    fi

    # Skip obsolete entries (referenced files deleted during migration)
    if [[ "$status" == "obsolete" ]]; then
        continue
    fi

    total=$((total + 1))
    full_path="$PROJECT_ROOT/$file"

    # Check file exists
    if [[ ! -f "$full_path" ]]; then
        if [[ "$expected" == "absent" ]]; then
            echo -e "  ${GREEN}[FIXED]${NC} $id ($severity): $summary"
            echo -e "           File deleted: $file (pattern trivially absent)"
            fixed=$((fixed + 1))
        else
            echo -e "  ${RED}[ERROR]${NC} $id ($severity): File not found: $file"
            errors=$((errors + 1))
        fi
        continue
    fi

    # Count grep matches (prefer -P for Perl regex, fall back to -E for environments
    # where grep -P is unavailable, e.g. GNU grep 3.0 on Git for Windows/MSYS2)
    # Note: grep -c returns exit code 1 when count is 0, so we handle it explicitly
    match_count=$(grep -cP "$pattern" "$full_path" 2>/dev/null) || \
    match_count=$(grep -cE "$pattern" "$full_path" 2>/dev/null) || \
    match_count=0

    if [[ "$expected" == "present" ]]; then
        if [[ "$match_count" -gt 0 ]]; then
            echo -e "  ${GREEN}[VERIFIED]${NC} $id ($severity): $summary"
            echo -e "           Pattern found in $file (${match_count} matches)"
            verified=$((verified + 1))
        else
            echo -e "  ${YELLOW}[STALE]${NC} $id ($severity): $summary"
            echo -e "           Pattern NOT found in $file (may be fixed or moved)"
            stale=$((stale + 1))
        fi
    elif [[ "$expected" == "absent" ]]; then
        if [[ "$match_count" -eq 0 ]]; then
            echo -e "  ${GREEN}[FIXED]${NC} $id ($severity): $summary"
            echo -e "           Pattern no longer present in $file"
            fixed=$((fixed + 1))
        else
            echo -e "  ${RED}[UNFIXED]${NC} $id ($severity): $summary"
            echo -e "           Pattern still present in $file (${match_count} matches)"
            errors=$((errors + 1))
        fi
    fi

done < "$EXTRACT_OUT"

# The extractor parsed the registry, so any mismatch here means rows were lost
# between it and this loop — report it rather than verifying a subset quietly.
if [[ -z "$declared_count" ]]; then
    echo -e "  ${RED}[ERROR]${NC} Registry extraction ended without a row-count trailer — output was truncated"
    errors=$((errors + 1))
elif [[ "$rows_read" -ne "$declared_count" ]]; then
    echo -e "  ${RED}[ERROR]${NC} Read $rows_read of $declared_count registry entries — output was truncated"
    errors=$((errors + 1))
fi

# Summary
echo ""
echo -e "${CYAN}=== Summary ===${NC}"
echo -e "Total issues:  $total"
echo -e "Verified:      ${GREEN}$verified${NC}"
echo -e "Stale:         ${YELLOW}$stale${NC}"
echo -e "Fixed:         ${GREEN}$fixed${NC}"
echo -e "Errors:        ${RED}$errors${NC}"

# Checked before the empty-registry case on purpose: an extraction problem
# reports zero verified issues, and treating that as "nothing to do" is what
# made this gate fail open (bug-494).
if [[ $stale -gt 0 || $errors -gt 0 ]]; then
    echo ""
    echo -e "${RED}WARNING: $((stale + errors)) issue(s) need attention${NC}"
    exit 1
fi

if [[ $total -eq 0 ]]; then
    echo ""
    echo -e "${YELLOW}No issues found in registry.${NC}"
    exit 0
fi

echo ""
echo -e "${GREEN}All issues verified successfully.${NC}"
exit 0
