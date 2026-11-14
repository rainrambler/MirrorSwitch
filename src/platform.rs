//! Rustup mirror environment persistence.
//!
//! - Windows: the current user's `HKCU\Environment` values.
//! - macOS / Unix: a marked block in the user's shell profile, plus
//!   `launchctl setenv` on macOS so newly launched GUI apps see the change.

use std::env;
use std::path::PathBuf;

#[cfg(windows)]
use anyhow::Context;
use anyhow::Result;

#[cfg(unix)]
use crate::shell;

pub const RUSTUP_DIST_SERVER: &str = "RUSTUP_DIST_SERVER";
pub const RUSTUP_UPDATE_ROOT: &str = "RUSTUP_UPDATE_ROOT";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Some storage variants only occur on the other OS.
pub enum EnvScope {
    User,
    Shell,
    System,
    Process,
    Unset,
}

impl EnvScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "用户变量",
            Self::Shell => "Shell 配置",
            Self::System => "系统变量",
            Self::Process => "当前进程",
            Self::Unset => "未设置",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct EnvVarState {
    pub effective: Option<String>,
    pub user: Option<String>,
    pub system: Option<String>,
    pub scope: Option<EnvScope>,
}

impl EnvVarState {
    pub fn is_set(&self) -> bool {
        self.effective
            .as_deref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    }

    pub fn scope_label(&self) -> &'static str {
        self.scope.unwrap_or(EnvScope::Unset).label()
    }

    pub fn effective_display(&self) -> String {
        match self.effective.as_deref() {
            Some(value) if !value.trim().is_empty() => value.to_owned(),
            _ => "未设置（官方）".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RustupEnvState {
    pub dist: EnvVarState,
    pub update: EnvVarState,
    pub warnings: Vec<String>,
}

impl RustupEnvState {
    pub fn is_official(&self) -> bool {
        !self.dist.is_set() && !self.update.is_set()
    }

    pub fn has_system_override(&self) -> bool {
        let dist = self
            .dist
            .system
            .as_deref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        let update = self
            .update
            .system
            .as_deref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        dist || update
    }
}

pub fn read_rustup_env() -> RustupEnvState {
    #[cfg(windows)]
    {
        RustupEnvState {
            dist: read_env_var(RUSTUP_DIST_SERVER),
            update: read_env_var(RUSTUP_UPDATE_ROOT),
            warnings: Vec::new(),
        }
    }
    #[cfg(unix)]
    {
        read_rustup_env_unix()
    }
}

/// Where mirror settings are persisted, for display in the UI.
pub fn storage_note() -> String {
    #[cfg(windows)]
    {
        "HKCU\\Environment（当前用户环境变量）".to_owned()
    }
    #[cfg(unix)]
    {
        let profile = shell::detect();
        format!(
            "{}（{} 托管块）",
            profile.path.display(),
            profile.kind.label()
        )
    }
}

/// The shell profile file managed on macOS/Unix, if any.
pub fn shell_profile_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        None
    }
    #[cfg(unix)]
    {
        Some(shell::detect().path)
    }
}

pub fn rustup_restart_tip() -> &'static str {
    #[cfg(windows)]
    {
        "Rustup 镜像通过用户环境变量保存；已打开的终端、IDE 需要重启后才会读取新的 RUSTUP_DIST_SERVER / RUSTUP_UPDATE_ROOT。"
    }
    #[cfg(unix)]
    {
        "Rustup 镜像写入 Shell 配置；新开的终端会生效。已运行的终端、IDE 需要重启；从 Finder/Dock 启动的图形应用可能需要重新登录才能继承 launchctl 环境。"
    }
}

