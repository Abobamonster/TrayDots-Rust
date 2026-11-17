#![windows_subsystem = "windows"]
#![allow(unsafe_op_in_unsafe_fn)]

mod glass;

use std::{
    cell::Cell,
    env,
    ffi::c_void,
    fs, io,
    mem::size_of,
    path::{Path, PathBuf},
    process::{self, Command},
    ptr::{null, null_mut},
    thread,
    time::Duration,
};
use windows_sys::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{Com::*, LibraryLoader::*, Registry::*, SystemInformation::*, Threading::*},
        UI::{Accessibility::*, HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
    core::{BOOL, GUID, w},
};

type Result<T> = std::result::Result<T, String>;
const ROOT: *const u16 = w!("TrayDotsRust.Controller.1");
const BUTTON: *const u16 = w!("TrayDotsRust.Ellipsis.1");
const EXPLORER: *const u16 = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer");
const RUN: *const u16 = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const AUTO: *const u16 = w!("EnableAutoTray");
const STARTUP: *const u16 = w!("TrayDotsRust");
const TOGGLE: u32 = WM_APP + 1;
const MENU: u32 = WM_APP + 2;
const STOP: u32 = WM_APP + 3;
const SYNC_TRAY: u32 = WM_APP + 4;
const HOTKEY: i32 = 1;

thread_local! {
    static TRAY_PARENT: Cell<HWND> = const { Cell::new(null_mut()) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn data_dir() -> Result<PathBuf> {
    Ok(
        PathBuf::from(env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is missing")?)
            .join("TrayDotsRust"),
    )
}

fn win_error(operation: &str) -> String {
    format!("{operation}: {}", io::Error::last_os_error())
}

fn io_result<T>(result: io::Result<T>) -> Result<T> {
    result.map_err(|err| err.to_string())
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

struct Lock(Handle);
impl Lock {
    fn acquire(timeout: u32) -> Result<Option<Self>> {
        unsafe {
            let handle = CreateMutexW(null(), 0, w!("Local\\TrayDotsRust.Instance.1"));
            if handle.is_null() {
                return Err(win_error("CreateMutexW"));
            }
            let handle = Handle(handle);
            match WaitForSingleObject(handle.0, timeout) {
                WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Some(Self(handle))),
                WAIT_TIMEOUT => Ok(None),
                _ => Err(win_error("WaitForSingleObject")),
            }
        }
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe { ReleaseMutex(self.0.0) };
    }
}

fn reg_auto() -> Result<Option<u32>> {
    let mut value = 0u32;
    let mut len = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            EXPLORER,
            AUTO,
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut len,
        )
    };
    match status {
        ERROR_SUCCESS => Ok(Some(value)),
        ERROR_FILE_NOT_FOUND => Ok(None),
        _ => Err(format!("Read EnableAutoTray: {status}")),
    }
}

fn delete_reg(subkey: *const u16, value: *const u16) -> Result<()> {
    match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey, value) } {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        status => Err(format!("RegDeleteKeyValueW: {status}")),
    }
}

fn reg_set(
    subkey: *const u16,
    name: *const u16,
    ty: u32,
    bytes: *const u8,
    len: u32,
) -> Result<()> {
    let status = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, subkey, name, ty, bytes.cast(), len) };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("RegSetKeyValueW: {status}"))
    }
}

fn startup(enabled: bool) -> Result<()> {
    if !enabled {
        return delete_reg(RUN, STARTUP);
    }
    let exe = io_result(env::current_exe())?;
    let quoted = wide(&format!("\"{}\" --background", exe.display()));
    reg_set(
        RUN,
        STARTUP,
        REG_SZ,
        quoted.as_ptr().cast(),
        (quoted.len() * 2) as u32,
    )
}

fn startup_enabled() -> bool {
    (unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN,
            STARTUP,
            RRF_RT_REG_SZ,
            null_mut(),
            null_mut(),
            null_mut(),
        )
    }) == ERROR_SUCCESS
}

