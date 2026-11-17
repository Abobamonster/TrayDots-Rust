Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "WinVer.nsh"
!include "x64.nsh"

!define APP "TrayDots Rust"
!define EXE "tray-dots-rust.exe"
!define UNINSTALLER "Uninstall.exe"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define APPROVAL_KEY "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\TrayDotsRust"

Name "${APP}"
Caption "${APP} ${APP_VERSION}"
OutFile "..\target\TrayDots-Rust-v${APP_VERSION}-Windows10-x64-Setup.exe"
InstallDir "$LOCALAPPDATA\TrayDotsRust"

VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "${APP}"
VIAddVersionKey /LANG=1033 "FileDescription" "TrayDots Rust Setup"
VIAddVersionKey /LANG=1033 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" ""

!define MUI_WELCOMEPAGE_TEXT "$(WelcomeText)"
!define MUI_FINISHPAGE_TEXT "$(FinishedText)"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Russian"

LangString WelcomeText ${LANG_ENGLISH} "TrayDots Rust opens hidden notification icons and customizes the taskbar.$\r$\n$\r$\nSetup installs it for your account, starts it in the background and enables startup at sign-in."
LangString WelcomeText ${LANG_RUSSIAN} "TrayDots Rust открывает скрытые значки и настраивает фон панели задач.$\r$\n$\r$\nУстановщик добавит программу для вашей учётной записи, запустит её в фоне и включит автозапуск при входе в Windows."
LangString FinishedText ${LANG_ENGLISH} "TrayDots Rust is installed. Click the arrow by the notification area to open your hidden icons."
LangString FinishedText ${LANG_RUSSIAN} "TrayDots Rust установлен. Нажмите стрелку возле часов, чтобы открыть скрытые значки."
LangString Unsupported ${LANG_ENGLISH} "TrayDots Rust requires Windows 10 x64, version 2004 through 22H2."
LangString Unsupported ${LANG_RUSSIAN} "Для TrayDots Rust нужна Windows 10 x64 версии от 2004 до 22H2."
LangString Busy ${LANG_ENGLISH} "TrayDots Rust could not be closed. Exit it from the arrow's context menu and try again."
LangString Busy ${LANG_RUSSIAN} "Не удалось закрыть TrayDots Rust. Выйдите из программы через меню стрелки и попробуйте снова."
LangString SetupBusy ${LANG_ENGLISH} "Another TrayDots Rust installer or uninstaller is already running."
LangString SetupBusy ${LANG_RUSSIAN} "Установщик или программа удаления TrayDots Rust уже запущены."
LangString InstallError ${LANG_ENGLISH} "Setup could not finish. Check that you can write to the TrayDots Rust folder and try again."
LangString InstallError ${LANG_RUSSIAN} "Установка не завершилась. Проверьте доступ к папке TrayDots Rust и попробуйте снова."

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

!macro DefineStopTrayDots Prefix
Function ${Prefix}StopTrayDots
    FindWindow $0 "TrayDotsRust.Controller.1"
    ${If} $0 != 0
        SendMessage $0 ${WM_CLOSE} 0 0 /TIMEOUT=1000
    ${EndIf}

    ; Wait for the controller and its recovery process to release the EXE.
    ${For} $2 1 80
        FindWindow $0 "TrayDotsRust.Controller.1"
        ${If} $0 == 0
            ${IfNot} ${FileExists} "$INSTDIR\${EXE}"
                Return
            ${EndIf}
            System::Call 'kernel32::CreateFileW(w "$INSTDIR\${EXE}", i 0x40000000, i 7, p 0, i 3, i 0x80, p 0) p.r1'
            ${If} $1 != -1
                System::Call 'kernel32::CloseHandle(p r1)'
                Return
            ${EndIf}
        ${EndIf}
        Sleep 100
    ${Next}
    MessageBox MB_ICONEXCLAMATION|MB_OK "$(Busy)" /SD IDOK
    SetErrorLevel 1
    Abort
FunctionEnd
!macroend
!insertmacro DefineStopTrayDots ""
!insertmacro DefineStopTrayDots "un."

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
            ${AndIf} $3 == "$INSTDIR\${EXE}"
                Delete "$0"
            ${EndIf}
        ${EndIf}
        System::Call '$2->2()'
    ${EndIf}
    System::Call '$1->2()'
FunctionEnd

Section "TrayDots Rust"
    ClearErrors
    SetOutPath "$INSTDIR"
    File /oname=tray-dots-rust.new "..\target\release\${EXE}"
    IfErrors install_error
    Call StopTrayDots
    System::Call 'kernel32::MoveFileExW(w "$INSTDIR\tray-dots-rust.new", w "$INSTDIR\${EXE}", i 9) i.r0'
    ${If} $0 == 0
        IfFileExists "$INSTDIR\${EXE}" 0 install_error
        Exec '"$INSTDIR\${EXE}" --background'
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
    WriteRegStr HKCU "${RUN_KEY}" "TrayDotsRust" '"$INSTDIR\${EXE}" --background'
    IfErrors install_error
    DeleteRegValue HKCU "${APPROVAL_KEY}" "TrayDotsRust"

    CreateDirectory "$SMPROGRAMS\TrayDots"
    CreateShortcut "$SMPROGRAMS\TrayDots\TrayDots.lnk" "$INSTDIR\${EXE}"
    Push "$DESKTOP\TrayDots.lnk"
    Call RemovePreviousShortcut
    Push "$DESKTOP\TrayDots Rust.lnk"
    Call RemovePreviousShortcut
    Push "$SMPROGRAMS\TrayDots\Остановить и убрать автозапуск.lnk"
    Call RemovePreviousShortcut

    ClearErrors
    Exec '"$INSTDIR\${EXE}" --background'
    IfErrors install_error
    Goto installed
install_error:
    Delete "$INSTDIR\tray-dots-rust.new"
    MessageBox MB_ICONSTOP|MB_OK "$(InstallError)" /SD IDOK
    SetErrorLevel 1
    Abort
installed:
SectionEnd

Section "Uninstall"
    Call un.StopTrayDots
    ClearErrors
    Delete "$INSTDIR\${EXE}"
    ${If} ${Errors}
        MessageBox MB_ICONEXCLAMATION|MB_OK "$(Busy)" /SD IDOK
        SetErrorLevel 1
        Abort
    ${EndIf}
    DeleteRegValue HKCU "${RUN_KEY}" "TrayDotsRust"
    DeleteRegValue HKCU "${APPROVAL_KEY}" "TrayDotsRust"
    DeleteRegKey HKCU "${UNINSTALL_KEY}"
    Delete "$SMPROGRAMS\TrayDots\TrayDots.lnk"
    RMDir "$SMPROGRAMS\TrayDots"
    Delete "$INSTDIR\${UNINSTALLER}"
    RMDir "$INSTDIR"
SectionEnd
