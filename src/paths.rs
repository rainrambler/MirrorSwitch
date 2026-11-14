use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};

pub const APP_NAME: &str = "MirrorSwitch";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub backup_dir: PathBuf,
    pub log_dir: PathBuf,
    pub settings_path: PathBuf,
    pub speed_cache_path: PathBuf,
    pub log_path: PathBuf,
    pub cargo_home: PathBuf,
    pub cargo_config: PathBuf,
    pub cargo_config_toml: PathBuf,
    pub cargo_config_legacy: PathBuf,
    pub legacy_config_in_use: bool,
    pub both_configs_exist: bool,
    pub rustup_home: PathBuf,
    pub rustup_settings: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let home = home_dir().context("无法确定用户主目录")?;
        let cargo_home = env::var_os("CARGO_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".cargo"));
        let rustup_home = env::var_os("RUSTUP_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".rustup"));

        let explicit_data = env::var_os("MIRRORSWITCH_DATA_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let mut candidates = Vec::new();
        if let Some(data_dir) = explicit_data {
            candidates.push(data_dir);
        } else {
            if let Some(appdata) = env::var_os("APPDATA")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .or_else(dirs::data_dir)
            {
                candidates.push(appdata.join(APP_NAME));
            }
            if let Some(local) = env::var_os("LOCALAPPDATA")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .or_else(dirs::data_local_dir)
            {
                let path = local.join(APP_NAME);
                if !candidates.contains(&path) {
                    candidates.push(path);
                }
            }
            candidates.push(home.join(".mirrorswitch"));
        }

        let mut errors = Vec::new();
        for data_dir in candidates {
            let paths = Self::with_roots(data_dir, cargo_home.clone(), rustup_home.clone());
            match paths.ensure_dirs() {
                Ok(()) => return Ok(paths),
                Err(error) => errors.push(format!("{}：{error:#}", paths.data_dir.display())),
            }
        }
        anyhow::bail!("没有可写的应用数据目录。已尝试：{}", errors.join("；"))
    }

    pub fn with_roots(data_dir: PathBuf, cargo_home: PathBuf, rustup_home: PathBuf) -> Self {
        let cargo_config_toml = cargo_home.join("config.toml");
        let cargo_config_legacy = cargo_home.join("config");
        let toml_exists = cargo_config_toml.exists();
        let legacy_exists = cargo_config_legacy.exists();
        let legacy_config_in_use = !toml_exists && legacy_exists;
        let cargo_config = if legacy_config_in_use {
            cargo_config_legacy.clone()
        } else {
            cargo_config_toml.clone()
        };
        let log_dir = data_dir.join("logs");
        let backup_dir = data_dir.join("backups");
        Self {
            settings_path: data_dir.join("settings.json"),
            speed_cache_path: data_dir.join("last_speed.json"),
            log_path: log_dir.join("mirrorswitch.log"),
            data_dir,
            backup_dir,
            log_dir,
            cargo_home,
            cargo_config,
            cargo_config_toml,
            cargo_config_legacy,
            legacy_config_in_use,
            both_configs_exist: toml_exists && legacy_exists,
            rustup_settings: rustup_home.join("settings.toml"),
            rustup_home,
        }
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.data_dir)
            .with_context(|| format!("无法创建应用数据目录 {}", self.data_dir.display()))?;
        std::fs::create_dir_all(&self.backup_dir)
            .with_context(|| format!("无法创建备份目录 {}", self.backup_dir.display()))?;
        std::fs::create_dir_all(&self.log_dir)
            .with_context(|| format!("无法创建日志目录 {}", self.log_dir.display()))?;
        Ok(())
    }

    pub fn path_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if self.both_configs_exist {
            warnings.push(format!(
                "检测到 {} 与 {} 同时存在。Cargo 会优先读取 config.toml，本工具只修改该文件。",
                self.cargo_config_toml.display(),
                self.cargo_config_legacy.display()
            ));
        } else if self.legacy_config_in_use {
            warnings.push(format!(
                "当前使用旧式配置文件 {}。本工具会保留该文件名，建议未来迁移到 config.toml。",
                self.cargo_config_legacy.display()
            ));
        }
        if !self.rustup_settings.exists() {
            warnings.push(format!(
                "未找到 {}。Rustup 可能尚未初始化，但环境变量仍可正常设置。",
                self.rustup_settings.display()
            ));
        }
        warnings
    }
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("USERPROFILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_config_is_used_when_toml_is_absent() {
        let root = crate::util::unique_temp_dir("paths-legacy");
        let cargo = root.join("cargo");
        std::fs::create_dir_all(&cargo).unwrap();
        std::fs::write(cargo.join("config"), "[net]\n").unwrap();
        let paths = AppPaths::with_roots(root.join("data"), cargo, root.join("rustup"));
        assert!(paths.legacy_config_in_use);
        assert!(paths.cargo_config.ends_with("config"));
        assert!(!paths.both_configs_exist);
        let _ = std::fs::remove_dir_all(root);
    }
}
