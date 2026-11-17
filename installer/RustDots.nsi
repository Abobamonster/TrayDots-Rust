Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "WinVer.nsh"
!include "x64.nsh"

!define APP "RustDots"
!define EXE "RustDots.exe"
!define BUILD_EXE "rust-dots.exe"
!define PREVIOUS_EXE "tray-dots-rust.exe"
!define STAGED_EXE "RustDots.new"
!define STARTUP "RustDots"
!define PREVIOUS_STARTUP "TrayDotsRust"
!define UNINSTALLER "Uninstall.exe"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define APPROVAL_KEY "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\TrayDotsRust"

Name "${APP}"
Caption "${APP} ${APP_VERSION}"
OutFile "..\target\RustDots-${APP_VERSION}.exe"
InstallDir "$LOCALAPPDATA\TrayDotsRust"

VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "${APP}"
VIAddVersionKey /LANG=1033 "FileDescription" "RustDots Setup"
VIAddVersionKey /LANG=1033 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" ""

!define MUI_ICON "RustDots-Setup.ico"
!define MUI_WELCOMEPAGE_TEXT "$(WelcomeText)"
!define MUI_FINISHPAGE_TEXT "$(FinishedText)"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Russian"

LangString WelcomeText ${LANG_ENGLISH} "${APP} opens hidden notification icons and customizes the taskbar.$\r$\n$\r$\nSetup installs it for your account, starts it in the background and enables startup at sign-in."
LangString WelcomeText ${LANG_RUSSIAN} "${APP} открывает скрытые значки и настраивает фон панели задач.$\r$\n$\r$\nУстановщик добавит программу для вашей учётной записи, запустит её в фоне и включит автозапуск при входе в Windows."
LangString FinishedText ${LANG_ENGLISH} "${APP} is installed. Click the arrow by the notification area to open your hidden icons."
LangString FinishedText ${LANG_RUSSIAN} "${APP} установлен. Нажмите стрелку возле часов, чтобы открыть скрытые значки."
LangString Unsupported ${LANG_ENGLISH} "${APP} requires Windows 10 x64, version 2004 through 22H2."
LangString Unsupported ${LANG_RUSSIAN} "Для ${APP} нужна Windows 10 x64 версии от 2004 до 22H2."
LangString Busy ${LANG_ENGLISH} "${APP} could not be closed. Exit it from the arrow's context menu and try again."
LangString Busy ${LANG_RUSSIAN} "Не удалось закрыть ${APP}. Выйдите из программы через меню стрелки и попробуйте снова."
LangString SetupBusy ${LANG_ENGLISH} "Another ${APP} installer or uninstaller is already running."
LangString SetupBusy ${LANG_RUSSIAN} "Установщик или программа удаления ${APP} уже запущены."
LangString InstallError ${LANG_ENGLISH} "Setup could not finish. Check that you can write to the installation folder and try again."
LangString InstallError ${LANG_RUSSIAN} "Установка не завершилась. Проверьте доступ к папке установки и попробуйте снова."

!macro InitSetup
    SetShellVarContext current
    SetRegView 64
    System::Call 'kernel32::CreateMutexW(p 0, i 0, w "Local\TrayDotsRust.Setup.1") p.r0 ?e'
    Pop $1
    ${If} $0 == 0
    ${OrIf} $1 == 183
        MessageBox MB_ICONEXCLAMATION|MB_OK "$(SetupBusy)" /SD IDOK
        SetErrorLevel 1
        Abort
    ${EndIf}
!macroend

Function .onInit
    !insertmacro InitSetup
    ${IfNot} ${IsNativeAMD64}
    ${OrIfNot} ${AtLeastBuild} 19041
    ${OrIf} ${AtLeastBuild} 22000
        MessageBox MB_ICONEXCLAMATION|MB_OK "$(Unsupported)" /SD IDOK
        SetErrorLevel 1
        Abort
    ${EndIf}
    StrCpy $INSTDIR "$LOCALAPPDATA\TrayDotsRust"
FunctionEnd

Function un.onInit
    !insertmacro InitSetup
FunctionEnd

!macro DefineStopRustDots Prefix
Function ${Prefix}ExecutableReleased
    Pop $1
    ${If} ${FileExists} "$1"
        System::Call 'kernel32::CreateFileW(w r1, i 0x40000000, i 7, p 0, i 3, i 0x80, p 0) p.r1'
        ${If} $1 == -1
            Push 0
            Return
        ${EndIf}
        System::Call 'kernel32::CloseHandle(p r1)'
    ${EndIf}
    Push 1
FunctionEnd

