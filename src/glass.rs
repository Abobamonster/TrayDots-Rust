//! Decorate the native overflow popup; Explorer still receives mouse/keyboard input.
use super::*;
use std::{cell::RefCell, mem::zeroed};
use windows_sys::Win32::{
    System::{Diagnostics::Debug::ReadProcessMemory, Memory::*},
    UI::Controls::*,
};

pub const TIMER: usize = 2;
const GLASS: *const u16 = w!("TrayDotsRust.OverflowGlass.1");
const MASK: *const u16 = w!("TrayDotsRust.OverflowMask.1");
const SCRATCH: usize = 64;
const ICON_LIMIT: usize = 128;
const SAMPLE_SIZE: i32 = 24;

thread_local! {
    static ACTIVE: RefCell<Option<Flyout>> = const { RefCell::new(None) };
}

fn overflow() -> HWND {
    unsafe { FindWindowW(w!("NotifyIconOverflowWindow"), null()) }
}

fn toolbar(popup: HWND) -> HWND {
    unsafe { FindWindowExW(popup, null_mut(), w!("ToolbarWindow32"), null()) }
}

fn send(window: HWND, msg: u32, wp: usize, lp: isize) -> Option<usize> {
    let mut result = 0;
    (unsafe {
        SendMessageTimeoutW(
            window,
            msg,
            wp,
            lp,
            SMTO_BLOCK | SMTO_ABORTIFHUNG,
            150,
            &mut result,
        )
    } != 0)
        .then_some(result)
}

struct Remote {
    process: Handle,
    scratch: *mut c_void,
}
impl Remote {
    fn open(bar: HWND) -> Option<Self> {
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(bar, &mut pid);
            let process = OpenProcess(PROCESS_VM_OPERATION | PROCESS_VM_READ, 0, pid);
            if process.is_null() {
                return None;
            }
            let process = Handle(process);
            let scratch = VirtualAllocEx(
                process.0,
                null(),
                SCRATCH,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_READWRITE,
            );
            (!scratch.is_null()).then_some(Self { process, scratch })
        }
    }
    fn read<T>(&self, address: *const c_void) -> Option<T> {
        unsafe {
            let mut value: T = zeroed();
            let mut read = 0;
            (ReadProcessMemory(
                self.process.0,
                address,
                (&mut value as *mut T).cast(),
                size_of::<T>(),
                &mut read,
            ) != 0
                && read == size_of::<T>())
            .then_some(value)
        }
    }
    fn query<T>(&mut self, bar: HWND, msg: u32, index: usize) -> Option<T> {
        debug_assert!(size_of::<T>() <= SCRATCH);
        match send(bar, msg, index, self.scratch as isize) {
            Some(0) => None,
            Some(_) => self.read(self.scratch),
            None => {
                // A timed-out receiver may still be writing the buffer. Do
                // not free that single page out from under Explorer.
                self.scratch = null_mut();
                None
            }
        }
    }
}
impl Drop for Remote {
    fn drop(&mut self) {
        if !self.scratch.is_null() {
            unsafe { VirtualFreeEx(self.process.0, self.scratch, 0, MEM_RELEASE) };
        }
    }
}

// Windows 10 x64 Explorer's toolbar item points at a shell tray record.
#[repr(C)]
struct TrayData {
    owner: HWND,
    id: u32,
    callback: u32,
    reserved: [u32; 2],
    icon: HICON,
}