/// Set or delete both Rustup variables. On Unix this edits the shell profile
/// and updates the current process + launchd session.
pub fn set_rustup_env(dist: Option<&str>, update: Option<&str>) -> Result<Vec<String>> {
    #[cfg(windows)]
    {
        set_user_values(&[(RUSTUP_DIST_SERVER, dist), (RUSTUP_UPDATE_ROOT, update)])?;
        set_process_value(RUSTUP_DIST_SERVER, dist);
        set_process_value(RUSTUP_UPDATE_ROOT, update);
        broadcast_environment_change();

        let state = read_rustup_env();
        let mut warnings = Vec::new();
        if state.has_system_override() {
            let mut names = Vec::new();
            if state
                .dist
                .system
                .as_deref()
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
            {
                names.push(RUSTUP_DIST_SERVER);
            }
            if state
                .update
                .system
                .as_deref()
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
            {
                names.push(RUSTUP_UPDATE_ROOT);
            }
            warnings.push(format!(
                "检测到系统级环境变量 {}。系统变量优先级高于用户变量，需以管理员身份在“系统环境变量”中清理后才能完全恢复官方源。",
                names.join("、")
            ));
        }
        Ok(warnings)
    }
    #[cfg(unix)]
    {
        let profile = shell::detect();
        let mut warnings = shell::apply(&profile, dist, update)?;
        set_process_value(RUSTUP_DIST_SERVER, dist);
        set_process_value(RUSTUP_UPDATE_ROOT, update);
        warnings.extend(update_launchd(dist, update));
        Ok(warnings)
    }
}

/// Re-apply the shell-profile values to the current process and launchd session
/// after restoring a whole shell profile file from a snapshot.
pub fn sync_session_env_from_profile() -> Vec<String> {
    #[cfg(windows)]
    {
        Vec::new()
    }
    #[cfg(unix)]
    {
        let profile = shell::detect();
        match shell::read(&profile) {
            Ok(values) => {
                set_process_value(RUSTUP_DIST_SERVER, values.dist.as_deref());
                set_process_value(RUSTUP_UPDATE_ROOT, values.update.as_deref());
                let mut warnings = update_launchd(values.dist.as_deref(), values.update.as_deref());
                if values.unmanaged {
                    warnings.push(
                        "Shell 配置中仍存在未托管的 RUSTUP_* 设置；新终端可能继续读取旧值。"
                            .to_owned(),
                    );
                }
                warnings
            }
            Err(error) => vec![format!("无法重新读取 Shell 配置：{error:#}")],
        }
    }
}

fn set_process_value(name: &str, value: Option<&str>) {
    match value {
        Some(value) => env::set_var(name, value),
        None => env::remove_var(name),
    }
}

#[cfg(windows)]
fn read_env_var(name: &str) -> EnvVarState {
    let user = read_user_value(name);
    let system = read_system_value(name);
    let process = env::var(name).ok().filter(|v| !v.is_empty());

    let (effective, scope) = if user.is_some() {
        (user.clone(), Some(EnvScope::User))
    } else if system.is_some() {
        (system.clone(), Some(EnvScope::System))
    } else if process.is_some() {
        (process.clone(), Some(EnvScope::Process))
    } else {
        (None, Some(EnvScope::Unset))
    };

    EnvVarState {
        effective,
        user,
        system,
        scope,
    }
}

#[cfg(unix)]
fn read_rustup_env_unix() -> RustupEnvState {
    let profile = shell::detect();
    let (values, mut warnings) = match shell::read(&profile) {
        Ok(values) => (values, Vec::new()),
        Err(error) => (
            shell::ShellValues::default(),
            vec![format!("无法读取 Shell 配置：{error:#}")],
        ),
    };
    if values.unmanaged {
        warnings.push(format!(
            "{} 中存在未托管的 RUSTUP_* 设置。MirrorSwitch 的托管块会覆盖它们，但建议之后手动清理旧行。",
            profile.path.display()
        ));
    }
    RustupEnvState {
        dist: unix_env_var(values.dist, RUSTUP_DIST_SERVER),
        update: unix_env_var(values.update, RUSTUP_UPDATE_ROOT),
        warnings,
    }
}

