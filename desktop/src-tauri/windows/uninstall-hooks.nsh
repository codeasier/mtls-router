!macro NSIS_HOOK_PREUNINSTALL
  ; The autostart plugin registers the Cargo package name for the current user.
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "mtls-router-desktop"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "mtls-router-desktop"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Tauri CLI 2.11.4 owns this checkbox; only explicit interactive consent
  ; removes the business data directory, whose name differs from BUNDLEID.
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    RMDir /r "$APPDATA\com.codeasier.mtls-router"
  ${EndIf}
!macroend
