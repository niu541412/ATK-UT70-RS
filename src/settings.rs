use crate::i18n::Language;
use std::fs;
use std::path::PathBuf;

pub const WINDOWS: [u16; 8] = [1, 5, 10, 30, 60, 600, 3600, 0];

pub fn window_label(seconds: u16, lang: Language) -> String {
    match seconds {
        0 => lang.pick("全部", "All").to_owned(),
        60 => lang.pick("1 分钟", "1 min").to_owned(),
        600 => lang.pick("10 分钟", "10 min").to_owned(),
        3600 => lang.pick("1 小时", "1 h").to_owned(),
        _ => match lang {
            Language::Chinese => format!("{seconds} 秒"),
            Language::English => format!("{seconds} s"),
        },
    }
}

pub struct Settings {
    pub language: Language,
    pub dark_mode: bool,
    pub cursor_values: bool,
    pub window_secs: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::Chinese,
            dark_mode: true,
            cursor_values: false,
            window_secs: 30,
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        #[cfg(target_os = "windows")]
        let base = std::env::var_os("APPDATA").map(PathBuf::from)?;
        #[cfg(target_os = "macos")]
        let base = PathBuf::from(std::env::var_os("HOME")?)
            .join("Library")
            .join("Application Support");
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
        Some(base.join("ATK-UT70-RS").join("settings.ini"))
    }

    pub fn load() -> Self {
        let content = Self::path().and_then(|path| fs::read_to_string(path).ok());
        content.as_deref().map(Self::parse).unwrap_or_default()
    }

    fn parse(content: &str) -> Self {
        let mut settings = Self::default();
        for line in content.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "language" => match value {
                    "zh" => settings.language = Language::Chinese,
                    "en" => settings.language = Language::English,
                    _ => {}
                },
                "dark_mode" => settings.dark_mode = value == "true",
                "cursor_values" => settings.cursor_values = value == "true",
                "window_secs" => {
                    if let Ok(window) = value.parse::<u16>() {
                        if WINDOWS.contains(&window) {
                            settings.window_secs = window;
                        }
                    }
                }
                _ => {}
            }
        }
        settings
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }
        let language = match self.language {
            Language::Chinese => "zh",
            Language::English => "en",
        };
        let content = format!(
            "language={language}\ndark_mode={}\ncursor_values={}\nwindow_secs={}\n",
            self.dark_mode, self.cursor_values, self.window_secs
        );
        let _ = fs::write(path, content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_window_keeps_default() {
        let settings = Settings::parse("language=en\ndark_mode=false\nwindow_secs=42\n");
        assert!(matches!(settings.language, Language::English));
        assert!(!settings.dark_mode);
        assert_eq!(settings.window_secs, 30);
        assert_eq!(Settings::parse("window_secs=0\n").window_secs, 0);
        assert_eq!(Settings::parse("window_secs=3600\n").window_secs, 3600);
    }
}
