#!/usr/bin/env bash
# Test Count Ratchet — blocks CI if test count decreases
set -euo pipefail

BASELINE_FILE="qa/test-baseline.json"

# Detect python command (python3 on Linux CI, python on Windows)
PYTHON_CMD="python3"
if ! "$PYTHON_CMD" -c "pass" &>/dev/null 2>&1; then
    PYTHON_CMD="python"
fi

# 1. Run cargo test and extract count
RUST_OUTPUT=$(cargo test --workspace --exclude app 2>&1)
RUST_COUNT=$(echo "$RUST_OUTPUT" | grep -oP '\d+ passed' | awk '{sum += $1} END {print sum}')

# 2. Read baseline
BASELINE_RUST=$($PYTHON_CMD -c "import json; print(json.load(open('$BASELINE_FILE'))['rust_test_count'])")

# 3. Compare
echo "Rust tests: ${RUST_COUNT} (baseline: ${BASELINE_RUST})"

if [ "$RUST_COUNT" -lt "$BASELINE_RUST" ]; then
    echo "❌ RATCHET FAILED: Rust test count decreased (${RUST_COUNT} < ${BASELINE_RUST})"
    echo "   If tests were intentionally removed, update qa/test-baseline.json"
    exit 1
fi

if [ "$RUST_COUNT" -gt "$BASELINE_RUST" ]; then
    echo "📈 Test count increased! Consider updating baseline: ${RUST_COUNT}"
fi

echo "✅ Test count ratchet passed"
