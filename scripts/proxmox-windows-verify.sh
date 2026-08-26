#!/usr/bin/env bash
# proxmox-windows-verify.sh — Mac-side verify driver for a Proxmox Win11 guest.
#
# Rolls the guest back to a pristine snapshot, installs the FROM version from
# GitHub Releases, seeds a dummy database to detect data preservation, then
# installs the TO version (from a local file or GitHub Releases) and diffs
# the pre/post Windows fingerprint against the bug-386 NSIS hook assertion
# matrix. Replaces ~30-45 min Windows Sandbox manual verify with ~6 min
# automation. Which host and which guest are named entirely by the
# environment (see "Required environment" below); this script ships no
# defaults for them.
#
# Usage:
#   ./scripts/proxmox-windows-verify.sh \
#     --from <version> \
#     --to-version <version> \
#     [--to-asset <local-path>] \
#     [--out <report.json>]
#
# Examples:
#   # Verify the 0.6.5 → 0.6.7 upgrade path (hook PRIMARY, uses Releases for both):
#   ./scripts/proxmox-windows-verify.sh --from 0.6.5 --to-version 0.6.7
#
#   # Verify a locally-built installer (pre-merge smoke):
#   ./scripts/proxmox-windows-verify.sh \
#     --from 0.6.7 --to-version 0.6.8-alpha.1 \
#     --to-asset ./artifacts/ClotoCore_0.6.8-alpha.1_x64-setup.exe \
#     --out /tmp/verify-report.json
#
# Required environment (no defaults). These name machines that belong to
# whoever runs the script, so a value committed here would be wrong for every
# other reader. An unset one is reported by name, rather than reaching ssh and
# surfacing minutes later as a connect timeout:
#   PVE_HOST     Proxmox host address
#   PVE_USER     Proxmox SSH user
#   PVE_VMID     Win11 VM id on that host
#   VM_IP        Win11 guest address (keep it stable, e.g. a DHCP MAC binding)
#   VM_USER      Win11 local administrator account
#
# Optional environment:
#   PRISTINE_SNAPSHOT=pristine   PVE snapshot name to rollback to
#   GH_REPO=Cloto-dev/ClotoCore  GitHub repository for release asset lookup
#
# Prerequisites:
#   * Mac ssh-key wired to ${PVE_USER}@${PVE_HOST} and to Administrators on
#     the guest.
#   * gh CLI authenticated (gh auth login) for `gh release view`.
#   * jq installed.

set -euo pipefail

# ─── arg parse ─────────────────────────────────────────────────────────
FROM=""; TO=""; TO_ASSET=""; OUT=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --from)       FROM="$2"; shift 2 ;;
    --to-version) TO="$2"; shift 2 ;;
    --to-asset)   TO_ASSET="$2"; shift 2 ;;
    --out)        OUT="$2"; shift 2 ;;
    -h|--help)    sed -n '2,/^$/p' "$0"; exit 0 ;;
    *) echo "ERROR: unknown arg: $1" >&2; exit 2 ;;
  esac
done
[[ -n "$FROM" && -n "$TO" ]] || { echo "ERROR: --from and --to-version required" >&2; exit 2; }
[[ -z "$TO_ASSET" || -f "$TO_ASSET" ]] || { echo "ERROR: --to-asset not found: $TO_ASSET" >&2; exit 2; }

# ─── env ───────────────────────────────────────────────────────────────
: "${PVE_HOST:?not set -- the Proxmox host address (see --help)}"
: "${PVE_USER:?not set -- the Proxmox SSH user (see --help)}"
: "${PVE_VMID:?not set -- the Win11 VM id (see --help)}"
: "${VM_IP:?not set -- the Win11 guest address (see --help)}"
: "${VM_USER:?not set -- the Win11 administrator account (see --help)}"
PRISTINE_SNAPSHOT="${PRISTINE_SNAPSHOT:-pristine}"
GH_REPO="${GH_REPO:-Cloto-dev/ClotoCore}"

PVE="ssh ${PVE_USER}@${PVE_HOST}"
VM="ssh -o StrictHostKeyChecking=accept-new ${VM_USER}@${VM_IP}"

log() { printf '\033[36m[%s]\033[0m %s\n' "$(date +%H:%M:%S)" "$*"; }
fail() { printf '\033[31m[%s] FAIL: %s\033[0m\n' "$(date +%H:%M:%S)" "$*" >&2; exit 1; }

