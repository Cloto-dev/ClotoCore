#!/usr/bin/env bash
# Local Rust lint gate: the CI Lint job's clippy invocation, verbatim.
#
# The lint selection (the -A clippy::* allowlist, --all-targets) is NOT duplicated
# here — it is read out of .github/workflows/ci.yml, which CLAUDE.md declares the
# authoritative gate. A hand-copied allowlist drifts from CI silently and turns the
# local gate into noise; deriving it means the two cannot disagree.
#
# --all-targets is appended only if CI does not already pass it, so this stays at
# least as strict as CI even if the flag is ever dropped there.
#
# Fails closed: if the clippy step cannot be extracted from ci.yml in the shape we
# expect, this exits non-zero rather than silently linting with weaker flags.
#
# Usage:
#   bash scripts/lint-rust.sh            run the derived command
#   bash scripts/lint-rust.sh --print    print the derived command and exit
#                                        (the CI "lint-rust.sh self-check" step)
set -euo pipefail

print_only=false
case "${1:-}" in
  --print) print_only=true ;;
  "") ;;
  *)
    echo "lint-rust: unknown argument: $1 (expected --print or nothing)" >&2
    exit 2
    ;;
esac

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ci_yml="$repo_root/.github/workflows/ci.yml"

if [[ ! -f "$ci_yml" ]]; then
  echo "lint-rust: $ci_yml not found" >&2
  exit 1
fi

# The single-line `run:` that follows `- name: Run clippy`.
clippy_cmd="$(awk '
  /^[[:space:]]*-[[:space:]]*name:[[:space:]]*Run clippy[[:space:]]*$/ { seen = 1; next }
  seen && /^[[:space:]]*run:[[:space:]]*/ {
    sub(/^[[:space:]]*run:[[:space:]]*/, "")
    print
    exit
  }
  seen && /^[[:space:]]*-[[:space:]]*name:/ { exit }
' "$ci_yml")"

if [[ "$clippy_cmd" != cargo\ clippy\ * || "$clippy_cmd" != *" -- -D warnings"* ]]; then
  echo "lint-rust: could not extract the clippy step from ci.yml in the expected shape." >&2
  echo "lint-rust: got: ${clippy_cmd:-<empty>}" >&2
  echo "lint-rust: update this script alongside the CI Lint job." >&2
  exit 1
fi

# Ahead of the `--` separator that starts the lint flags, and only once.
if [[ "$clippy_cmd" == *" --all-targets "* ]]; then
  cmd="$clippy_cmd"
else
  cmd="${clippy_cmd/ -- -D warnings/ --all-targets -- -D warnings}"
fi

if [[ "$print_only" == true ]]; then
  echo "$cmd"
  exit 0
fi

echo "+ $cmd"
cd "$repo_root"
eval "$cmd"
