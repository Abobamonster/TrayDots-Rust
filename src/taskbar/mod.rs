mod config;
mod menu;

use crate::{Handle, Result, controller};
use config::{Accent, Appearance, Mode, Settings, Tint, WindowDetails};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    mem::size_of,
    ptr::{null, null_mut},
};
use windows_sys::{
    Win32::{
        Foundation::*,
        Graphics::{
            Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
            Gdi::*,
        },
        System::{Com::*, LibraryLoader::*, Power::*, Threading::*},
        UI::{Accessibility::*, Shell::AppVisibility, WindowsAndMessaging::*},
    },
    core::{BOOL, GUID, w},
};

pub const TIMER: usize = 3;
const ACCENT_MARK: *const u16 = w!("TrayDotsRust.TaskbarAccent.1");
const PEEK_MARK: *const u16 = w!("TrayDotsRust.TaskbarPeek.1");

thread_local! {
    static TIMER_PENDING: Cell<bool> = const { Cell::new(false) };
    static FORCE_REFRESH: Cell<bool> = const { Cell::new(false) };
}

#[repr(C)]
#[derive(Clone, Copy, Default, Eq, PartialEq)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    color: u32,
    animation: u32,
}

impl AccentPolicy {
    fn from_appearance(a: Appearance) -> Option<Self> {
        let [r, g, b, alpha] = a.color.0;
        let (state, flags, alpha) = match a.accent {
            Accent::Normal => return None,
            Accent::Opaque => (1, 2, 255),
            Accent::Clear => (2, 2, alpha),
            Accent::Blur => (3, 2, alpha),
            Accent::Acrylic => (4, 0, alpha.max(1)),
        };
        Some(Self {
            state,
            flags,
            color: u32::from_le_bytes([r, g, b, alpha]),
            animation: 0,
        })
    }
}

#[repr(C)]
struct CompositionData {
    attribute: u32,
    data: *mut c_void,
    length: usize,
}

type Composition = unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL;

fn composition_api() -> Option<Composition> {
    unsafe {
        let user32 = GetModuleHandleW(w!("user32.dll"));
        let set = GetProcAddress(user32, c"SetWindowCompositionAttribute".as_ptr().cast())?;
        Some(std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            Composition,
        >(set))
    }
}

struct LauncherVisibility(*mut c_void);

impl LauncherVisibility {
    fn connect() -> Option<Self> {
        const IID: GUID = GUID::from_u128(0x2246ea2d_caea_4444_a3c4_6de827e44313);
        unsafe {
            if CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) < 0 {
                return None;
            }
            let mut object = null_mut();
            if CoCreateInstance(
                &AppVisibility,
                null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID,
                &mut object,
            ) < 0
                || object.is_null()
            {
                CoUninitialize();
                return None;
            }
            Some(Self(object))
        }
    }

    fn visible(&self) -> Option<bool> {
        unsafe {
            let table = *self.0.cast::<*const *const c_void>();
            let visible: unsafe extern "system" fn(*mut c_void, *mut BOOL) -> i32 =
                std::mem::transmute(*table.add(4)); // IAppVisibility::IsLauncherVisible
            let mut value = 0;
            (visible(self.0, &mut value) >= 0).then_some(value != 0)
        }
    }
}

impl Drop for LauncherVisibility {
    fn drop(&mut self) {
        unsafe {
            let table = *self.0.cast::<*const *const c_void>();
            let release: unsafe extern "system" fn(*mut c_void) -> u32 =
                std::mem::transmute(*table.add(2));
            release(self.0);
            CoUninitialize();
        }
    }
}

fn class(hwnd: HWND, buf: &mut [u16]) -> &[u16] {
    let len = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    &buf[..len.max(0) as usize]
}

fn is_class(actual: &[u16], expected: &str) -> bool {
    actual.iter().copied().eq(expected.encode_utf16())
}

fn process_name(hwnd: HWND) -> String {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if proc.is_null() {
            return String::new();
        }
        let proc = Handle(proc);
        let mut path = [0u16; 1024];
        let mut len = path.len() as u32;
        if QueryFullProcessImageNameW(proc.0, 0, path.as_mut_ptr(), &mut len) == 0 {
            return String::new();
        }
        let path = &path[..len as usize];
        String::from_utf16_lossy(path.rsplit(|c| *c == b'\\' as u16).next().unwrap_or(path))
    }
}

fn details(hwnd: HWND) -> WindowDetails {
    let mut class_buffer = [0; 256];
    let mut title = [0; 1024];
    let len = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
    WindowDetails {
        class: String::from_utf16_lossy(class(hwnd, &mut class_buffer)),
        title: String::from_utf16_lossy(&title[..len.max(0) as usize]),
        process: process_name(hwnd),
    }
}

