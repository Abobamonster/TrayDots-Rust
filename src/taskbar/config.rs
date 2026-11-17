use crate::{Result, data_dir, io_result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io, path::PathBuf};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Accent {
    #[default]
    Normal,
    Opaque,
    Clear,
    Blur,
    Acrylic,
}

impl Accent {
    pub(super) const OPTIONS: [(Self, &'static str); 5] = [
        (Self::Normal, "Стандартная"),
        (Self::Opaque, "Сплошная"),
        (Self::Clear, "Прозрачная"),
        (Self::Blur, "Размытие"),
        (Self::Acrylic, "Акрил"),
    ];
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(try_from = "String", into = "String")]
pub(super) struct Tint(pub [u8; 4]);

impl TryFrom<String> for Tint {
    type Error = &'static str;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Self::parse(&value).ok_or("Expected #RGB, #RGBA, #RRGGBB or #RRGGBBAA")
    }
}

impl From<Tint> for String {
    fn from(Tint([r, g, b, a]): Tint) -> Self {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    }
}

impl Tint {
    fn parse(value: &str) -> Option<Self> {
        let value = value.strip_prefix('#')?;
        if !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let digits = match value.len() {
            3 | 4 => 1,
            6 | 8 => 2,
            _ => return None,
        };
        let mut rgba = [0, 0, 0, 255];
        for (index, channel) in rgba.iter_mut().take(value.len() / digits).enumerate() {
            let start = index * digits;
            *channel = u8::from_str_radix(&value[start..start + digits], 16).ok()?;
            if digits == 1 {
                *channel *= 17;
            }
        }
        Some(Self(rgba))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct Appearance {
    pub accent: Accent,
    pub color: Tint,
    pub show_peek: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            accent: Accent::Normal,
            color: Tint::default(),
            show_peek: true,
        }
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct AppearanceOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    accent: Option<Accent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<Tint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    show_peek: Option<bool>,
}

impl AppearanceOverride {
    fn apply(&self, a: Appearance) -> Appearance {
        Appearance {
            accent: self.accent.unwrap_or(a.accent),
            color: self.color.unwrap_or(a.color),
            show_peek: self.show_peek.unwrap_or(a.show_peek),
        }
    }
}

#[derive(Default, Deserialize, Serialize)]
struct Rule {
    #[serde(flatten)]
    active: AppearanceOverride,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    inactive: Option<AppearanceOverride>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Rules {
    window_class: BTreeMap<String, Rule>,
    window_title: BTreeMap<String, Rule>,
    process_name: BTreeMap<String, Rule>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct Ignored {
    window_class: Vec<String>,
    window_title: Vec<String>,
    process_name: Vec<String>,
}

pub(super) struct WindowDetails {
    pub class: String,
    pub title: String,
    pub process: String,
}

impl Ignored {
    pub fn is_empty(&self) -> bool {
        self.window_class.is_empty() && self.window_title.is_empty() && self.process_name.is_empty()
    }

    pub fn matches(&self, w: &WindowDetails) -> bool {
        self.window_class
            .iter()
            .any(|p| w.class.eq_ignore_ascii_case(p))
            || self.window_title.iter().any(|p| w.title.contains(p))
            || self
                .process_name
                .iter()
                .any(|p| w.process.eq_ignore_ascii_case(p))
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct Profile {
    pub enabled: bool,
    #[serde(flatten)]
    pub appearance: Appearance,
    rules: Rules,
}

impl Profile {
    pub fn has_rules(&self) -> bool {
        !self.rules.window_class.is_empty()
            || !self.rules.window_title.is_empty()
            || !self.rules.process_name.is_empty()
    }

    pub fn resolve(&self, w: &WindowDetails, active: bool) -> Appearance {
        let rule = self
            .rules
            .window_class
            .iter()
            .find(|(p, _)| w.class.eq_ignore_ascii_case(p))
            .or_else(|| {
                self.rules
                    .window_title
                    .iter()
                    .find(|(p, _)| w.title.contains(p.as_str()))
            })
            .or_else(|| {
                self.rules
                    .process_name
                    .iter()
                    .find(|(p, _)| w.process.eq_ignore_ascii_case(p))
            });
        match rule {
            Some((_, rule)) => {
                let appearance = rule.active.apply(self.appearance);
                if active {
                    appearance
                } else {
                    rule.inactive
                        .as_ref()
                        .map_or(appearance, |inactive| inactive.apply(appearance))
                }
            }
            None => self.appearance,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Mode {
    Desktop,
    Visible,
    Maximized,
    Start,
    Search,
    TaskView,
    Battery,
}

impl Mode {
    pub(super) const OPTIONS: [(Self, &'static str); 7] = [
        (Self::Desktop, "Рабочий стол"),
        (Self::Visible, "Видимое окно"),
        (Self::Maximized, "Развёрнутое окно"),
        (Self::Start, "Открыт Пуск"),
        (Self::Search, "Открыт поиск"),
        (Self::TaskView, "Представление задач"),
        (Self::Battery, "Экономия заряда"),
    ];
}

#[derive(Deserialize, Serialize)]
#[serde(default)]
pub(super) struct Settings {
    pub enabled: bool,
    desktop_appearance: Profile,
    visible_window_appearance: Profile,
    maximized_window_appearance: Profile,
    start_opened_appearance: Profile,
    search_opened_appearance: Profile,
    task_view_opened_appearance: Profile,
    battery_saver_appearance: Profile,
    pub ignored_windows: Ignored,
}

impl Default for Settings {
    fn default() -> Self {
        fn profile(enabled: bool, accent: Accent, show_peek: bool) -> Profile {
            Profile {
                enabled,
                appearance: Appearance {
                    accent,
                    show_peek,
                    ..Appearance::default()
                },
                ..Profile::default()
            }
        }
        Self {
            enabled: true,
            desktop_appearance: profile(true, Accent::Clear, false),
            visible_window_appearance: profile(false, Accent::Clear, true),
            maximized_window_appearance: profile(false, Accent::Clear, true),
            start_opened_appearance: profile(true, Accent::Normal, true),
            search_opened_appearance: profile(true, Accent::Normal, true),
            task_view_opened_appearance: profile(true, Accent::Normal, false),
            battery_saver_appearance: profile(false, Accent::Opaque, true),
            ignored_windows: Ignored::default(),
        }
    }
}

impl Settings {
    pub fn profile(&self, mode: Mode) -> &Profile {
        match mode {
            Mode::Desktop => &self.desktop_appearance,
            Mode::Visible => &self.visible_window_appearance,
            Mode::Maximized => &self.maximized_window_appearance,
            Mode::Start => &self.start_opened_appearance,
            Mode::Search => &self.search_opened_appearance,
            Mode::TaskView => &self.task_view_opened_appearance,
            Mode::Battery => &self.battery_saver_appearance,
        }
    }

    pub fn profile_mut(&mut self, mode: Mode) -> &mut Profile {
        match mode {
            Mode::Desktop => &mut self.desktop_appearance,
            Mode::Visible => &mut self.visible_window_appearance,
            Mode::Maximized => &mut self.maximized_window_appearance,
            Mode::Start => &mut self.start_opened_appearance,
            Mode::Search => &mut self.search_opened_appearance,
            Mode::TaskView => &mut self.task_view_opened_appearance,
            Mode::Battery => &mut self.battery_saver_appearance,
        }
    }

    pub fn path() -> Result<PathBuf> {
        Ok(data_dir()?.join("taskbar.json"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let imported = import_translucent_tb().unwrap_or_default();
                imported.save()?;
                Ok(imported)
            }
            Err(e) => Err(e.to_string()),
        }
    }

    fn parse(text: &str) -> Result<Self> {
        let text = text.trim_start_matches('\u{feff}');
        // The original settings file starts with a comment before its JSON object.
        let text = text
            .lines()
            .skip_while(|line| {
                let line = line.trim();
                line.is_empty() || line.starts_with("//")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut settings: Self =
            serde_json::from_str(&text).map_err(|e| format!("taskbar.json: {e}"))?;
        settings.desktop_appearance.enabled = true;
        Ok(settings)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        io_result(fs::create_dir_all(data_dir()?))?;
        let temp = path.with_extension("tmp");
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        io_result(fs::write(&temp, json))?;
        io_result(fs::rename(temp, path))
    }
}

fn import_translucent_tb() -> Option<Settings> {
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA")?);
    let candidates = [
        local
            .join("Packages")
            .join("28017CharlesMilette.TranslucentTB_v826wp6bftszj")
            .join("RoamingState")
            .join("settings.json"),
        local.join("TranslucentTB").join("settings.json"),
    ];
    candidates.iter().find_map(|path| {
        let text = fs::read_to_string(path).ok()?;
        Settings::parse(&text).ok()
    })
}
