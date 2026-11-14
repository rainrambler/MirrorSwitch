use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::util::atomic_write;

pub const SETTINGS_SCHEMA: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Dark,
    Light,
}

impl ThemeMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "深色",
            Self::Light => "浅色",
        }
    }

    pub fn toggled(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema: u32,
    pub theme: ThemeMode,
    pub auto_backup: bool,
    pub max_auto_backups: usize,
    pub test_sample_mb: u64,
    pub test_concurrency: usize,
    pub include_official: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema: SETTINGS_SCHEMA,
            theme: ThemeMode::Dark,
            auto_backup: true,
            max_auto_backups: 30,
            test_sample_mb: 2,
            test_concurrency: 3,
            include_official: true,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> (Self, Option<String>) {
        if !path.exists() {
            return (Self::default(), None);
        }
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Self>(&text) {
                Ok(settings) => {
                    let mut settings = settings;
                    settings.normalize();
                    (settings, None)
                }
                Err(error) => (
                    Self::default(),
                    Some(format!("设置文件无法解析，已使用默认值：{error}")),
                ),
            },
            Err(error) => (
                Self::default(),
                Some(format!("无法读取设置文件，已使用默认值：{error}")),
            ),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        atomic_write(path, json.as_bytes())
    }

    pub fn normalize(&mut self) {
        self.schema = SETTINGS_SCHEMA;
        self.max_auto_backups = self.max_auto_backups.clamp(5, 100);
        self.test_sample_mb = self.test_sample_mb.clamp(1, 8);
        self.test_concurrency = self.test_concurrency.clamp(1, 6);
    }

    pub fn sample_bytes(&self) -> u64 {
        self.test_sample_mb * 1024 * 1024
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::unique_temp_dir;

    #[test]
    fn defaults_roundtrip_and_normalize() {
        let dir = unique_temp_dir("settings");
        let path = dir.join("settings.json");
        let settings = Settings {
            test_concurrency: 99,
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let (loaded, warning) = Settings::load(&path);
        assert!(warning.is_none());
        assert_eq!(loaded.test_concurrency, 6);
        assert_eq!(loaded.theme, ThemeMode::Dark);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_settings_fall_back() {
        let dir = unique_temp_dir("settings-invalid");
        let path = dir.join("settings.json");
        std::fs::write(&path, "{nope").unwrap();
        let (settings, warning) = Settings::load(&path);
        assert_eq!(settings.theme, ThemeMode::Dark);
        assert!(warning.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
}
