!define MUI_FINISHPAGE_RUN_TEXT "Launch Snapline"

!macro NSIS_HOOK_PREINSTALL
  StrCpy $INSTDIR "$LOCALAPPDATA\Programs\${PRODUCTNAME}"
  RMDir "$LOCALAPPDATA\${PRODUCTNAME}"
  CreateDirectory "$INSTDIR"
  SetOutPath "$INSTDIR"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    !insertmacro CheckIfAppIsRunning "$INSTDIR\snapline.exe" "${PRODUCTNAME}"
    ${If} ${FileExists} "$INSTDIR\snapline.exe"
      SetOutPath "$INSTDIR"
      ExecWait '"$INSTDIR\snapline.exe" --uninstall-cleanup'
      SetOutPath "$TEMP"
    ${EndIf}
  ${EndIf}
!macroend
