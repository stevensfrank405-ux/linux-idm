Unicode true
!include "MUI2.nsh"
!include "x64.nsh"
Name "OpenDownload"
OutFile "..\..\dist\OpenDownload-Setup-x64.exe"
InstallDir "$LOCALAPPDATA\Programs\OpenDownload"
RequestExecutionLevel user
!define MUI_ICON "..\..\assets\opendownload.ico"
!define MUI_UNICON "..\..\assets\opendownload.ico"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "..\..\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\OpenDownload.exe"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Section "OpenDownload" Main
  SetOutPath "$INSTDIR"
  File "..\..\dist\windows\OpenDownload.exe"
  !if /FileExists "..\..\dist\windows\*.dll"
    File "..\..\dist\windows\*.dll"
  !endif
  File "..\..\README.md"
  File "..\..\LICENSE"
  SetOutPath "$INSTDIR\extensions"
  File /r "..\..\dist\windows\extensions\*"
  CreateDirectory "$SMPROGRAMS\OpenDownload"
  CreateShortcut "$SMPROGRAMS\OpenDownload\OpenDownload.lnk" "$INSTDIR\OpenDownload.exe"
  CreateShortcut "$DESKTOP\OpenDownload.lnk" "$INSTDIR\OpenDownload.exe"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenDownload" "DisplayName" "OpenDownload"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenDownload" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenDownload" "DisplayVersion" "0.3.0"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenDownload" "Publisher" "OpenDownload contributors"
SectionEnd
Section "Uninstall"
  Delete "$DESKTOP\OpenDownload.lnk"
  Delete "$SMPROGRAMS\OpenDownload\OpenDownload.lnk"
  RMDir "$SMPROGRAMS\OpenDownload"
  Delete "$INSTDIR\OpenDownload.exe"
  Delete "$INSTDIR\libwinpthread-1.dll"
  Delete "$INSTDIR\libgcc_s_seh-1.dll"
  Delete "$INSTDIR\libstdc++-6.dll"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir /r "$INSTDIR\extensions"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenDownload"
  ; User downloads, settings, and queue are intentionally retained.
SectionEnd
