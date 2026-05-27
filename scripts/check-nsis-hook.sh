#!/usr/bin/env bash
# check-nsis-hook.sh — Structural gate for installer.nsh
#
# Ensures the bug-386 NSIS_HOOK_PREINSTALL macro is intact at source level.
# Hook integrity is essential for 0.6.5-era in-place upgrades; silent
# removal (e.g. an accidental revert, a refactor that drops the file) must
# not be possible without CI failure.
#
# This is a source-level grep gate. It does not run makensis; a future
# alpha may add a build-level dump assertion as a second layer.
#
# Exit codes:
#   0 — all required patterns present
#   1 — at least one pattern missing (output includes ::error:: annotation)

set -euo pipefail

NSH="dashboard/src-tauri/installer.nsh"

if [[ ! -f "$NSH" ]]; then
  echo "::error file=$NSH::NSIS hook gate failed: $NSH does not exist"
  exit 1
fi

fail=0

require() {
  local pattern="$1" label="$2"
  if ! grep -qE "$pattern" "$NSH"; then
    echo "::error file=$NSH::NSIS hook gate failed: missing $label (pattern: $pattern)"
    fail=$((fail + 1))
  fi
}

# Macro entry point — Tauri NSIS bundler calls this during PREINSTALL.
require '^!macro NSIS_HOOK_PREINSTALL' "NSIS_HOOK_PREINSTALL macro"

# Legacy install detection (cloto-system productName, ≤0.6.5).
# Use . to dodge backslash escape and match both HKLM/HKCU probes.
require 'Uninstall.cloto-system' "legacy registry probe"

# Silent uninstall invocation against the legacy uninstaller.
require 'ExecWait.*\$0.*/S' "legacy silent uninstall"

# Audit log marker — required so post-install logs identify the migration path.
require 'DetailPrint.*bug-386:' "bug-386 audit log line"

if [[ $fail -gt 0 ]]; then
  echo ""
  echo "NSIS hook gate failed: $fail required pattern(s) missing in $NSH"
  echo "If the bug-386 hook was intentionally removed, update this script and add a registry entry."
  exit 1
fi

echo "OK: installer.nsh bug-386 hook intact ($NSH, 4/4 patterns matched)"
