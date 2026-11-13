Unicode true
ManifestDPIAware true
RequestExecutionLevel user
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "LogicLib.nsh"

!ifndef ROOT
  !error "pass -DROOT=<repository root>"
!endif
!ifndef OUTFILE
  !error "pass -DOUTFILE=<installer path>"
!endif
!ifndef VERSION
  !error "pass -DVERSION=<x.y.z>"
!endif

!define PRODUCT "Snapline"
!define EXE "snapline.exe"
!define APP_ID "io.github.aaronburt.snapline"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT}"
!define WINDOW_CLASS "SnaplineMainMessageWindow"

Name "${PRODUCT}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${PRODUCT}"
BrandingText "${PRODUCT} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${PRODUCT}"
VIAddVersionKey "FileDescription" "${PRODUCT} installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "GPL-3.0-or-later"

!define MUI_ICON "${ROOT}\apps\snapline\icons\icon.ico"
!define MUI_UNICON "${ROOT}\apps\snapline\icons\icon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Launch Snapline"

!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!macro DefineCloseApp un
Function ${un}CloseApp
  StrCpy $R1 0
  ${Do}
    nsExec::Exec 'cmd /c tasklist /FI "IMAGENAME eq ${EXE}" /NH | find /I "${EXE}"'
    Pop $R0
    ${If} $R0 != 0
      ${Break}
    ${EndIf}
    System::Call 'user32::FindWindowExW(p -3, p 0, w "${WINDOW_CLASS}", p 0) p .r0'
    ${If} $0 <> 0
      System::Call 'user32::PostMessageW(p r0, i 0x10, p 0, p 0)'
    ${EndIf}
    Sleep 500
    IntOp $R1 $R1 + 1
    ${If} $R1 >= 20
      ${If} ${Silent}
        Abort "${PRODUCT} is still running. Exit it from the tray and try again."
      ${EndIf}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${PRODUCT} is still running. Exit it from the tray icon, then choose Retry." IDRETRY +2
      Abort
      StrCpy $R1 0
    ${EndIf}
  ${Loop}
FunctionEnd
!macroend
!insertmacro DefineCloseApp ""
!insertmacro DefineCloseApp "un."



Section "Install"
  Call CloseApp
  RMDir "$LOCALAPPDATA\${PRODUCT}"
  CreateDirectory "$INSTDIR"
  SetOutPath "$INSTDIR"
  File "${ROOT}\target\release\${EXE}"
  File "${ROOT}\third_party\ffmpeg\ffmpeg.exe"
  File "${ROOT}\LICENSE"
  File "/oname=FFmpeg-LICENSE.txt" "${ROOT}\third_party\ffmpeg\LICENSE"
  File "${ROOT}\third_party\ffmpeg\SOURCE.txt"
  SetOutPath "$INSTDIR\assets\sounds"
  File "${ROOT}\assets\sounds\shutter.wav"
  File "${ROOT}\assets\sounds\done.wav"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\${PRODUCT}.lnk" "$INSTDIR\${EXE}"
  ExecWait '"$INSTDIR\${EXE}" --set-shortcut-app-id "$SMPROGRAMS\${PRODUCT}.lnk"'
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${PRODUCT}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${PRODUCT}"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Call un.CloseApp
  ${If} ${FileExists} "$INSTDIR\${EXE}"
    SetOutPath "$INSTDIR"
    ExecWait '"$INSTDIR\${EXE}" --uninstall-cleanup'
    SetOutPath "$TEMP"
  ${EndIf}
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\ffmpeg.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\FFmpeg-LICENSE.txt"
  Delete "$INSTDIR\SOURCE.txt"
  Delete "$INSTDIR\assets\sounds\shutter.wav"
  Delete "$INSTDIR\assets\sounds\done.wav"
  RMDir "$INSTDIR\assets\sounds"
  RMDir "$INSTDIR\assets"
  Delete "$SMPROGRAMS\${PRODUCT}.lnk"
  DeleteRegKey HKCU "${UNINSTKEY}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