fn monitor(hwnd: HWND) -> HMONITOR {
    unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL) }
}

fn foreground_mode(foreground: HWND, launcher: Option<&LauncherVisibility>) -> Option<Mode> {
    let mut buf = [0; 128];
    let class = class(foreground, &mut buf);
    if !is_class(class, "Windows.UI.Core.CoreWindow") {
        return None;
    }
    let name = process_name(foreground);
    if name.eq_ignore_ascii_case("explorer.exe") {
        return Some(Mode::TaskView);
    }
    if name.eq_ignore_ascii_case("StartMenuExperienceHost.exe")
        || launcher.and_then(LauncherVisibility::visible) == Some(true)
    {
        return Some(Mode::Start);
    }
    if name.eq_ignore_ascii_case("SearchApp.exe") || name.eq_ignore_ascii_case("SearchUI.exe") {
        return Some(Mode::Search);
    }
    None
}

#[derive(Default)]
struct Bar {
    hwnd: HWND,
    monitor: HMONITOR,
    visible: bool,
    maximized: HWND,
}

fn bars() -> Vec<Bar> {
    let mut bars = Vec::with_capacity(3);
    let mut add = |hwnd| {
        bars.push(Bar {
            hwnd,
            monitor: monitor(hwnd),
            ..Bar::default()
        });
    };
    unsafe {
        let primary = FindWindowW(w!("Shell_TrayWnd"), null());
        if !primary.is_null() {
            add(primary);
        }
        let mut secondary = null_mut();
        loop {
            secondary = FindWindowExW(null_mut(), secondary, w!("Shell_SecondaryTrayWnd"), null());
            if secondary.is_null() {
                break;
            }
            add(secondary);
        }
    }
    bars
}

struct Windows<'a> {
    bars: &'a mut [Bar],
    settings: &'a Settings,
    pid: u32,
}

unsafe extern "system" fn find_windows(hwnd: HWND, context: LPARAM) -> BOOL {
    if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
        return 1;
    }
    let scan = &mut *(context as *mut Windows<'_>);
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    if pid == scan.pid
        || style & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) != 0
        || (!GetWindow(hwnd, GW_OWNER).is_null() && style & WS_EX_APPWINDOW == 0)
    {
        return 1;
    }
    let mut buf = [0; 128];
    let class = class(hwnd, &mut buf);
    if [
        "Progman",
        "WorkerW",
        "Shell_TrayWnd",
        "Shell_SecondaryTrayWnd",
        "Windows.UI.Core.CoreWindow",
        "NotifyIconOverflowWindow",
        "MultitaskingViewFrame",
    ]
    .iter()
    .any(|name| is_class(class, name))
    {
        return 1;
    }
    let mut cloaked = 0u32;
    DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED as u32,
        std::ptr::from_mut(&mut cloaked).cast(),
        4,
    );
    if cloaked != 0
        || (!scan.settings.ignored_windows.is_empty()
            && scan.settings.ignored_windows.matches(&details(hwnd)))
    {
        return 1;
    }
    if let Some(bar) = scan
        .bars
        .iter_mut()
        .find(|bar| bar.monitor == monitor(hwnd))
    {
        bar.visible = true;
        if bar.maximized.is_null() && IsZoomed(hwnd) != 0 {
            bar.maximized = hwnd;
        }
    }
    1
}

unsafe extern "system" fn find_peek(hwnd: HWND, context: LPARAM) -> BOOL {
    let mut buf = [0; 64];
    if is_class(class(hwnd, &mut buf), "TrayShowDesktopButtonWClass") {
        *(context as *mut HWND) = hwnd;
        return 0;
    }
    1
}

fn peek(bar: HWND) -> HWND {
    let mut peek = null_mut();
    unsafe {
        EnumChildWindows(
            bar,
            Some(find_peek),
            std::ptr::from_mut(&mut peek) as LPARAM,
        )
    };
    peek
}

fn set_peek(bar: HWND, show: bool) {
    let window = peek(bar);
    if window.is_null() {
        return;
    }
    unsafe {
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
        let marked = !GetPropW(window, PEEK_MARK).is_null();
        let layered = style & WS_EX_LAYERED as isize != 0;
        if show && marked {
            SetWindowLongPtrW(window, GWL_EXSTYLE, style & !(WS_EX_LAYERED as isize));
            RemovePropW(window, PEEK_MARK);
            InvalidateRect(window, null(), 1);
        } else if !show
            && !layered
            && (marked || SetPropW(window, PEEK_MARK, 1usize as HANDLE) != 0)
        {
            SetWindowLongPtrW(window, GWL_EXSTYLE, style | WS_EX_LAYERED as isize);
            if SetLayeredWindowAttributes(window, 0, 1, LWA_ALPHA) == 0 {
                SetWindowLongPtrW(window, GWL_EXSTYLE, style);
                RemovePropW(window, PEEK_MARK);
            }
        }
    }
}