// The only private Explorer operation used is EnableAutoTray. No callbacks,
// icon snapshots, cross-process allocations, or per-app preferences.
fn set_auto(on: bool) -> Result<()> {
    const CLASS: GUID = GUID::from_u128(0x25dead04_1eac_4911_9e3a_ad0a4ab560fd);
    const INTERFACE: GUID = GUID::from_u128(0xd133ce13_3537_48ba_93a7_afcd5d2053b4);
    type Method = unsafe extern "system" fn(*mut c_void, BOOL) -> i32;
    type Release = unsafe extern "system" fn(*mut c_void) -> u32;
    unsafe {
        let hr = CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
        if hr < 0 {
            return Err(format!("CoInitializeEx: {hr:#010x}"));
        }
        let mut tray: *mut c_void = null_mut();
        let hr = CoCreateInstance(
            &CLASS,
            null_mut(),
            CLSCTX_LOCAL_SERVER,
            &INTERFACE,
            &mut tray,
        );
        let hr = if hr >= 0 && !tray.is_null() {
            // Three IUnknown slots, then RegisterCallback, UnregisterCallback,
            // SetPreference, and EnableAutoTray.
            let vtable = *tray.cast::<*const *const c_void>();
            let enable: Method = std::mem::transmute(*vtable.add(6));
            let release: Release = std::mem::transmute(*vtable.add(2));
            let hr = enable(tray, i32::from(on));
            release(tray);
            hr
        } else if hr >= 0 {
            E_FAIL
        } else {
            hr
        };
        CoUninitialize();
        if hr >= 0 {
            Ok(())
        } else {
            Err(format!("Explorer EnableAutoTray: {hr:#010x}"))
        }
    }
}

fn journal() -> Result<PathBuf> {
    Ok(data_dir()?.join("restore-v1.txt"))
}

fn save_state(path: &Path, value: Option<u32>) -> Result<()> {
    let dir = path.parent().ok_or("No state directory")?;
    io_result(fs::create_dir_all(dir))?;
    let temp = path.with_extension("tmp");
    let value = value.map_or("-".to_string(), |v| v.to_string());
    io_result(fs::write(&temp, format!("{}\n{value}\n", process::id())))?;
    io_result(fs::rename(temp, path))
}

fn restore(path: &Path, expected_pid: Option<u32>) -> Result<()> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.to_string()),
    };
    let mut lines = raw.lines();
    let pid: u32 = lines
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or("Invalid restore record")?;
    if expected_pid.is_some_and(|expected| expected != pid) {
        return Ok(());
    }
    let value = match lines.next() {
        Some("-") => None,
        Some(raw) => Some(raw.parse::<u32>().map_err(|_| "Invalid restore value")?),
        None => return Err("Incomplete restore record".into()),
    };
    let shell_result = set_auto(value != Some(0));
    // Persist the exact previous value even if Explorer is shutting down.
    match value {
        None => delete_reg(EXPLORER, AUTO)?,
        Some(value) => reg_set(EXPLORER, AUTO, REG_DWORD, value.to_le_bytes().as_ptr(), 4)?,
    }
    shell_result?;
    io_result(fs::remove_file(path))
}

fn recover_after(pid: u32) -> Result<()> {
    unsafe {
        let parent = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if !parent.is_null() {
            let handle = Handle(parent);
            WaitForSingleObject(handle.0, INFINITE);
        }
    }
    let _lock = Lock::acquire(INFINITE)?.ok_or("Recovery lock not acquired")?;
    glass::recover_mask();
    let path = journal()?;
    for _ in 0..4 {
        if restore(&path, Some(pid)).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }
    restore(&path, Some(pid))
}

