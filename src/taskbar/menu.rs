use super::{Accent, Appearance, Mode, Settings, Taskbar, Tint};
use crate::{Result, wide, win_error};
use std::{cell::Cell, mem::size_of, ptr::null};
use windows_sys::{
    Win32::{
        Foundation::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{Dialogs::*, *},
            Input::KeyboardAndMouse::EnableWindow,
            Shell::ShellExecuteW,
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

const ENABLE: u32 = 100;
const EDIT: u32 = 101;
const RELOAD: u32 = 102;
const PROFILES: u32 = 110;
const STRIDE: u32 = 10;
const PROFILE_ENABLED: u32 = 5;
const PROFILE_PEEK: u32 = 6;
const PROFILE_COLOR: u32 = 7;

unsafe fn item(menu: HMENU, id: u32, label: &str, checked: bool, enabled: bool) {
    let flags =
        MF_STRING | if checked { MF_CHECKED } else { 0 } | if enabled { 0 } else { MF_GRAYED };
    AppendMenuW(menu, flags, id as usize, wide(label).as_ptr());
}

unsafe fn attach(parent: HMENU, menu: HMENU, label: &str) {
    if AppendMenuW(parent, MF_POPUP, menu as usize, wide(label).as_ptr()) == 0 {
        DestroyMenu(menu);
    }
}

pub fn append(parent: HMENU, config: &Settings) {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        item(
            menu,
            ENABLE,
            "Управлять фоном панели задач",
            config.enabled,
            true,
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        for (index, (mode, label)) in Mode::OPTIONS.into_iter().enumerate() {
            let sub = CreatePopupMenu();
            if sub.is_null() {
                continue;
            }
            let id = PROFILES + index as u32 * STRIDE;
            let profile = config.profile(mode);
            if mode != Mode::Desktop {
                item(
                    sub,
                    id + PROFILE_ENABLED,
                    "Использовать этот профиль",
                    profile.enabled,
                    true,
                );
                AppendMenuW(sub, MF_SEPARATOR, 0, null());
            }
            for (index, (accent, label)) in Accent::OPTIONS.into_iter().enumerate() {
                item(
                    sub,
                    id + index as u32,
                    label,
                    profile.appearance.accent == accent,
                    true,
                );
            }
            AppendMenuW(sub, MF_SEPARATOR, 0, null());
            let color = format!(
                "Цвет и прозрачность… ({})",
                String::from(profile.appearance.color)
            );
            item(
                sub,
                id + PROFILE_COLOR,
                &color,
                false,
                profile.appearance.accent != Accent::Normal,
            );
            item(
                sub,
                id + PROFILE_PEEK,
                "Показывать кнопку рабочего стола",
                profile.appearance.show_peek,
                true,
            );
            attach(menu, sub, label);
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        item(menu, EDIT, "Открыть файл настроек", false, true);
        item(menu, RELOAD, "Перечитать файл настроек", false, true);
        attach(parent, menu, "Панель задач");
    }
}

pub fn command(taskbar: &mut Taskbar, root: HWND, command: u32) -> Result<()> {
    match command {
        EDIT => {
            let path = Settings::path()?;
            let path = wide(&format!("\"{}\"", path.display()));
            // Use the system editor so a file association cannot run the JSON as a command.
            let result = unsafe {
                ShellExecuteW(
                    root,
                    w!("open"),
                    w!("notepad.exe"),
                    path.as_ptr(),
                    null(),
                    SW_SHOWNORMAL,
                )
            };
            return if result as usize > 32 {
                Ok(())
            } else {
                Err("Не удалось открыть taskbar.json.".into())
            };
        }
        RELOAD => return taskbar.reload(),
        ENABLE => {
            if let Some(error) = &taskbar.load_error {
                return Err(error.clone());
            }
            taskbar.config.enabled = !taskbar.config.enabled;
        }
        id if id >= PROFILES => {
            let Some((mode, _)) = Mode::OPTIONS.get(((id - PROFILES) / STRIDE) as usize) else {
                return Ok(());
            };
            if let Some(error) = &taskbar.load_error {
                return Err(error.clone());
            }
            let profile = taskbar.config.profile_mut(*mode);
            match (id - PROFILES) % STRIDE {
                PROFILE_ENABLED if *mode != Mode::Desktop => profile.enabled = !profile.enabled,
                PROFILE_PEEK => profile.appearance.show_peek = !profile.appearance.show_peek,
                PROFILE_COLOR if profile.appearance.accent != Accent::Normal => {
                    let initial = profile.appearance;
                    let result = pick_color(root, taskbar, initial);
                    taskbar.apply_settings();
                    let Some(color) = result? else { return Ok(()) };
                    taskbar.config.profile_mut(*mode).appearance.color = color;
                }
                index if (index as usize) < Accent::OPTIONS.len() => {
                    profile.appearance.accent = Accent::OPTIONS[index as usize].0
                }
                _ => return Ok(()),
            }
        }
        _ => return Ok(()),
    }
    if let Err(error) = taskbar.config.save() {
        let _ = taskbar.reload();
        return Err(error);
    }
    taskbar.update_hooks();
    taskbar.apply_settings();
    Ok(())
}

const HEX: i32 = 30;
const ALPHA: i32 = 31;
const PICK: i32 = 32;
const ALPHA_LABEL: i32 = 33;
const TBM_GETPOS: u32 = WM_USER;
thread_local! {
    static SETTING_HEX: Cell<bool> = const { Cell::new(false) };
}

// Dialog template in dialog units. Pack 16-bit fields into aligned DWORDs
// for DialogBoxIndirectParamW and align each DLGITEMTEMPLATE separately.
fn color_template() -> Vec<u32> {
    struct Control<'a> {
        id: i32,
        class: &'a str,
        title: &'a str,
        style: u32,
        rect: [u16; 4],
    }
    fn dword(words: &mut Vec<u16>, value: u32) {
        words.extend([value as u16, (value >> 16) as u16]);
    }
    fn text(words: &mut Vec<u16>, s: &str) {
        words.extend(s.encode_utf16().chain(Some(0)));
    }
    let controls = [
        Control {
            id: -1,
            class: "Static",
            title: "RGB и непрозрачность (#RRGGBBAA)",
            style: 0,
            rect: [9, 9, 190, 10],
        },
        Control {
            id: HEX,
            class: "Edit",
            title: "",
            style: WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
            rect: [9, 24, 90, 14],
        },
        Control {
            id: PICK,
            class: "Button",
            title: "Палитра…",
            style: WS_TABSTOP,
            rect: [119, 23, 71, 15],
        },
        Control {
            id: ALPHA_LABEL,
            class: "Static",
            title: "Непрозрачность",
            style: 0,
            rect: [9, 46, 180, 10],
        },
        Control {
            id: ALPHA,
            class: "msctls_trackbar32",
            title: "",
            style: WS_TABSTOP | TBS_NOTICKS,
            rect: [7, 58, 185, 19],
        },
        Control {
            id: IDOK,
            class: "Button",
            title: "Применить",
            style: WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            rect: [72, 84, 57, 15],
        },
        Control {
            id: IDCANCEL,
            class: "Button",
            title: "Отмена",
            style: WS_TABSTOP,
            rect: [134, 84, 56, 15],
        },
    ];
    let mut words = Vec::with_capacity(256);
    dword(
        &mut words,
        WS_POPUP
            | WS_CAPTION
            | WS_SYSMENU
            | DS_SETFONT as u32
            | DS_MODALFRAME as u32
            | DS_CENTER as u32,
    );
    dword(&mut words, 0);
    words.extend([controls.len() as u16, 0, 0, 199, 108, 0, 0]);
    text(&mut words, "Цвет панели задач");
    words.push(9);
    text(&mut words, "Segoe UI");
    for c in controls {
        if words.len() % 2 != 0 {
            words.push(0);
        }
        dword(&mut words, WS_CHILD | WS_VISIBLE | c.style);
        dword(&mut words, 0);
        words.extend(c.rect);
        words.push(c.id as u16);
        text(&mut words, c.class);
        text(&mut words, c.title);
        words.push(0);
    }
    words
        .chunks(2)
        .map(|pair| pair[0] as u32 | ((pair.get(1).copied().unwrap_or(0) as u32) << 16))
        .collect()
}

struct ColorDialog<'a> {
    taskbar: &'a Taskbar,
    preview: Appearance,
    choice: Tint,
    custom: [u32; 16],
}

unsafe fn edited_color(dlg: HWND) -> Option<Tint> {
    let mut buffer = [0; 10];
    let len = GetDlgItemTextW(dlg, HEX, buffer.as_mut_ptr(), buffer.len() as i32);
    Tint::try_from(String::from_utf16_lossy(&buffer[..len as usize])).ok()
}

impl ColorDialog<'_> {
    fn change(&mut self, dlg: HWND, color: Tint, update_edit: bool) {
        self.choice = color;
        unsafe {
            if update_edit {
                SETTING_HEX.set(true);
                SetDlgItemTextW(dlg, HEX, wide(&String::from(color)).as_ptr());
                SETTING_HEX.set(false);
            }
            SendDlgItemMessageW(dlg, ALPHA, TBM_SETPOS, 1, color.0[3] as isize);
            let label = format!(
                "Непрозрачность: {}%",
                (u32::from(color.0[3]) * 100 + 127) / 255
            );
            SetDlgItemTextW(dlg, ALPHA_LABEL, wide(&label).as_ptr());
            EnableWindow(GetDlgItem(dlg, IDOK), 1);
        }
        self.preview.color = color;
        self.taskbar.apply(Some(self.preview), false);
    }
}