# Resolve a setup.exe asset name for a given version. Handles productName
# transition: ≤0.6.5 = cloto-system_<v>_x64-setup.exe, ≥0.6.6 = ClotoCore_<v>_x64-setup.exe.
resolve_asset() {
  local ver="$1"
  local name
  if command -v gh >/dev/null && name=$(gh release view "v$ver" --repo "$GH_REPO" \
        --json assets --jq '.assets[].name | select(test("setup\\.exe$"))' 2>/dev/null | head -1); then
    [[ -n "$name" ]] && { echo "$name"; return; }
  fi
  # Fallback table.
  case "$ver" in
    0.6.5|0.6.4|0.6.3|0.6.2|0.6.1|0.6.0) echo "cloto-system_${ver}_x64-setup.exe" ;;
    *)                                    echo "ClotoCore_${ver}_x64-setup.exe" ;;
  esac
}

# PowerShell heredoc → JSON fingerprint of the install state.
# Uses PowerShell single quotes + Join-Path to avoid nested double-quote stripping
# by the Windows OpenSSH default shell (cmd.exe) en route to powershell.exe.
FINGERPRINT_PS=$(cat <<'PS'
$dbPath = Join-Path $env:APPDATA 'cloto-system\cloto_memories.db'
@{
  dbHash = if (Test-Path $dbPath) { (Get-FileHash $dbPath -Algorithm SHA256).Hash } else { $null }
  legacyDir_cloto_system = Test-Path 'C:\Program Files\cloto-system'
  legacyDir_ClotoCore    = Test-Path 'C:\Program Files\ClotoCore'
  uninstallKey_HKLM_cloto_system = [bool](Get-ItemProperty 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\cloto-system' -ErrorAction SilentlyContinue)
  uninstallKey_HKLM_ClotoCore    = [bool](Get-ItemProperty 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ClotoCore' -ErrorAction SilentlyContinue)
  uninstallKey_HKCU_cloto_system = [bool](Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\cloto-system' -ErrorAction SilentlyContinue)
  uninstallKey_HKCU_ClotoCore    = [bool](Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ClotoCore' -ErrorAction SilentlyContinue)
  installedVersion = (Get-ItemProperty 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ClotoCore' DisplayVersion -ErrorAction SilentlyContinue).DisplayVersion
} | ConvertTo-Json -Compress
PS
)

# Send the PowerShell payload to the guest via -EncodedCommand to immunize it
# against cmd.exe nested-quote handling on the Windows OpenSSH side.
FINGERPRINT_PS_B64=$(printf '%s' "$FINGERPRINT_PS" | iconv -f UTF-8 -t UTF-16LE | base64 | tr -d '\n')

capture_fingerprint() {
  $VM "powershell -NoProfile -EncodedCommand $FINGERPRINT_PS_B64"
}

# ─── 1. rollback + cold boot ───────────────────────────────────────────
log "Rollback VM $PVE_VMID to snapshot '$PRISTINE_SNAPSHOT'"
$PVE "qm rollback $PVE_VMID $PRISTINE_SNAPSHOT && qm start $PVE_VMID"

log "Wait for guest SSH (max 180s)"
for i in $(seq 1 60); do
  if ssh -o ConnectTimeout=3 -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
       "${VM_USER}@${VM_IP}" 'whoami' >/dev/null 2>&1; then
    log "Guest SSH ready (attempt $i, ~$((i*3))s)"; break
  fi
  sleep 3
  [[ $i -eq 60 ]] && fail "Guest SSH did not come up within 180s"
done

# ─── 2. download FROM asset on guest ───────────────────────────────────
FROM_ASSET=$(resolve_asset "$FROM")
log "Download FROM asset: $FROM_ASSET"
$VM "powershell -NoProfile -Command \"Invoke-WebRequest 'https://github.com/${GH_REPO}/releases/download/v${FROM}/${FROM_ASSET}' -OutFile C:\\setup-from.exe -UseBasicParsing\""

# ─── 3. seed dummy DB (detect data preservation across upgrade) ────────
SEED="seed-$(date +%s)"
log "Seed dummy cloto_memories.db with marker '$SEED'"
$VM "powershell -NoProfile -Command \"New-Item -ItemType Directory -Force -Path \$env:APPDATA\\cloto-system | Out-Null; [System.IO.File]::WriteAllText((Join-Path \$env:APPDATA 'cloto-system\\cloto_memories.db'), '$SEED')\""

# ─── 4. install FROM + capture PRE fingerprint ─────────────────────────
log "Silent install FROM (v$FROM)"
$VM 'powershell -NoProfile -Command "Start-Process -FilePath C:\setup-from.exe -ArgumentList /S -Wait"'
sleep 5
PRE_JSON=$(capture_fingerprint)
log "PRE: $PRE_JSON"

# ─── 5. transport TO asset to guest ────────────────────────────────────
if [[ -n "$TO_ASSET" ]]; then
  log "scp TO asset: $TO_ASSET"
  scp "$TO_ASSET" "${VM_USER}@${VM_IP}:C:/setup-to.exe"
else
  TO_ASSET_NAME=$(resolve_asset "$TO")
  log "Download TO asset: $TO_ASSET_NAME"
  $VM "powershell -NoProfile -Command \"Invoke-WebRequest 'https://github.com/${GH_REPO}/releases/download/v${TO}/${TO_ASSET_NAME}' -OutFile C:\\setup-to.exe -UseBasicParsing\""
fi

# ─── 6. install TO + capture POST fingerprint ──────────────────────────
log "Silent install TO (v$TO)"
$VM 'powershell -NoProfile -Command "Start-Process -FilePath C:\setup-to.exe -ArgumentList /S -Wait"'
sleep 8
POST_JSON=$(capture_fingerprint)
log "POST: $POST_JSON"

# ─── 7. assertions ─────────────────────────────────────────────────────
declare -a CHECKS=()
declare -a RESULTS=()

assert_field() {
  local field="$1" pre_expect="$2" post_expect="$3"
  local pre post status
  pre=$(echo "$PRE_JSON" | jq -r ".${field}")
  post=$(echo "$POST_JSON" | jq -r ".${field}")
  if [[ "$pre" == "$pre_expect" && "$post" == "$post_expect" ]]; then
    status="PASS"
  else
    status="FAIL"
  fi
  CHECKS+=("$field: pre=$pre (want $pre_expect) post=$post (want $post_expect) → $status")
  RESULTS+=("$status")
}

assert_unchanged() {
  local field="$1" pre post status
  pre=$(echo "$PRE_JSON" | jq -r ".${field}")
  post=$(echo "$POST_JSON" | jq -r ".${field}")
  if [[ "$pre" == "$post" && "$pre" != "null" && -n "$pre" ]]; then
    status="PASS"
  else
    status="FAIL"
  fi
  CHECKS+=("$field: pre=$pre post=$post (must be unchanged + non-null) → $status")
  RESULTS+=("$status")
}

# Path selection: 0.6.5 → hook PRIMARY (legacy migration), else NO-OP (Tauri default upgrade flow).
if [[ "$FROM" == "0.6.5" ]]; then
  log "Assertion path: hook PRIMARY (0.6.5 → $TO)"
  assert_field legacyDir_cloto_system            "true"  "false"
  assert_field legacyDir_ClotoCore               "false" "true"
  assert_field uninstallKey_HKLM_cloto_system    "true"  "false"
  assert_field uninstallKey_HKLM_ClotoCore       "false" "true"
  assert_field uninstallKey_HKCU_cloto_system    "false" "false"
  assert_field uninstallKey_HKCU_ClotoCore       "false" "false"
  assert_field installedVersion                  "0.6.5" "$TO"
  assert_unchanged dbHash
else
  log "Assertion path: hook NO-OP (v$FROM → v$TO, Tauri default flow)"
  assert_field legacyDir_cloto_system            "false" "false"
  assert_field legacyDir_ClotoCore               "true"  "true"
  assert_field uninstallKey_HKLM_cloto_system    "false" "false"
  assert_field uninstallKey_HKLM_ClotoCore       "true"  "true"
  assert_field uninstallKey_HKCU_cloto_system    "false" "false"
  assert_field uninstallKey_HKCU_ClotoCore       "false" "false"
  assert_field installedVersion                  "$FROM" "$TO"
  assert_unchanged dbHash
fi

# ─── 8. report ─────────────────────────────────────────────────────────
total=${#RESULTS[@]}
pass=0
for r in "${RESULTS[@]}"; do [[ "$r" == "PASS" ]] && pass=$((pass + 1)); done

echo ""
echo "=== Assertion report (v$FROM → v$TO) ==="
for c in "${CHECKS[@]}"; do echo "  $c"; done
echo ""
if [[ $pass -eq $total ]]; then
  printf '\033[32mPASS: %d/%d\033[0m\n' "$pass" "$total"
else
  printf '\033[31mFAIL: %d/%d (%d failing)\033[0m\n' "$pass" "$total" "$((total - pass))"
fi

if [[ -n "$OUT" ]]; then
  jq -n \
    --arg from "$FROM" --arg to "$TO" \
    --argjson pre "$PRE_JSON" --argjson post "$POST_JSON" \
    --arg pass "$pass" --arg total "$total" \
    --argjson checks "$(printf '%s\n' "${CHECKS[@]}" | jq -R . | jq -s .)" \
    '{from: $from, to: $to, pre: $pre, post: $post, pass: ($pass | tonumber), total: ($total | tonumber), checks: $checks}' \
    > "$OUT"
  log "Report written to $OUT"
fi

[[ $pass -eq $total ]] || exit 1
exit 0
