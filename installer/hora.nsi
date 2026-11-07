; Installer für Hora (NSIS 3, Unicode). Installiert für den angemeldeten
; Benutzer nach %LOCALAPPDATA%\Programs\Hora, ohne Administratorrechte.
; Aufruf über tools\build-installer.ps1, oder direkt:
;   makensis /DVERSION=0.5.0 /DEXE=C:\pfad\hora.exe /DOUT=C:\pfad\Hora-Setup.exe installer\hora.nsi

Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

!ifndef VERSION
  !error "VERSION fehlt (z. B. /DVERSION=0.5.0)"
!endif
!ifndef EXE
  !error "EXE fehlt (Pfad zu hora.exe)"
!endif
!ifndef OUT
  !define OUT "Hora-Setup-${VERSION}.exe"
!endif

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Hora"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

Name "Hora"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\Programs\Hora"
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
RequestExecutionLevel user
SetCompressor /SOLID lzma
ManifestDPIAware true
BrandingText "Hora ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=1031 "ProductName" "Hora"
VIAddVersionKey /LANG=1031 "FileDescription" "Hora Kalender – Installation"
VIAddVersionKey /LANG=1031 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1031 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1031 "LegalCopyright" "Copyright (c) 2026 lenkr0, MIT-Lizenz"

!define MUI_ICON "..\crates\app\assets\hora.ico"
!define MUI_UNICON "..\crates\app\assets\hora.ico"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TEXT "Hora ist ein nativer CalDAV-Kalender für Windows.$\r$\n$\r$\nDie Installation gilt nur für dein Benutzerkonto und braucht keine Administratorrechte."
!define MUI_FINISHPAGE_RUN "$INSTDIR\hora.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Hora jetzt starten"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "German"

; Läuft Hora schon (Update), vor dem Überschreiben beenden
!macro QuitHora
  ${If} ${FileExists} "$INSTDIR\hora.exe"
    ExecWait '"$INSTDIR\hora.exe" --quit'
    Sleep 1500
  ${EndIf}
!macroend

Section "Hora (erforderlich)" SecMain
  SectionIn RO
  SetShellVarContext current
  !insertmacro QuitHora
  SetOutPath "$INSTDIR"
  File "/oname=hora.exe" "${EXE}"
  File "..\LICENSE"
  File "..\NOTICE.md"
  SetOutPath "$INSTDIR\THIRD_PARTY_LICENSES"
  File "..\THIRD_PARTY_LICENSES\*.txt"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Hora.lnk" "$INSTDIR\hora.exe" "" "$INSTDIR\hora.exe" 0 SW_SHOWNORMAL "" "Nativer CalDAV-Kalender"

  ; Eintrag unter Einstellungen → Apps
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "Hora"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "lenkr0"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\hora.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" $0
SectionEnd

Section "Mit Windows starten" SecAutostart
  WriteRegStr HKCU "${RUN_KEY}" "Hora" '"$INSTDIR\hora.exe" --tray'
SectionEnd

Section /o "Verknüpfung auf dem Desktop" SecDesktop
  SetShellVarContext current
  CreateShortcut "$DESKTOP\Hora.lnk" "$INSTDIR\hora.exe"
SectionEnd

LangString DESC_Main ${LANG_GERMAN} "Das Programm, ein Eintrag im Startmenü und die Deinstallation."
LangString DESC_Autostart ${LANG_GERMAN} "Hora startet beim Anmelden im Infobereich, damit Erinnerungen ankommen. Lässt sich jederzeit in den Einstellungen von Hora ändern."
LangString DESC_Desktop ${LANG_GERMAN} "Eine Verknüpfung zu Hora auf dem Desktop."
!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} $(DESC_Main)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecAutostart} $(DESC_Autostart)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} $(DESC_Desktop)
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  SetShellVarContext current
  !insertmacro QuitHora
  Delete "$INSTDIR\hora.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\NOTICE.md"
  RMDir /r "$INSTDIR\THIRD_PARTY_LICENSES"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Hora.lnk"
  Delete "$DESKTOP\Hora.lnk"
  DeleteRegValue HKCU "${RUN_KEY}" "Hora"
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\Hora.Kalender"
  DeleteRegKey HKCU "${UNINST_KEY}"
  ${IfNot} ${Silent}
    MessageBox MB_YESNO|MB_DEFBUTTON2|MB_ICONQUESTION "Auch die lokalen Daten von Hora löschen (zwischengespeicherte Termine, Einstellungen)?$\r$\n$\r$\nDie Termine auf deinem CalDAV-Server bleiben unberührt." IDNO keep
      RMDir /r "$LOCALAPPDATA\Hora"
    keep:
  ${EndIf}
SectionEnd
