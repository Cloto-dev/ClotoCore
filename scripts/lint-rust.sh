#!/usr/bin/env bash
# Local Rust lint gate: the CI Lint job's clippy invocation, plus --all-targets.
#
# The lint selection (the -A clippy::* allowlist) is NOT duplicated here — it is
# read out of .github/workflows/ci.yml, which CLAUDE.md declares the authoritative
# gate. A hand-copied allowlist drifts from CI silently and turns the local gate
# into noise; deriving it means the two cannot disagree.
#
# --all-targets is the one deliberate difference: CI lints the lib only, this also
# lints tests, benches and examples. That is the extra strictness the local gate
# buys you, and it is expected to be green.
#
# Fails closed: if the clippy step cannot be extracted from ci.yml in the shape we
# expect, this exits non-zero rather than silently linting with weaker flags.
set -euo pipefail

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

# Insert --all-targets ahead of the `--` separator that starts the lint flags.
cmd="${clippy_cmd/ -- -D warnings/ --all-targets -- -D warnings}"

echo "+ $cmd"
cd "$repo_root"
eval "$cmd"