unsafe extern "system" fn color_proc(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    if msg == WM_INITDIALOG {
        SetWindowLongPtrW(dlg, GWLP_USERDATA, lp);
        let state = &mut *(lp as *mut ColorDialog<'_>);
        SendDlgItemMessageW(dlg, HEX, EM_SETLIMITTEXT, 9, 0);
        SendDlgItemMessageW(dlg, ALPHA, TBM_SETRANGE, 1, 255 << 16);
        state.change(dlg, state.choice, true);
        return 1;
    }
    let ptr = GetWindowLongPtrW(dlg, GWLP_USERDATA) as *mut ColorDialog<'_>;
    if ptr.is_null() {
        return 0;
    }
    let id = (wp & 0xffff) as i32;
    match msg {
        WM_CLOSE => {
            EndDialog(dlg, IDCANCEL as isize);
            1
        }
        WM_COMMAND if id == IDCANCEL => {
            EndDialog(dlg, IDCANCEL as isize);
            1
        }
        WM_COMMAND if id == IDOK => {
            if let Some(color) = edited_color(dlg) {
                (*ptr).choice = color;
                EndDialog(dlg, IDOK as isize);
            }
            1
        }
        WM_COMMAND if id == HEX && (wp >> 16) as u32 == EN_CHANGE => {
            if !SETTING_HEX.get() {
                match edited_color(dlg) {
                    Some(color) => (*ptr).change(dlg, color, false),
                    None => {
                        EnableWindow(GetDlgItem(dlg, IDOK), 0);
                    }
                }
            }
            1
        }
        WM_COMMAND if id == PICK => {
            let [r, g, b, alpha] = (*ptr).choice.0;
            let mut picker = CHOOSECOLORW {
                lStructSize: size_of::<CHOOSECOLORW>() as u32,
                hwndOwner: dlg,
                rgbResult: u32::from_le_bytes([r, g, b, 0]),
                lpCustColors: (&raw mut (*ptr).custom).cast(),
                Flags: CC_RGBINIT | CC_FULLOPEN,
                ..CHOOSECOLORW::default()
            };
            if ChooseColorW(&mut picker) != 0 {
                let [r, g, b, _] = picker.rgbResult.to_le_bytes();
                (*ptr).change(dlg, Tint([r, g, b, alpha]), true);
            }
            1
        }
        WM_HSCROLL if lp as HWND == GetDlgItem(dlg, ALPHA) => {
            let mut color = (*ptr).choice;
            color.0[3] = SendDlgItemMessageW(dlg, ALPHA, TBM_GETPOS, 0, 0) as u8;
            (*ptr).change(dlg, color, true);
            1
        }
        _ => 0,
    }
}

fn pick_color(root: HWND, taskbar: &Taskbar, initial: Appearance) -> Result<Option<Tint>> {
    let mut state = ColorDialog {
        taskbar,
        preview: initial,
        choice: initial.color,
        custom: [0; 16],
    };
    let template = color_template();
    unsafe {
        InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES,
        });
        let outcome = DialogBoxIndirectParamW(
            GetModuleHandleW(null()),
            template.as_ptr().cast(),
            root,
            Some(color_proc),
            std::ptr::from_mut(&mut state) as LPARAM,
        );
        match outcome {
            -1 => Err(win_error("Не удалось открыть палитру")),
            n if n == IDOK as isize => Ok(Some(state.choice)),
            _ => Ok(None),
        }
    }
}
