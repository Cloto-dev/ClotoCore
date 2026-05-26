; Tauri 2.x NSIS hook — bug-386 legacy install migration
;
; Detects a 0.6.5-era install (productName=cloto-system) and silently
; uninstalls it before the new ClotoCore install proceeds. User data
; at %APPDATA%\Roaming\cloto-system\ is preserved (config.rs:37 still
; reads from that literal path post-upgrade), so chat history, agent
; state, embedding namespaces, and registered MCP servers survive.
;
; No-op on fresh 0.6.7 installs (no legacy uninstall key present).

!macro NSIS_HOOK_PREINSTALL
  ; installMode: "both" can land per-machine (HKLM) or per-user (HKCU).
  ; Probe per-machine first.
  ReadRegStr $0 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\cloto-system" "UninstallString"
  StrCpy $1 "perMachine"
  ${If} $0 == ""
    ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\cloto-system" "UninstallString"
    StrCpy $1 "currentUser"
  ${EndIf}

  ${If} $0 != ""
    ; UninstallString may be quoted: strip surrounding quotes for ExecWait.
    StrCpy $2 $0 1
    ${If} $2 == '"'
      StrLen $3 $0
      IntOp $3 $3 - 2
      StrCpy $0 $0 $3 1
    ${EndIf}

    DetailPrint "bug-386: legacy cloto-system install detected ($1); running silent uninstall"
    ExecWait '"$0" /S' $4
    DetailPrint "bug-386: legacy uninstaller exit code $4 (data at %APPDATA%\Roaming\cloto-system\ preserved)"
  ${EndIf}
!macroend