#[cfg(unix)]
fn unix_env_var(profile_value: Option<String>, name: &str) -> EnvVarState {
    let process = env::var(name).ok().filter(|v| !v.is_empty());
    let profile_is_set = profile_value
        .as_deref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let (effective, scope) = if profile_is_set {
        (profile_value.clone(), Some(EnvScope::Shell))
    } else if process.is_some() {
        (process.clone(), Some(EnvScope::Process))
    } else {
        (None, Some(EnvScope::Unset))
    };
    EnvVarState {
        effective,
        user: profile_value,
        system: None,
        scope,
    }
}

#[cfg(target_os = "macos")]
fn update_launchd(dist: Option<&str>, update: Option<&str>) -> Vec<String> {
    use std::process::Command;

    let mut warnings = Vec::new();
    for (name, value) in [(RUSTUP_DIST_SERVER, dist), (RUSTUP_UPDATE_ROOT, update)] {
        let status = match value {
            Some(value) => Command::new("/bin/launchctl")
                .args(["setenv", name, value])
                .status(),
            None => Command::new("/bin/launchctl")
                .args(["unsetenv", name])
                .status(),
        };
        match status {
            Ok(status) if status.success() => {}
            Ok(status) => warnings.push(format!(
                "launchctl 更新 {name} 失败（退出码 {}）；新终端仍可从 Shell 配置读取。",
                status.code().unwrap_or(-1)
            )),
            Err(error) => warnings.push(format!(
                "无法执行 launchctl 更新 {name}：{error}。新终端仍可从 Shell 配置读取。"
            )),
        }
    }
    warnings
}

#[cfg(all(unix, not(target_os = "macos")))]
fn update_launchd(_dist: Option<&str>, _update: Option<&str>) -> Vec<String> {
    Vec::new()
}

#[cfg(windows)]
fn read_user_value(name: &str) -> Option<String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_READ)
        .ok()?;
    key.get_value::<String, _>(name).ok()
}

#[cfg(windows)]
fn read_system_value(name: &str) -> Option<String> {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
            KEY_READ,
        )
        .ok()?;
    key.get_value::<String, _>(name).ok()
}

#[cfg(windows)]
fn set_user_values(values: &[(&str, Option<&str>)]) -> Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
    use winreg::RegKey;

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_READ | KEY_SET_VALUE)
        .context("无法打开当前用户的环境变量注册表项")?;

    let previous: Vec<(&str, Option<String>)> = values
        .iter()
        .map(|(name, _)| (*name, key.get_value::<String, _>(name).ok()))
        .collect();

    let mut applied: Vec<&str> = Vec::new();
    for (name, value) in values {
        let result = match value {
            Some(value) => key.set_value(name, &value.to_string()),
            None => match key.delete_value(name) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            },
        };
        match result {
            Ok(()) => applied.push(name),
            Err(error) => {
                for rollback_name in applied {
                    if let Some((_, old)) = previous.iter().find(|(n, _)| *n == rollback_name) {
                        match old {
                            Some(old) => {
                                let _ = key.set_value(rollback_name, old);
                            }
                            None => {
                                let _ = key.delete_value(rollback_name);
                            }
                        }
                    }
                }
                return Err(anyhow::Error::new(error))
                    .with_context(|| format!("无法写入环境变量 {name}"));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn broadcast_environment_change() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    let parameter = b"Environment\0";
    let mut result: usize = 0;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            parameter.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            1_000,
            &mut result,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_without_panicking() {
        let state = read_rustup_env();
        let _ = state.dist.effective_display();
        let _ = state.update.scope_label();
        let _ = storage_note();
    }

    #[test]
    fn scope_labels_cover_all_storage_types() {
        assert_eq!(EnvScope::User.label(), "用户变量");
        assert_eq!(EnvScope::Shell.label(), "Shell 配置");
        assert_eq!(EnvScope::System.label(), "系统变量");
    }
}
