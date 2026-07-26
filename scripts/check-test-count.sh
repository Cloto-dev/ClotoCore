#!/usr/bin/env bash
# Test Count Ratchet — fails when a suite has fewer tests than the recorded floor.
#
# A ratchet is only as good as the floor it holds, and there are two ways it
# stops holding one. Both had happened by 2026-07-26:
#
#   * It does not run. The count was extracted with `grep -oP`, which is
#     GNU-only, so on macOS — the dogfooding platform — this script died on its
#     first command and never reached a comparison.
#   * The floor falls behind. The Rust baseline said 234 while the suite had
#     599, leaving room for 365 tests to disappear unnoticed. `--update` exists
#     so raising the floor is one command rather than a hand-edited JSON file.
#
# Usage:
#   check-test-count.sh [rust|dashboard|all]   # default: all
#   check-test-count.sh --update [target]      # raise the floor to what is measured
#
# A target that cannot be measured is an error, never a silent skip: the Rust
# job has no Node and the Dashboard job has no cargo, so each names the target
# it can actually run.

set -euo pipefail

cd "$(dirname "$0")/.."

BASELINE_FILE="qa/test-baseline.json"

# python3 on Linux CI, python on Windows.
PYTHON_CMD="python3"
if ! "$PYTHON_CMD" -c "pass" >/dev/null 2>&1; then
    PYTHON_CMD="python"
fi

UPDATE=false
TARGET="all"
for arg in "$@"; do
    case "$arg" in
        --update) UPDATE=true ;;
        rust | dashboard | all) TARGET="$arg" ;;
        *)
            echo "Unknown argument: $arg" >&2
            echo "Usage: $0 [--update] [rust|dashboard|all]" >&2
            exit 2
            ;;
    esac
done

# Sum every "<n> passed" in a test run's output.
#
# `grep -oE` rather than `-oP`: BSD grep (macOS) has no `-P`, and a ratchet that
# only runs on the CI platform is one the developer never watches fail.
# A run with no match still prints 0 (awk sees empty input), and `|| true`
# keeps `set -e` from killing the script before that 0 can be compared: a
# suite that suddenly reports nothing must break the floor loudly, not abort
# with grep's bare exit code.
sum_passed() {
    printf '%s\n' "$1" | grep -oE '[0-9]+ passed' | awk '{sum += $1} END {print sum+0}' || true
}

read_baseline() {
    "$PYTHON_CMD" -c "import json; print(json.load(open('$BASELINE_FILE'))['$1'])"
}

write_baseline() {
    "$PYTHON_CMD" - "$BASELINE_FILE" "$1" "$2" <<'PY'
import datetime, json, sys

path, key, value = sys.argv[1], sys.argv[2], int(sys.argv[3])
with open(path) as f:
    data = json.load(f)
data[key] = value
data["updated_at"] = datetime.date.today().isoformat()
data["updated_by"] = "check-test-count.sh --update"
with open(path, "w") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
PY
}

# Compare one suite against its floor. Returns non-zero when the floor is
# broken, so the caller can report every suite rather than stop at the first.
check_suite() {
    local label="$1" key="$2" count="$3"
    local baseline

    if [ -z "$count" ] || ! [ "$count" -ge 0 ] 2>/dev/null; then
        echo "❌ ${label}: no test count could be read from the run" >&2
        echo "   The output format changed, or the suite never ran." >&2
        return 1
    fi

    baseline=$(read_baseline "$key")
    echo "${label}: ${count} (floor: ${baseline})"

    if [ "$UPDATE" = true ]; then
        write_baseline "$key" "$count"
        echo "   floor raised to ${count}"
        return 0
    fi

    if [ "$count" -lt "$baseline" ]; then
        echo "❌ RATCHET FAILED: ${label} fell below the floor (${count} < ${baseline})" >&2
        echo "   If tests were removed on purpose: bash scripts/check-test-count.sh --update ${TARGET}" >&2
        return 1
    fi

    if [ "$count" -gt "$baseline" ]; then
        # Said loudly, with the exact command: this is the state a floor rots
        # in. Nobody objects to a passing run, and the gap widens quietly.
        echo "   📈 floor is $((count - baseline)) behind — raise it with:"
        echo "      bash scripts/check-test-count.sh --update ${TARGET}"
    fi
    return 0
}

FAILED=false

if [ "$TARGET" = "rust" ] || [ "$TARGET" = "all" ]; then
    if ! command -v cargo >/dev/null 2>&1; then
        echo "❌ Rust tests requested but cargo is not on PATH" >&2
        exit 2
    fi
    # `if ! VAR=$(...)` keeps the output instead of letting `set -e` abort with
    # it swallowed. A failing run must not be counted either: "50 passed" out of
    # a run that also failed two tests would otherwise clear the floor.
    # Colour off: the pattern reads digits out of human output, and an escape
    # sequence landing between the number and the word would silently zero the
    # count (the dashboard side hit exactly that under CI's forced colour).
    if ! RUST_OUTPUT=$(CARGO_TERM_COLOR=never cargo test --workspace --exclude app 2>&1); then
        printf '%s\n' "$RUST_OUTPUT"
        echo "❌ Rust tests failed — no count is certified from a failing run" >&2
        exit 1
    fi
    check_suite "Rust tests" rust_test_count "$(sum_passed "$RUST_OUTPUT")" || FAILED=true
fi

if [ "$TARGET" = "dashboard" ] || [ "$TARGET" = "all" ]; then
    if ! command -v npx >/dev/null 2>&1; then
        echo "❌ Dashboard tests requested but npx is not on PATH" >&2
        exit 2
    fi
    # `--run` because vitest otherwise watches. Deliberately no
    # `--passWithNoTests`: a suite that vanished has to read as zero and break
    # the floor, which is the one case this guard exists for.
    # The count comes from vitest's JSON reporter, not from its console output.
    # The human summary is styled, and under CI vitest forces colour on, so
    # `Tests 52 passed` arrives as `Tests \e[22m \e[1m\e[32m52 passed` and any
    # pattern written against a local (uncoloured, piped) run misses it — which
    # is exactly what happened on the first run of this step. `--run` because
    # vitest otherwise watches, and deliberately no `--passWithNoTests`: a suite
    # that vanished has to read as zero and break the floor.
    DASHBOARD_REPORT=$(mktemp)
    trap 'rm -f "$DASHBOARD_REPORT"' EXIT
    if ! DASHBOARD_OUTPUT=$(cd dashboard && npx vitest run --reporter=json --outputFile="$DASHBOARD_REPORT" 2>&1); then
        printf '%s\n' "$DASHBOARD_OUTPUT"
        echo "❌ Dashboard tests failed — no count is certified from a failing run" >&2
        exit 1
    fi
    DASHBOARD_COUNT=$("$PYTHON_CMD" -c "
import json, sys
try:
    print(json.load(open(sys.argv[1]))['numPassedTests'])
except Exception:
    pass
" "$DASHBOARD_REPORT")
    check_suite "Dashboard tests" dashboard_test_count "${DASHBOARD_COUNT:-}" || FAILED=true
fi

if [ "$FAILED" = true ]; then
    exit 1
fi

echo "✅ Test count ratchet passed"