fn controller() -> HWND {
    unsafe { FindWindowW(ROOT, null()) }
}
fn tray(shell: HWND) -> HWND {
    unsafe { FindWindowExW(shell, null_mut(), w!("TrayNotifyWnd"), null()) }
}
fn chevron(parent: HWND) -> HWND {
    unsafe { FindWindowExW(parent, null_mut(), w!("Button"), null()) }
}

fn signal(message: u32) -> bool {
    let h = controller();
    !h.is_null() && unsafe { PostMessageW(h, message, 0, 0) != 0 }
}

unsafe extern "system" fn tray_reordered(
    _: HWINEVENTHOOK,
    _: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if !hwnd.is_null()
        && object == OBJID_CLIENT
        && child == CHILDID_SELF as i32
        && TRAY_PARENT.get() == hwnd
    {
        signal(SYNC_TRAY);
    }
}

fn toggle() {
    unsafe {
        let shell = FindWindowW(w!("Shell_TrayWnd"), null());
        if shell.is_null() {
            return;
        }
        let parent = tray(shell);
        if parent.is_null() {
            return;
        }
        let arrow = chevron(parent);
        if !arrow.is_null() && IsWindowVisible(arrow) != 0 {
            let mask = glass::mask_before_open();
            SendMessageTimeoutW(
                arrow,
                BM_CLICK,
                0,
                0,
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                500,
                null_mut(),
            );
            let root = controller();
            if !root.is_null() {
                glass::finish_toggle(root, mask);
            }
        }
    }
}