Function ${Prefix}StopRustDots
    FindWindow $0 "TrayDotsRust.Controller.1"
    ${If} $0 != 0
        SendMessage $0 ${WM_CLOSE} 0 0 /TIMEOUT=1000
    ${EndIf}

    ; Both executables share the controller class and recovery files.
    ${For} $2 1 80
        FindWindow $0 "TrayDotsRust.Controller.1"
        ${If} $0 == 0
            Push "$INSTDIR\${EXE}"
            Call ${Prefix}ExecutableReleased
            Pop $1
            ${If} $1 == 1
                Push "$INSTDIR\${PREVIOUS_EXE}"
                Call ${Prefix}ExecutableReleased
                Pop $1
                ${If} $1 == 1
                    Return
                ${EndIf}
            ${EndIf}
        ${EndIf}
        Sleep 100
    ${Next}
    MessageBox MB_ICONEXCLAMATION|MB_OK "$(Busy)" /SD IDOK
    SetErrorLevel 1
    Abort
FunctionEnd
!macroend
!insertmacro DefineStopRustDots ""
!insertmacro DefineStopRustDots "un."

Function RemovePreviousShortcut
    Pop $0
    ${IfNot} ${FileExists} "$0"
        Return
    ${EndIf}

    ; Only remove a shortcut if its target matches the previous Rust installer.
    System::Call 'ole32::CoCreateInstance(g "{00021401-0000-0000-C000-000000000046}", p 0, i 1, g "{000214F9-0000-0000-C000-000000000046}", *p .r1) i.r4'
    ${If} $4 != 0
        Return
    ${EndIf}
    System::Call '$1->0(g "{0000010B-0000-0000-C000-000000000046}", *p .r2) i.r4'
    ${If} $4 == 0
        System::Call '$2->5(w r0, i 0) i.r4'
        ${If} $4 == 0
            System::Call '$1->3(w .r3, i ${NSIS_MAX_STRLEN}, p 0, i 4) i.r4'
            ${If} $4 == 0
            ${AndIf} $3 == "$INSTDIR\${PREVIOUS_EXE}"
                Delete "$0"
            ${EndIf}
        ${EndIf}
        System::Call '$2->2()'
    ${EndIf}
    System::Call '$1->2()'
FunctionEnd

Section "${APP}"
    ClearErrors
    SetOutPath "$INSTDIR"
    File /oname=${STAGED_EXE} "..\target\release\${BUILD_EXE}"
    IfErrors install_error
    Call StopRustDots
    System::Call 'kernel32::MoveFileExW(w "$INSTDIR\${STAGED_EXE}", w "$INSTDIR\${EXE}", i 9) i.r0'
    ${If} $0 == 0
        ${If} ${FileExists} "$INSTDIR\${EXE}"
            Exec '"$INSTDIR\${EXE}" --background'
        ${ElseIf} ${FileExists} "$INSTDIR\${PREVIOUS_EXE}"
            Exec '"$INSTDIR\${PREVIOUS_EXE}" --background'
        ${EndIf}
        Goto install_error
    ${EndIf}

    ClearErrors
    WriteUninstaller "$INSTDIR\${UNINSTALLER}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${APP_VERSION}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${EXE},0"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\${UNINSTALLER}"'
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
    WriteRegStr HKCU "${RUN_KEY}" "${STARTUP}" '"$INSTDIR\${EXE}" --background'
    IfErrors install_error
    DeleteRegValue HKCU "${RUN_KEY}" "${PREVIOUS_STARTUP}"
    DeleteRegValue HKCU "${APPROVAL_KEY}" "${PREVIOUS_STARTUP}"
    DeleteRegValue HKCU "${APPROVAL_KEY}" "${STARTUP}"

    CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE}"
    Push "$SMPROGRAMS\TrayDots\TrayDots.lnk"
    Call RemovePreviousShortcut
    Push "$DESKTOP\TrayDots.lnk"
    Call RemovePreviousShortcut
    Push "$DESKTOP\TrayDots Rust.lnk"
    Call RemovePreviousShortcut
    Push "$SMPROGRAMS\TrayDots\Остановить и убрать автозапуск.lnk"
    Call RemovePreviousShortcut
    RMDir "$SMPROGRAMS\TrayDots"

    ClearErrors
    Exec '"$INSTDIR\${EXE}" --background'
    IfErrors install_error
    Delete "$INSTDIR\${PREVIOUS_EXE}"
    Goto installed
install_error:
    Delete "$INSTDIR\${STAGED_EXE}"
    MessageBox MB_ICONSTOP|MB_OK "$(InstallError)" /SD IDOK
    SetErrorLevel 1
    Abort
installed:
SectionEnd

Section "Uninstall"
    Call un.StopRustDots
    ClearErrors
    Delete "$INSTDIR\${EXE}"
    ${If} ${Errors}
        MessageBox MB_ICONEXCLAMATION|MB_OK "$(Busy)" /SD IDOK
        SetErrorLevel 1
        Abort
    ${EndIf}
    DeleteRegValue HKCU "${RUN_KEY}" "${STARTUP}"
    DeleteRegValue HKCU "${APPROVAL_KEY}" "${STARTUP}"
    DeleteRegKey HKCU "${UNINSTALL_KEY}"
    Delete "$SMPROGRAMS\${APP}.lnk"
    Delete "$INSTDIR\${UNINSTALLER}"
    RMDir "$INSTDIR"
SectionEnd