fn reset_accent(hwnd: HWND) -> bool {
    unsafe {
        if GetPropW(hwnd, ACCENT_MARK).is_null() {
            return true;
        }
        if SendMessageTimeoutW(
            hwnd,
            WM_DWMCOMPOSITIONCHANGED,
            0,
            0,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            200,
            null_mut(),
        ) != 0
        {
            RemovePropW(hwnd, ACCENT_MARK);
            true
        } else {
            false
        }
    }
}

fn set_accent(set: Composition, hwnd: HWND, mut policy: AccentPolicy) -> bool {
    let mut data = CompositionData {
        attribute: 19, // WCA_ACCENT_POLICY
        data: std::ptr::from_mut(&mut policy).cast(),
        length: size_of::<AccentPolicy>(),
    };
    unsafe {
        let marked = !GetPropW(hwnd, ACCENT_MARK).is_null();
        if !marked && SetPropW(hwnd, ACCENT_MARK, 1usize as HANDLE) == 0 {
            return false;
        }
        if set(hwnd, &mut data) != 0 {
            return true;
        }
        if !marked {
            RemovePropW(hwnd, ACCENT_MARK);
        }
        false
    }
}

pub fn recover() {
    for bar in bars() {
        reset_accent(bar.hwnd);
        set_peek(bar.hwnd, true);
    }
}

fn battery_saver() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status) != 0 && status.SystemStatusFlag == 1 }
}

unsafe extern "system" fn window_event(
    _: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if object == OBJID_WINDOW
        && child == CHILDID_SELF as i32
        && !hwnd.is_null()
        && (event == EVENT_SYSTEM_FOREGROUND
            || (event == EVENT_OBJECT_LOCATIONCHANGE && hwnd == GetForegroundWindow())
            || ((EVENT_OBJECT_SHOW..=EVENT_OBJECT_HIDE).contains(&event)
                && GetAncestor(hwnd, GA_ROOT) == hwnd)
            || (EVENT_SYSTEM_MINIMIZESTART..=EVENT_SYSTEM_MINIMIZEEND).contains(&event))
    {
        schedule(controller(), event == EVENT_SYSTEM_FOREGROUND);
    }
}

pub fn schedule(root: HWND, force: bool) {
    if force {
        FORCE_REFRESH.set(true);
    }
    if !root.is_null() && !TIMER_PENDING.get() && unsafe { SetTimer(root, TIMER, 100, None) } != 0 {
        TIMER_PENDING.set(true);
    }
}

pub fn consume_timer(root: HWND) -> bool {
    unsafe { KillTimer(root, TIMER) };
    TIMER_PENDING.set(false);
    FORCE_REFRESH.replace(false)
}

pub struct Taskbar {
    config: Settings,
    api: Option<Composition>,
    applied: RefCell<Vec<(HWND, Option<AccentPolicy>)>>,
    launcher: Option<LauncherVisibility>,
    hooks: Vec<HWINEVENTHOOK>,
    tracks_windows: bool,
    load_error: Option<String>,
}

impl Taskbar {
    pub fn new() -> Self {
        let (config, load_error) = match Settings::load() {
            Ok(config) => (config, None),
            Err(error) => {
                let mut config = Settings::default();
                config.enabled = false;
                (config, Some(error))
            }
        };
        let mut taskbar = Self {
            config,
            api: composition_api(),
            applied: RefCell::new(Vec::new()),
            launcher: None,
            hooks: Vec::new(),
            tracks_windows: false,
            load_error,
        };
        taskbar.update_hooks();
        taskbar
    }

