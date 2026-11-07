; ==============================================================================
; Nuva Installer Script (NSIS)
; Modern, lightweight, per-user setup bundling Nuva
; Compiles to: NusaSetup.exe
; ==============================================================================

Unicode True

!define PRODUCT_NAME "Nuva"
!define PRODUCT_VERSION "1.2.0"
!define PRODUCT_PUBLISHER "Nuva"
!define PRODUCT_DESCRIPTION "A living notch for Windows"
!define PRODUCT_EXE "nuva.exe"
!define PRODUCT_UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Nuva"
!define PRODUCT_RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

; Compression settings for minimal installer size
SetCompressor /SOLID lzma

; Setup Information
Name "${PRODUCT_NAME}"
Caption "${PRODUCT_NAME} ${PRODUCT_VERSION} Setup"
OutFile "..\NuvaSetup.exe"
InstallDir "$LOCALAPPDATA\Programs\Nuva"
InstallDirRegKey HKCU "${PRODUCT_UNINST_KEY}" "InstallLocation"
RequestExecutionLevel user

; Version resources for the setup executable itself
VIProductVersion "${PRODUCT_VERSION}.0"
VIFileVersion "${PRODUCT_VERSION}.0"
VIAddVersionKey "ProductName" "${PRODUCT_NAME}"
VIAddVersionKey "ProductVersion" "${PRODUCT_VERSION}"
VIAddVersionKey "FileDescription" "${PRODUCT_NAME} Setup"
VIAddVersionKey "FileVersion" "${PRODUCT_VERSION}"
VIAddVersionKey "LegalCopyright" "Copyright (C) 2026 ${PRODUCT_PUBLISHER}"

; Modern UI 2
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

; Interface settings
!define MUI_ICON "..\assets\app.ico"
!define MUI_UNICON "..\assets\app.ico"
!define MUI_ABORTWARNING

; Welcome page
!insertmacro MUI_PAGE_WELCOME

; Components page (Desktop shortcut, Autostart)
!insertmacro MUI_PAGE_COMPONENTS

; Directory selection page
!insertmacro MUI_PAGE_DIRECTORY

; Installation progress page
!insertmacro MUI_PAGE_INSTFILES

; Finish page
!define MUI_FINISHPAGE_RUN "$INSTDIR\${PRODUCT_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Launch Nuva now"
!insertmacro MUI_PAGE_FINISH

; Uninstaller pages
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

; Language
!insertmacro MUI_LANGUAGE "English"

; ------------------------------------------------------------------------------
; Helper Functions to stop running instances
; ------------------------------------------------------------------------------
Function StopNuva
    ; Gracefully stop running Nuva or legacy Nusa instances
    nsExec::Exec 'taskkill /IM nuva.exe'
    nsExec::Exec 'taskkill /IM nusa.exe'
    Sleep 800
    ; Force stop if still lingering
    nsExec::Exec 'taskkill /F /IM nuva.exe'
    nsExec::Exec 'taskkill /F /IM nusa.exe'
FunctionEnd

Function un.StopNuva
    nsExec::Exec 'taskkill /IM nuva.exe'
    nsExec::Exec 'taskkill /IM nusa.exe'
    Sleep 800
    nsExec::Exec 'taskkill /F /IM nuva.exe'
    nsExec::Exec 'taskkill /F /IM nusa.exe'
FunctionEnd