struct Icon(HICON);
impl Drop for Icon {
    fn drop(&mut self) {
        unsafe { DestroyIcon(self.0) };
    }
}

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    pixels: *mut u32,
    len: usize,
}
impl Surface {
    fn new(width: i32, height: i32) -> Option<Self> {
        let len = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?;
        if len == 0 || len > 2_000_000 {
            return None;
        }
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            if dc.is_null() {
                return None;
            }
            let mut bits = null_mut();
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    ..Default::default()
                },
                ..Default::default()
            };
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                if !bitmap.is_null() {
                    DeleteObject(bitmap);
                }
                DeleteDC(dc);
                return None;
            }
            let previous = SelectObject(dc, bitmap);
            Some(Self {
                dc,
                bitmap,
                previous,
                pixels: bits.cast(),
                len,
            })
        }
    }
    fn pixels(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.pixels, self.len) }
    }

    fn group(&mut self, icon: HICON) -> u8 {
        const SENTINEL: u32 = 0x0001fe13;
        unsafe {
            self.pixels().fill(SENTINEL);
            if DrawIconEx(
                self.dc,
                0,
                0,
                icon,
                SAMPLE_SIZE,
                SAMPLE_SIZE,
                0,
                null_mut(),
                DI_NORMAL,
            ) == 0
            {
                return 9;
            }
            GdiFlush();
            let mut votes = [0u32; 9];
            for &pixel in self.pixels().iter() {
                if pixel == SENTINEL {
                    continue;
                }
                let a = (pixel >> 24) as u8;
                // Some old mask-based icons return RGB without an alpha channel.
                if (1..96).contains(&a) {
                    continue;
                }
                let r = (pixel >> 16) as u8;
                let g = (pixel >> 8) as u8;
                let b = pixel as u8;
                votes[color_group(r, g, b) as usize] += if a == 0 { 255 } else { a as u32 };
            }
            votes
                .into_iter()
                .enumerate()
                .filter(|(_, n)| *n > 0)
                .max_by_key(|&(group, count)| (count, std::cmp::Reverse(group)))
                .map_or(9, |(group, _)| group as u8)
        }
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

fn color_group(r: u8, g: u8, b: u8) -> u8 {
    let (r, g, b) = (r as f32, g as f32, b as f32);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let spread = max - min;
    if max < 55.0 {
        return 8;
    }
    if spread < 26.0 || spread / max < 0.20 {
        return if max >= 135.0 { 0 } else { 8 };
    }
    let sector = if max == r {
        (g - b) / spread
    } else if max == g {
        2.0 + (b - r) / spread
    } else {
        4.0 + (r - g) / spread
    };
    let h = (sector * 60.0).rem_euclid(360.0);
    if !(15.0..345.0).contains(&h) {
        1
    } else if h < 46.0 {
        2
    } else if h < 72.0 {
        3
    } else if h < 163.0 {
        4
    } else if h < 204.0 {
        5
    } else if h < 255.0 {
        6
    } else {
        7
    }
}

struct Slot {
    button: TBBUTTON,
    icon: Icon,
    group: u8,
}

fn snapshot(bar: HWND, remote: &mut Remote, count: usize) -> Option<Vec<Slot>> {
    let mut sample = Surface::new(SAMPLE_SIZE, SAMPLE_SIZE)?;
    let mut slots = Vec::with_capacity(count);
    for i in 0..count {
        let button = remote.query::<TBBUTTON>(bar, TB_GETBUTTON, i)?;
        let record: TrayData = remote.read(button.dwData as *const c_void)?;
        let icon = unsafe { CopyIcon(record.icon) };
        if icon.is_null() {
            return None;
        }
        let icon = Icon(icon);
        let group = sample.group(icon.0);
        slots.push(Slot {
            button,
            icon,
            group,
        });
    }
    Some(slots)
}

fn command_index(bar: HWND, id: i32) -> Option<usize> {
    let i = send(bar, TB_COMMANDTOINDEX, id as usize, 0)?;
    (i <= ICON_LIMIT).then_some(i)
}

fn reorder(bar: HWND, desired: &[(i32, u8)]) -> Option<()> {
    // Wrap belongs to the position in the grid, while all other button flags
    // belong to the original command (including its notification identity).
    if send(bar, TB_BUTTONCOUNT, 0, 0)? != desired.len() {
        return None;
    }
    for (index, &(id, _)) in desired.iter().enumerate() {
        let old = command_index(bar, id)?;
        if old != index && send(bar, TB_MOVEBUTTON, old, index as isize)? == 0 {
            return None;
        }
    }
    for &(id, state) in desired {
        if send(bar, TB_GETSTATE, id as usize, 0)? != state as usize
            && send(bar, TB_SETSTATE, id as usize, state as isize)? == 0
        {
            return None;
        }
    }
    Some(())
}

struct Order {
    bar: HWND,
    original: Vec<(i32, u8)>,
}
impl Drop for Order {
    fn drop(&mut self) {
        if unsafe { IsWindow(self.bar) } != 0 {
            let _ = reorder(self.bar, &self.original);
        }
    }
}

// Keep the native popup as the hit target, with just 1/255 of its original
// opacity. Only set WS_EX_LAYERED if another program did not already use it.
fn mask(popup: HWND) -> bool {
    unsafe {
        let style = GetWindowLongPtrW(popup, GWL_EXSTYLE);
        if style & WS_EX_LAYERED as isize != 0
            || !GetPropW(popup, MASK).is_null()
            || SetPropW(popup, MASK, 1usize as HANDLE) == 0
        {
            return false;
        }
        SetWindowLongPtrW(popup, GWL_EXSTYLE, style | WS_EX_LAYERED as isize);
        if SetLayeredWindowAttributes(popup, 0, 1, LWA_ALPHA) == 0 {
            unmask(popup);
            return false;
        }
        true
    }
}

fn unmask(popup: HWND) {
    unsafe {
        if !GetPropW(popup, MASK).is_null() {
            let style = GetWindowLongPtrW(popup, GWL_EXSTYLE);
            SetWindowLongPtrW(popup, GWL_EXSTYLE, style & !(WS_EX_LAYERED as isize));
            RemovePropW(popup, MASK);
            RedrawWindow(
                popup,
                null(),
                null_mut(),
                RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
            );
        }
    }
}

pub fn recover_mask() {
    let popup = overflow();
    if !popup.is_null() {
        unmask(popup);
    }
}

pub struct PopupMask(HWND);
impl Drop for PopupMask {
    fn drop(&mut self) {
        unmask(self.0);
    }
}

pub fn mask_before_open() -> Option<PopupMask> {
    let popup = overflow();
    // Explorer normally creates its overflow window before the first click.
    // Mask it while hidden, so the native background never gets a visible frame.
    if popup.is_null() || unsafe { IsWindowVisible(popup) } != 0 || !mask(popup) {
        return None;
    }
    Some(PopupMask(popup))
}

struct Tile {
    icon: Icon,
    rect: RECT,
}
struct Flyout {
    hwnd: HWND,
    popup: HWND,
    order: Order,
    tiles: Vec<Tile>,
    bounds: RECT,
    hover: Option<usize>,
    dark: Surface,
    light: Surface,
    _mask: PopupMask,
}
impl Drop for Flyout {
    fn drop(&mut self) {
        if !self.hwnd.is_null() {
            unsafe { DestroyWindow(self.hwnd) };
        }
    }
}

impl Flyout {
    fn render(&mut self) -> bool {
        unsafe {
            let dpi = GetDpiForWindow(self.popup).max(96) as i32;
            let size = 16 * dpi / 96;
            self.dark.pixels().fill(0);
            self.light.pixels().fill(0x00ffffff);
            for dc in [self.dark.dc, self.light.dc] {
                for (index, tile) in self.tiles.iter().enumerate() {
                    let r = &tile.rect;
                    if self.hover == Some(index) {
                        let mut hover = *r;
                        InflateRect(&mut hover, -2, -2);
                        SetDCBrushColor(dc, 0x787878);
                        FrameRect(dc, &hover, GetStockObject(DC_BRUSH) as HBRUSH);
                    }
                    if DrawIconEx(
                        dc,
                        (r.left + r.right - size) / 2,
                        (r.top + r.bottom - size) / 2,
                        tile.icon.0,
                        size,
                        size,
                        0,
                        null_mut(),
                        DI_NORMAL,
                    ) == 0
                    {
                        GdiFlush();
                        return false;
                    }
                }
            }
            GdiFlush();
            // Recover alpha from the difference between the two backgrounds.
            // This also preserves black pixels in older mask-based icons.
            for (black, white) in self.dark.pixels().iter_mut().zip(self.light.pixels()) {
                let black_rgb = *black & 0x00ffffff;
                let alpha = 255
                    - (0..=16)
                        .step_by(8)
                        .map(|shift| {
                            ((*white >> shift) & 255).saturating_sub((black_rgb >> shift) & 255)
                        })
                        .max()
                        .unwrap_or(0);
                *black = (alpha << 24) | black_rgb;
            }
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                self.hwnd,
                null_mut(),
                &POINT {
                    x: self.bounds.left,
                    y: self.bounds.top,
                },
                &SIZE {
                    cx: self.bounds.right - self.bounds.left,
                    cy: self.bounds.bottom - self.bounds.top,
                },
                self.dark.dc,
                &POINT::default(),
                0,
                &blend,
                ULW_ALPHA,
            ) != 0
        }
    }
}