    fn update_hooks(&mut self) {
        if !self.config.enabled {
            self.stop_hooks();
            return;
        }
        if self.launcher.is_none() && self.config.profile(Mode::Start).enabled {
            self.launcher = LauncherVisibility::connect();
        }
        let tracks_windows = self.config.profile(Mode::Visible).enabled
            || self.config.profile(Mode::Maximized).enabled;
        if tracks_windows != self.tracks_windows {
            self.unhook();
            self.tracks_windows = tracks_windows;
        }
        if self.hooks.is_empty() {
            for (min, max) in [
                (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
                (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
                (EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE),
                (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
            ] {
                if !tracks_windows
                    && (min == EVENT_SYSTEM_MINIMIZESTART || min == EVENT_OBJECT_LOCATIONCHANGE)
                {
                    continue;
                }
                let hook = unsafe {
                    SetWinEventHook(
                        min,
                        max,
                        null_mut(),
                        Some(window_event),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                    )
                };
                if !hook.is_null() {
                    self.hooks.push(hook);
                }
            }
        }
    }

    fn unhook(&mut self) {
        for hook in self.hooks.drain(..) {
            unsafe { UnhookWinEvent(hook) };
        }
    }

    fn stop_hooks(&mut self) {
        self.unhook();
        self.launcher = None;
    }

    pub fn refresh(&self, force: bool) {
        if self.config.enabled {
            self.apply(None, force);
        }
    }

    fn apply(&self, preview: Option<Appearance>, force: bool) {
        let Some(api) = self.api else { return };
        let mut bars = bars();
        if bars.is_empty() {
            return;
        }
        let foreground = unsafe { GetForegroundWindow() };
        let foreground_monitor = monitor(foreground);
        let shell_mode = if [Mode::Start, Mode::Search, Mode::TaskView]
            .into_iter()
            .any(|mode| self.config.profile(mode).enabled)
        {
            foreground_mode(foreground, self.launcher.as_ref())
        } else {
            None
        };
        let saving = self.config.profile(Mode::Battery).enabled && battery_saver();
        if preview.is_none()
            && !saving
            && (self.config.profile(Mode::Visible).enabled
                || self.config.profile(Mode::Maximized).enabled)
        {
            let mut scan = Windows {
                bars: &mut bars,
                settings: &self.config,
                pid: std::process::id(),
            };
            unsafe { EnumWindows(Some(find_windows), std::ptr::from_mut(&mut scan) as LPARAM) };
        }
        let mut applied = self.applied.borrow_mut();
        applied.retain(|(hwnd, _)| unsafe { IsWindow(*hwnd) != 0 });
        for bar in &bars {
            let a = preview.unwrap_or_else(|| {
                self.appearance(bar, foreground, foreground_monitor, shell_mode, saving)
            });
            let hwnd = bar.hwnd;
            let policy = AccentPolicy::from_appearance(a);
            let prev = applied.iter().position(|(window, _)| *window == hwnd);
            if (force || prev.is_none_or(|idx| applied[idx].1 != policy))
                && policy.map_or_else(
                    || reset_accent(hwnd),
                    |policy| set_accent(api, hwnd, policy),
                )
            {
                match prev {
                    Some(index) => applied[index].1 = policy,
                    None => applied.push((hwnd, policy)),
                }
            }
            set_peek(hwnd, a.show_peek);
        }
    }

    fn appearance(
        &self,
        bar: &Bar,
        foreground: HWND,
        foreground_monitor: HMONITOR,
        shell_mode: Option<Mode>,
        saving: bool,
    ) -> Appearance {
        if saving {
            return self.config.profile(Mode::Battery).appearance;
        }
        if let Some(mode) =
            shell_mode.filter(|mode| *mode == Mode::TaskView || bar.monitor == foreground_monitor)
        {
            let profile = self.config.profile(mode);
            if profile.enabled {
                return profile.appearance;
            }
        }
        let maximized = self.config.profile(Mode::Maximized);
        if maximized.enabled && !bar.maximized.is_null() {
            return if maximized.has_rules() {
                maximized.resolve(&details(bar.maximized), bar.maximized == foreground)
            } else {
                maximized.appearance
            };
        }
        let visible = self.config.profile(Mode::Visible);
        if visible.enabled && bar.visible {
            if visible.has_rules() && !foreground.is_null() && foreground_monitor == bar.monitor {
                return visible.resolve(&details(foreground), true);
            }
            return visible.appearance;
        }
        self.config.profile(Mode::Desktop).appearance
    }

    pub fn append_menu(&self, parent: HMENU) {
        menu::append(parent, &self.config);
    }

    pub fn command(&mut self, root: HWND, command: u32) -> Result<()> {
        menu::command(self, root, command)
    }

    pub fn reload(&mut self) -> Result<()> {
        self.config = Settings::load()?;
        self.load_error = None;
        self.update_hooks();
        self.apply_settings();
        Ok(())
    }

    fn apply_settings(&mut self) {
        if self.config.enabled {
            self.refresh(true);
        } else {
            recover();
            self.applied.get_mut().clear();
        }
    }
}

impl Drop for Taskbar {
    fn drop(&mut self) {
        self.stop_hooks();
        recover();
    }
}
