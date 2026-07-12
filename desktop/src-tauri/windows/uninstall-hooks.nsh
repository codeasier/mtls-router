!macro NSIS_HOOK_PREUNINSTALL
  ; The autostart plugin registers the Cargo package name for the current user.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "mtls-router-desktop"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "mtls-router-desktop"
!macroend
