; NeverWork Windows 安装包（NSIS 3 / Unicode）
;
; 构建：installer\build.ps1（会先编 release，再生成图标，最后调 makensis）
; 直接编译：makensis installer\neverwork.nsi
; 产出：NeverWork\dist\NeverWork-<版本>-setup.exe
;
; 设计取舍：
; - 按用户安装（$LOCALAPPDATA\Programs\NeverWork），全程不弹 UAC；
; - 卸载不删 %USERPROFILE%\.neverwork（里面有会话、设置与 API Key），
;   避免误删用户数据；
; - 安装/卸载前检测 neverwork.exe 是否在跑：交互模式先征求同意再关闭，
;   静默模式（/S）直接关闭，避免文件占用导致安装失败。

Unicode true

!include "MUI2.nsh"
!include "LogicLib.nsh"

!define APP_NAME "NeverWork"
; 版本默认值：installer\build.ps1 会从 workspace Cargo.toml 读取并用 /D 传入；
; 直接 makensis 时退回到这里的默认值。发版时记得同步 crates\app\assets\neverwork.rc。
!ifndef APP_VERSION
  !define APP_VERSION "0.2.0"
!endif
; CI 交叉编译 arm64 安装包时用 /D 指到别的产物目录；本地构建保持默认。
!ifndef APP_EXE_DIR
  !define APP_EXE_DIR "..\target\release"
!endif
!ifndef OUT_NAME
  !define OUT_NAME "NeverWork-${APP_VERSION}-setup.exe"
!endif
!define APP_EXE "neverwork.exe"
!define APP_PUBLISHER "NeverWork"
!define APP_REG "Software\NeverWork"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\NeverWork"

Name "${APP_NAME}"
BrandingText "${APP_NAME} ${APP_VERSION}"
OutFile "..\dist\${OUT_NAME}"
InstallDir "$LOCALAPPDATA\Programs\NeverWork"
InstallDirRegKey HKCU "${APP_REG}" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetCompressorDictSize 64

VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey "ProductName" "${APP_NAME}"
VIAddVersionKey "FileDescription" "${APP_NAME} 安装程序"
VIAddVersionKey "FileVersion" "${APP_VERSION}"
VIAddVersionKey "ProductVersion" "${APP_VERSION}"
VIAddVersionKey "LegalCopyright" "Apache-2.0"

!define MUI_ICON "${__FILEDIR__}\..\crates\app\assets\neverwork.ico"
!define MUI_UNICON "${__FILEDIR__}\..\crates\app\assets\neverwork.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "立即启动 ${APP_NAME}（首次使用在「设置」里填 API Key）"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

LangString DESC_SEC_MAIN ${LANG_SIMPCHINESE} "NeverWork 主程序与卸载器。"
LangString DESC_SEC_MAIN ${LANG_ENGLISH} "The NeverWork application and uninstaller."
LangString DESC_SEC_DESKTOP ${LANG_SIMPCHINESE} "在桌面创建 NeverWork 快捷方式。"
LangString DESC_SEC_DESKTOP ${LANG_ENGLISH} "Create a NeverWork shortcut on the desktop."
LangString APP_RUNNING ${LANG_SIMPCHINESE} "NeverWork 正在运行，安装程序需要先关闭它。$\r$\n点击「确定」自动关闭并继续，点击「取消」退出安装。"
LangString APP_RUNNING ${LANG_ENGLISH} "NeverWork is running and must be closed first.$\r$\nClick OK to close it and continue, or Cancel to abort."
LangString UN_RUNNING ${LANG_SIMPCHINESE} "NeverWork 正在运行，卸载程序需要先关闭它。$\r$\n点击「确定」自动关闭并继续，点击「取消」退出卸载。"
LangString UN_RUNNING ${LANG_ENGLISH} "NeverWork is running and must be closed first.$\r$\nClick OK to close it and continue, or Cancel to abort."

; ---------- 运行中检测 ----------

; 检查 neverwork.exe 是否在跑；交互模式征求同意后 taskkill，静默模式直接关。
Function CheckRunning
  nsExec::ExecToStack 'cmd /c tasklist /FI "IMAGENAME eq ${APP_EXE}" /NH | find /I "${APP_EXE}"'
  Pop $0                     ; 0 = 找到进程
  Pop $1                     ; 输出（不用）
  ${If} $0 == "0"
    IfSilent kill
    MessageBox MB_OKCANCEL|MB_ICONEXCLAMATION "$(APP_RUNNING)" IDOK kill IDCANCEL abort
    abort:
      Abort
    kill:
      nsExec::Exec 'taskkill /IM ${APP_EXE} /F'
      Sleep 600
  ${EndIf}
FunctionEnd

Function .onInit
  Call CheckRunning
FunctionEnd

Function un.onInit
  Call un.CheckRunning
FunctionEnd

Function un.CheckRunning
  nsExec::ExecToStack 'cmd /c tasklist /FI "IMAGENAME eq ${APP_EXE}" /NH | find /I "${APP_EXE}"'
  Pop $0
  Pop $1
  ${If} $0 == "0"
    IfSilent kill
    MessageBox MB_OKCANCEL|MB_ICONEXCLAMATION "$(UN_RUNNING)" IDOK kill IDCANCEL abort
    abort:
      Abort
    kill:
      nsExec::Exec 'taskkill /IM ${APP_EXE} /F'
      Sleep 600
  ${EndIf}
FunctionEnd

; ---------- 安装 ----------

Section "主程序" SEC_MAIN
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "${APP_EXE_DIR}\${APP_EXE}"
  File "..\crates\app\assets\neverwork.ico"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  WriteRegStr HKCU "${APP_REG}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${APP_PUBLISHER}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\neverwork.ico"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1

  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortCut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\neverwork.ico"
SectionEnd

Section "桌面快捷方式" SEC_DESKTOP
  CreateShortCut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\neverwork.ico"
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_MAIN} $(DESC_SEC_MAIN)
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_DESKTOP} $(DESC_SEC_DESKTOP)
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; ---------- 卸载 ----------

Section "Uninstall"
  Delete "$INSTDIR\${APP_EXE}"
  Delete "$INSTDIR\neverwork.ico"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk"
  RMDir "$SMPROGRAMS\${APP_NAME}"
  Delete "$DESKTOP\${APP_NAME}.lnk"

  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "${APP_REG}"
SectionEnd
