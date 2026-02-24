#!/usr/bin/env bash
# Sentinel — automated test quality guard
set -euo pipefail

ERRORS=0

# 1. Check for test file deletions in this commit/PR
# (git diff against base branch or previous commit)
DELETED_TEST_FILES=$(git diff --name-only --diff-filter=D HEAD~1 2>/dev/null | grep -E '_test\.(rs|ts|tsx)$|\.test\.(ts|tsx)$|tests/' || true)
if [ -n "$DELETED_TEST_FILES" ]; then
    echo "⚠️  SENTINEL: Test files deleted:"
    echo "$DELETED_TEST_FILES" | while read -r f; do echo "   - $f"; done
    ERRORS=$((ERRORS + 1))
fi

# 2. Check Rust tests for assertion-less test functions
# A test without assert!, assert_eq!, assert_ne!, #[should_panic], or .expect( is suspect
RUST_TEST_FILES=$(find crates/ -name '*_test.rs' -o -path '*/tests/*.rs' 2>/dev/null | grep -v common/ || true)
for f in $RUST_TEST_FILES; do
    # Extract test function blocks and check for assertions
    TESTS_WITHOUT_ASSERT=$(grep -c '#\[.*test\]' "$f" 2>/dev/null) || TESTS_WITHOUT_ASSERT=0
    ASSERTIONS=$(grep -cE 'assert!|assert_eq!|assert_ne!|should_panic|\.expect\(' "$f" 2>/dev/null) || ASSERTIONS=0
    if [ "$TESTS_WITHOUT_ASSERT" -gt 0 ] && [ "$ASSERTIONS" -eq 0 ]; then
        echo "⚠️  SENTINEL: $f has $TESTS_WITHOUT_ASSERT test(s) but no assertions"
        ERRORS=$((ERRORS + 1))
    fi
done

# 3. Check for large test deletions vs additions in diff
if git rev-parse HEAD~1 >/dev/null 2>&1; then
    TEST_ADDITIONS=$(git diff --numstat HEAD~1 -- '*.rs' '*.ts' '*.tsx' 2>/dev/null \
        | { grep -E '_test\.|\.test\.|/tests/' || true; } \
        | awk '{sum += $1} END {print sum+0}')
    TEST_DELETIONS=$(git diff --numstat HEAD~1 -- '*.rs' '*.ts' '*.tsx' 2>/dev/null \
        | { grep -E '_test\.|\.test\.|/tests/' || true; } \
        | awk '{sum += $2} END {print sum+0}')

    if [ "$TEST_DELETIONS" -gt 0 ] && [ "$TEST_DELETIONS" -gt "$((TEST_ADDITIONS * 2))" ]; then
        echo "⚠️  SENTINEL: Test deletions ($TEST_DELETIONS lines) far exceed additions ($TEST_ADDITIONS lines)"
        ERRORS=$((ERRORS + 1))
    fi
fi

# 4. Run issue registry verification
echo "--- Issue Registry Verification ---"
bash scripts/verify-issues.sh 2>&1 | tail -8 || true

if [ "$ERRORS" -gt 0 ]; then
    echo ""
    echo "❌ SENTINEL: $ERRORS warning(s) detected. Review required."
    # Warning only — don't block CI yet (can be promoted to exit 1 later)
    exit 0
fi

echo "✅ Sentinel passed — no quality regressions detected"