unsafe extern "system" fn root_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CLOSE => {
            PostQuitMessage(0);
            0
        }
        WM_QUERYENDSESSION => 1,
        WM_ENDSESSION if wp != 0 => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe extern "system" fn button_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
        WM_LBUTTONUP => {
            signal(TOGGLE);
            0
        }
        WM_CONTEXTMENU => {
            signal(MENU);
            0
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            if !dc.is_null() {
                let mut rect = RECT::default();
                GetClientRect(hwnd, &mut rect);
                SetDCBrushColor(dc, 0x302821);
                FillRect(dc, &rect, GetStockObject(DC_BRUSH) as HBRUSH);
                let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
                let old_brush = SelectObject(dc, GetStockObject(DC_BRUSH));
                SetDCBrushColor(dc, 0xf4f4f4);
                let x = rect.right / 2;
                let y = rect.bottom / 2;
                let r = (rect.right / 8).max(1);
                for i in -1..=1 {
                    let d = x + i * (2 * r + 2);
                    Ellipse(dc, d - r, y - r, d + r, y + r);
                }
                SelectObject(dc, old_brush);
                SelectObject(dc, old_pen);
            }
            EndPaint(hwnd, &ps);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

struct TrayUi {
    instance: HINSTANCE,
    shell: HWND,
    button: HWND,
    applied: bool,
    needs_restore: bool,
}
impl TrayUi {
    fn sync(&mut self, state: &Path) -> Result<()> {
        unsafe {
            let shell = FindWindowW(w!("Shell_TrayWnd"), null());
            if shell != self.shell {
                self.detach();
                self.shell = shell;
                self.applied = false;
            }
            if shell.is_null() {
                TRAY_PARENT.set(null_mut());
                return Ok(());
            }
            let parent = tray(shell);
            TRAY_PARENT.set(parent);
            if parent.is_null() {
                return Ok(());
            }
            if !self.applied {
                let old = reg_auto()?;
                if !self.needs_restore {
                    save_state(state, old)?;
                    self.needs_restore = true;
                }
                if old != Some(1) {
                    set_auto(true)?;
                }
                self.applied = true;
            }
            let arrow = chevron(parent);
            let mut rect = RECT::default();
            if arrow.is_null()
                || IsWindowVisible(arrow) == 0
                || GetWindowRect(arrow, &mut rect) == 0
                || rect.right <= rect.left
                || rect.bottom <= rect.top
            {
                if !self.button.is_null() {
                    ShowWindow(self.button, SW_HIDE);
                }
                return Ok(());
            }
            if IsWindow(self.button) == 0 || GetParent(self.button) != parent {
                self.detach();
                self.button = CreateWindowExW(
                    WS_EX_NOACTIVATE | WS_EX_LAYERED,
                    BUTTON,
                    w!("TrayDots Rust"),
                    WS_CHILD,
                    0,
                    0,
                    0,
                    0,
                    parent,
                    null_mut(),
                    self.instance,
                    null(),
                );
                if self.button.is_null() {
                    return Err(win_error("Create tray button"));
                }
                // Give the dots their own surface so a repaint of the native arrow
                // cannot overwrite them even when our window is already on top.
                if SetLayeredWindowAttributes(self.button, 0, 255, LWA_ALPHA) == 0 {
                    let error = win_error("Set tray button opacity");
                    self.detach();
                    return Err(error);
                }
            }
            let mut prev = RECT::default();
            GetWindowRect(self.button, &mut prev);
            // Explorer can put its arrow above our button without moving either.
            if IsWindowVisible(self.button) == 0
                || GetTopWindow(parent) != self.button
                || [prev.left, prev.top, prev.right, prev.bottom]
                    != [rect.left, rect.top, rect.right, rect.bottom]
            {
                let mut pt = POINT {
                    x: rect.left,
                    y: rect.top,
                };
                ScreenToClient(parent, &mut pt);
                if SetWindowPos(
                    self.button,
                    HWND_TOP,
                    pt.x,
                    pt.y,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                ) == 0
                {
                    return Err(win_error("Position tray button"));
                }
            }
            Ok(())
        }
    }
    fn detach(&mut self) {
        if !self.button.is_null() {
            unsafe { DestroyWindow(self.button) };
            self.button = null_mut();
        }
    }
}
impl Drop for TrayUi {
    fn drop(&mut self) {
        self.detach();
    }
}

fn context_menu(root: HWND) -> Result<bool> {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return Err(win_error("CreatePopupMenu"));
        }
        let enabled = startup_enabled();
        AppendMenuW(menu, MF_STRING, 1, w!("Открыть / закрыть значки"));
        AppendMenuW(
            menu,
            MF_STRING | if enabled { MF_CHECKED } else { 0 },
            2,
            w!("Запускать вместе с Windows"),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, 3, w!("Выйти и вернуть значки"));
        let mut pt = POINT::default();
        GetCursorPos(&mut pt);
        SetForegroundWindow(root);
        let action = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            pt.x,
            pt.y,
            0,
            root,
            null(),
        );
        DestroyMenu(menu);
        PostMessageW(root, WM_NULL, 0, 0);
        match action {
            1 => toggle(),
            2 => startup(!enabled)?,
            3 => return Ok(true),
            _ => {}
        }
        Ok(false)
    }
}

fn platform_check() -> Result<()> {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // RtlGetVersion reports the real OS without application manifest shims.
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(version: *mut OSVERSIONINFOW) -> i32;
    }
    if unsafe { RtlGetVersion(&mut version) } != 0
        || version.dwMajorVersion != 10
        || !(19041..22000).contains(&version.dwBuildNumber)
    {
        Err("TrayDots Rust поддерживает классический трей Windows 10 (2004–22H2).".into())
    } else {
        Ok(())
    }
}

