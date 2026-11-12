; Tableboat installer hooks: put the install folder on the user's PATH so `tb` works in any
; new terminal, and take it off again on uninstall. Per-user (HKCU), no admin rights needed.
;
; The registry value is read without expanding %VARIABLES% and written back as REG_EXPAND_SZ,
; so other entries keep their form; running it twice never adds a second entry. The folder is
; passed in an environment variable (TB_DIR), not pasted into the command, so any path works.

!macro TB_UPDATE_PATH ACTION
  System::Call 'Kernel32::SetEnvironmentVariable(t "TB_DIR", t "$INSTDIR")'
  nsExec::Exec `powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$$k = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment'); $$p = [string]$$k.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames); $$d = $$env:TB_DIR.TrimEnd('\'); $$parts = @($$p -split ';' | Where-Object { $$_ -and ($$_.TrimEnd('\') -ne $$d) }); if ('${ACTION}' -eq 'add') { $$parts += $$d }; $$k.SetValue('Path', ($$parts -join ';'), [Microsoft.Win32.RegistryValueKind]::ExpandString)"`
  Pop $0
  ; Tell running programs (Explorer, new terminals) that the environment changed.
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro TB_UPDATE_PATH add
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  !insertmacro TB_UPDATE_PATH remove
!macroend