; ------------------------------------------------------------------------------
; Installer Sections
; ------------------------------------------------------------------------------
Section "Nuva Core Application (required)" SecCore
    SectionIn RO

    DetailPrint "Closing running instances of Nuva..."
    Call StopNuva

    SetOutPath "$INSTDIR"
    File "..\target\release\nuva.exe"

    ; Create uninstaller
    WriteUninstaller "$INSTDIR\Uninstall.exe"

    ; Start Menu Shortcut
    CreateDirectory "$SMPROGRAMS"
    CreateShortcut "$SMPROGRAMS\Nuva.lnk" "$INSTDIR\${PRODUCT_EXE}" "" "$INSTDIR\${PRODUCT_EXE}" 0 SW_SHOWNORMAL "" "${PRODUCT_DESCRIPTION}"

    ; Clean up legacy Nusa shortcuts / run key
    Delete "$SMPROGRAMS\Nusa.lnk"
    DeleteRegValue HKCU "${PRODUCT_RUN_KEY}" "Nusa"

    ; Windows Add / Remove Programs registration
    WriteRegStr HKCU "${PRODUCT_UNINST_KEY}" "DisplayName" "${PRODUCT_NAME}"
    WriteRegStr HKCU "${PRODUCT_UNINST_KEY}" "DisplayVersion" "${PRODUCT_VERSION}"
    WriteRegStr HKCU "${PRODUCT_UNINST_KEY}" "Publisher" "${PRODUCT_PUBLISHER}"
    WriteRegStr HKCU "${PRODUCT_UNINST_KEY}" "DisplayIcon" "$INSTDIR\${PRODUCT_EXE},0"
    WriteRegStr HKCU "${PRODUCT_UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
    WriteRegStr HKCU "${PRODUCT_UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
    WriteRegStr HKCU "${PRODUCT_UNINST_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegDWORD HKCU "${PRODUCT_UNINST_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${PRODUCT_UNINST_KEY}" "NoRepair" 1

    ; Compute and store estimated size
    ${GetSize} "$INSTDIR" "/S=K" $0 $1 $2
    IntFmt $0 "0x%08X" $0
    WriteRegDWORD HKCU "${PRODUCT_UNINST_KEY}" "EstimatedSize" "$0"

    ; Invalidate Windows Explorer icon cache so updated icons appear immediately
    System::Call 'shell32.dll::SHChangeNotify(i, i, p, p) v (0x08000000, 0, 0, 0)'
SectionEnd

Section "Start Nuva with Windows" SecAutostart
    WriteRegStr HKCU "${PRODUCT_RUN_KEY}" "Nuva" '"$INSTDIR\${PRODUCT_EXE}"'
SectionEnd

Section "Create Desktop Shortcut" SecDesktop
    CreateShortcut "$DESKTOP\Nuva.lnk" "$INSTDIR\${PRODUCT_EXE}" "" "$INSTDIR\${PRODUCT_EXE}" 0 SW_SHOWNORMAL "" "${PRODUCT_DESCRIPTION}"
SectionEnd

; Descriptions for components
LangString DESC_SecCore ${LANG_ENGLISH} "The core Nuva executable and Start Menu integration."
LangString DESC_SecAutostart ${LANG_ENGLISH} "Automatically launches Nuva when you sign in to Windows."
LangString DESC_SecDesktop ${LANG_ENGLISH} "Places a shortcut to Nuva on your Desktop."

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
    !insertmacro MUI_DESCRIPTION_TEXT ${SecCore} $(DESC_SecCore)
    !insertmacro MUI_DESCRIPTION_TEXT ${SecAutostart} $(DESC_SecAutostart)
    !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} $(DESC_SecDesktop)
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; ------------------------------------------------------------------------------
; Uninstaller Section
; ------------------------------------------------------------------------------
Section "Uninstall"
    DetailPrint "Closing running instances of Nuva..."
    Call un.StopNuva

    ; Remove Shortcuts
    Delete "$SMPROGRAMS\Nuva.lnk"
    Delete "$SMPROGRAMS\Nusa.lnk"
    Delete "$DESKTOP\Nuva.lnk"

    ; Remove Autostart Entries
    DeleteRegValue HKCU "${PRODUCT_RUN_KEY}" "Nuva"
    DeleteRegValue HKCU "${PRODUCT_RUN_KEY}" "Nusa"

    ; Remove Windows Add/Remove entry
    DeleteRegKey HKCU "${PRODUCT_UNINST_KEY}"

    ; Remove App Files
    Delete "$INSTDIR\${PRODUCT_EXE}"
    Delete "$INSTDIR\Uninstall.exe"
    RMDir "$INSTDIR"

    ; Prompt to remove user data/settings (skipped in silent mode)
    IfSilent keep_data
    MessageBox MB_YESNO|MB_ICONQUESTION "Do you also want to remove your Nuva settings and data (tasks, water log, shelf)?$\r$\n$\r$\nClick 'Yes' to remove all data, or 'No' to preserve it." IDNO keep_data
        RMDir /r "$APPDATA\Nuva"
        RMDir /r "$LOCALAPPDATA\Nuva"
        RMDir /r "$APPDATA\Nusa"
        RMDir /r "$LOCALAPPDATA\Nusa"
    keep_data:

    ; Refresh icon cache
    System::Call 'shell32.dll::SHChangeNotify(i, i, p, p) v (0x08000000, 0, 0, 0)'
SectionEnd