fn run() -> Result<()> {
    platform_check()?;
    let Some(_lock) = Lock::acquire(0)? else {
        signal(TOGGLE);
        return Ok(());
    };
    let state = journal()?;
    glass::recover_mask();
    restore(&state, None)?;
    let exe = io_result(env::current_exe())?;
    io_result(
        Command::new(exe)
            .args(["--watch", &process::id().to_string()])
            .spawn(),
    )?;
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
        let instance = GetModuleHandleW(null());
        glass::register(instance)?;
        let classes: [(*const u16, WNDPROC); 2] =
            [(ROOT, Some(root_proc)), (BUTTON, Some(button_proc))];
        for (name, proc) in classes {
            let class = WNDCLASSW {
                lpfnWndProc: proc,
                hInstance: instance,
                lpszClassName: name,
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(win_error("RegisterClassW"));
            }
        }
        let root = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            ROOT,
            w!("TrayDots Rust"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if root.is_null() {
            return Err(win_error("Create controller"));
        }
        if SetTimer(root, 1, 2000, None) == 0 {
            return Err(win_error("SetTimer"));
        }
        let reorder_hook = SetWinEventHook(
            EVENT_OBJECT_REORDER,
            EVENT_OBJECT_REORDER,
            null_mut(),
            Some(tray_reordered),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        RegisterHotKey(
            root,
            HOTKEY,
            MOD_ALT | MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
            u32::from(b'T'),
        );
        let mut ui = TrayUi {
            instance,
            shell: null_mut(),
            button: null_mut(),
            applied: false,
            needs_restore: false,
        };
        let mut failures = 0;
        let result = (|| {
            // Explorer may still be initializing at logon. Give the timer a few
            // attempts before treating the first COM or window error as fatal.
            if ui.sync(&state).is_err() {
                failures = 1;
            }
            let mut msg = MSG::default();
            loop {
                match GetMessageW(&mut msg, null_mut(), 0, 0) {
                    0 => break,
                    -1 => return Err(win_error("GetMessageW")),
                    _ => {}
                }
                if msg.hwnd == root {
                    match msg.message {
                        STOP => break,
                        TOGGLE | WM_HOTKEY => {
                            toggle();
                            continue;
                        }
                        MENU => {
                            if context_menu(root)? {
                                break;
                            }
                            continue;
                        }
                        WM_TIMER if msg.wParam == glass::TIMER => {
                            glass::sync(root);
                            continue;
                        }
                        SYNC_TRAY => {
                            let _ = ui.sync(&state);
                            continue;
                        }
                        WM_TIMER => {
                            match ui.sync(&state) {
                                Ok(()) => failures = 0,
                                Err(err) => {
                                    failures += 1;
                                    if failures >= 5 {
                                        return Err(err);
                                    }
                                }
                            }
                            continue;
                        }
                        _ => {}
                    }
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            Ok(())
        })();
        if !reorder_hook.is_null() {
            UnhookWinEvent(reorder_hook);
        }
        glass::close(root);
        ui.detach();
        KillTimer(root, 1);
        UnregisterHotKey(root, HOTKEY);
        DestroyWindow(root);
        let recovery = restore(&state, Some(process::id()));
        result.and(recovery)
    }
}

fn entry() -> Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("--watch") => recover_after(
            args.next()
                .and_then(|a| a.parse().ok())
                .ok_or("Missing PID")?,
        ),
        Some("--startup-on") => startup(true),
        Some("--startup-off") => startup(false),
        Some("--toggle") => {
            signal(TOGGLE);
            Ok(())
        }
        Some("--quit") => {
            signal(STOP);
            Ok(())
        }
        Some("--uninstall") => {
            startup(false)?;
            signal(STOP);
            Ok(())
        }
        Some("--restore") => {
            let _lock = Lock::acquire(0)?.ok_or("Сначала остановите TrayDots Rust.")?;
            glass::recover_mask();
            restore(&journal()?, None)
        }
        None | Some("--background") => run(),
        Some(arg) => Err(format!("Unknown argument: {arg}")),
    }
}

fn main() {
    if let Err(error) = entry() {
        let mode = env::args().nth(1);
        if matches!(mode.as_deref(), Some("--watch" | "--background")) {
            if let Ok(dir) = data_dir() {
                let _ = fs::create_dir_all(&dir);
                let _ = fs::write(dir.join("last-error.txt"), error);
            }
        } else {
            unsafe {
                MessageBoxW(
                    null_mut(),
                    wide(&error).as_ptr(),
                    w!("TrayDots Rust"),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
        process::exit(1);
    }
}