fn create(mask: PopupMask) -> Option<Flyout> {
    let popup = mask.0;
    unsafe {
        let bar = toolbar(popup);
        if bar.is_null() {
            return None;
        }
        let count = send(bar, TB_BUTTONCOUNT, 0, 0)?;
        if !(1..=ICON_LIMIT).contains(&count) {
            return None;
        }
        let mut bounds = RECT::default();
        if GetWindowRect(popup, &mut bounds) == 0 {
            return None;
        }
        let mut remote = Remote::open(bar)?;
        let mut slots = snapshot(bar, &mut remote, count)?;
        let original: Vec<_> = slots
            .iter()
            .map(|s| (s.button.idCommand, s.button.fsState))
            .collect();
        let order = Order { bar, original };
        slots.sort_by_key(|s| s.group);
        let wrap = TBSTATE_WRAP as u8;
        let desired: Vec<_> = slots
            .iter()
            .zip(&order.original)
            .map(|(s, orig)| {
                (
                    s.button.idCommand,
                    (s.button.fsState & !wrap) | (orig.1 & wrap),
                )
            })
            .collect();
        reorder(bar, &desired)?;
        let mut origin = POINT::default();
        ClientToScreen(bar, &mut origin);
        let mut tiles = Vec::with_capacity(count);
        for (index, slot) in slots.into_iter().enumerate() {
            let mut rect: RECT = remote.query(bar, TB_GETITEMRECT, index)?;
            OffsetRect(&mut rect, origin.x - bounds.left, origin.y - bounds.top);
            if rect.left < 0
                || rect.top < 0
                || rect.right > bounds.right - bounds.left
                || rect.bottom > bounds.bottom - bounds.top
            {
                return None;
            }
            tiles.push(Tile {
                icon: slot.icon,
                rect,
            });
        }
        let width = bounds.right - bounds.left;
        let height = bounds.bottom - bounds.top;
        let dark = Surface::new(width, height)?;
        let light = Surface::new(width, height)?;
        let hwnd = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT,
            GLASS,
            w!("TrayDots Icons"),
            WS_POPUP,
            bounds.left,
            bounds.top,
            width,
            height,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if hwnd.is_null() {
            return None;
        }
        let mut flyout = Flyout {
            hwnd,
            popup,
            order,
            tiles,
            bounds,
            hover: None,
            dark,
            light,
            _mask: mask,
        };
        if !flyout.render()
            || SetWindowPos(
                hwnd,
                popup,
                bounds.left,
                bounds.top,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            ) == 0
        {
            return None;
        }
        Some(flyout)
    }
}

