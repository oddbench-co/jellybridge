; JellyBridge per-user installer. Built by scripts/build-windows.sh with
; makensis -DVERSION=x.y.z -DSTAGE=<dir with jellybridge.exe, libmpv-2.dll,
; vulkan-1.dll, VulkanRT-License.txt, jellybridge.ico> -DOUTFILE=<path>.
; Per-user: no administrator prompt, installs to %LOCALAPPDATA%\Programs.
Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user
!include "MUI2.nsh"

Name "JellyBridge"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\JellyBridge"
InstallDirRegKey HKCU "Software\JellyBridge" "InstallDir"
BrandingText "JellyBridge ${VERSION}"

!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\JellyBridge"
!define MUI_ICON "${STAGE}\jellybridge.ico"
!define MUI_UNICON "${STAGE}\jellybridge.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\jellybridge.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start JellyBridge"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "JellyBridge"
VIAddVersionKey "FileDescription" "JellyBridge installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "Oddbench"
VIAddVersionKey "LegalCopyright" "Oddbench"

Section "JellyBridge"
    ; An update replaces a running copy's files: ask it to close first.
    nsExec::Exec 'taskkill /IM jellybridge.exe'
    SetOutPath "$INSTDIR"
    File "${STAGE}\jellybridge.exe"
    File "${STAGE}\libmpv-2.dll"
    File "${STAGE}\vulkan-1.dll"
    File "${STAGE}\VulkanRT-License.txt"
    File "${STAGE}\jellybridge.ico"
    WriteUninstaller "$INSTDIR\uninstall.exe"
    CreateShortcut "$SMPROGRAMS\JellyBridge.lnk" "$INSTDIR\jellybridge.exe" "" "$INSTDIR\jellybridge.ico"
    WriteRegStr HKCU "Software\JellyBridge" "InstallDir" "$INSTDIR"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "JellyBridge"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Oddbench"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\jellybridge.ico"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
    WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
    SectionGetSize 0 $0
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" $0
SectionEnd

; Settings (%APPDATA%\JellyBridge) and the artwork cache stay, so a
; reinstall keeps the sign-in.
Section "Uninstall"
    nsExec::Exec 'taskkill /IM jellybridge.exe'
    Delete "$SMPROGRAMS\JellyBridge.lnk"
    Delete "$INSTDIR\jellybridge.exe"
    Delete "$INSTDIR\libmpv-2.dll"
    Delete "$INSTDIR\vulkan-1.dll"
    Delete "$INSTDIR\VulkanRT-License.txt"
    Delete "$INSTDIR\jellybridge.ico"
    Delete "$INSTDIR\uninstall.exe"
    RMDir "$INSTDIR"
    DeleteRegKey HKCU "${UNINSTALL_KEY}"
    DeleteRegKey HKCU "Software\JellyBridge"
SectionEnd