pub fn finish_toggle(root: HWND, mask: Option<PopupMask>) {
    sync(root);
    let Some(mask) = mask else { return };
    if unsafe { IsWindowVisible(mask.0) } == 0 {
        return;
    }
    ACTIVE.with(|cell| {
        let mut active = cell.borrow_mut();
        *active = create(mask);
        if active.is_some() && unsafe { SetTimer(root, TIMER, 80, None) } == 0 {
            drop(active.take());
        }
    });
}

pub fn sync(root: HWND) {
    let popup = overflow();
    ACTIVE.with(|cell| {
        let mut active = cell.borrow_mut();
        unsafe {
            if popup.is_null() || IsWindowVisible(popup) == 0 {
                drop(active.take());
                KillTimer(root, TIMER);
                return;
            }
            if let Some(flyout) = active.as_mut() {
                let mut bounds = RECT::default();
                if popup != flyout.popup
                    || GetWindowRect(popup, &mut bounds) == 0
                    || send(flyout.order.bar, TB_BUTTONCOUNT, 0, 0) != Some(flyout.tiles.len())
                {
                    drop(active.take());
                    KillTimer(root, TIMER);
                    return;
                }
                let old = &flyout.bounds;
                if (bounds.left, bounds.top, bounds.right, bounds.bottom)
                    != (old.left, old.top, old.right, old.bottom)
                {
                    drop(active.take());
                    KillTimer(root, TIMER);
                    return;
                }
                let mut cursor = POINT::default();
                GetCursorPos(&mut cursor);
                let x = cursor.x - bounds.left;
                let y = cursor.y - bounds.top;
                let hovered = flyout.tiles.iter().position(|tile| {
                    let r = &tile.rect;
                    x >= r.left && x < r.right && y >= r.top && y < r.bottom
                });
                if hovered != flyout.hover {
                    flyout.hover = hovered;
                    if !flyout.render() {
                        drop(active.take());
                        KillTimer(root, TIMER);
                    }
                }
            }
        }
    });
}

pub fn close(root: HWND) {
    unsafe { KillTimer(root, TIMER) };
    ACTIVE.with(|cell| {
        drop(cell.borrow_mut().take());
    });
}

pub fn register(instance: HINSTANCE) -> Result<()> {
    let class = WNDCLASSW {
        lpfnWndProc: Some(DefWindowProcW),
        hInstance: instance,
        lpszClassName: GLASS,
        ..Default::default()
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        Err(win_error("Register transparent icon window"))
    } else {
        Ok(())
    }
}
