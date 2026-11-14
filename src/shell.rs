//! Shell profile editing for Rustup mirror environment variables on macOS and
//! other Unix systems.
//!
//! A managed, clearly marked block is appended to the user's shell profile.
//! Append-at-end matters: if the user already has unmanaged RUSTUP_* exports,
//! the managed block must win in the shell's last-assignment-wins semantics.

use std::env;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::platform::{RUSTUP_DIST_SERVER, RUSTUP_UPDATE_ROOT};
use crate::util::atomic_write;

pub const BLOCK_START: &str = "# >>> MirrorSwitch rustup mirror >>>";
pub const BLOCK_END: &str = "# <<< MirrorSwitch rustup mirror <<<";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    Zsh,
    Bash,
    Fish,
    Posix,
}

impl ShellKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::Fish => "fish",
            Self::Posix => "sh",
        }
    }

    fn from_program(program: &str) -> Self {
        let name = Path::new(program)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(program)
            .trim_start_matches('-')
            .to_ascii_lowercase();
        match name.as_str() {
            "zsh" => Self::Zsh,
            "bash" => Self::Bash,
            "fish" => Self::Fish,
            _ => Self::Posix,
        }
    }

    fn is_fish(self) -> bool {
        matches!(self, Self::Fish)
    }
}

#[derive(Clone, Debug)]
pub struct ShellProfile {
    pub path: PathBuf,
    pub kind: ShellKind,
}

#[derive(Clone, Debug, Default)]
pub struct ShellValues {
    pub dist: Option<String>,
    pub update: Option<String>,
    /// RUSTUP_* assignments exist outside our managed block.
    pub unmanaged: bool,
}

pub fn detect() -> ShellProfile {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let shell_program = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_owned());
    let kind = env::var("MIRRORSWITCH_SHELL")
        .ok()
        .map(|shell| ShellKind::from_program(&shell))
        .unwrap_or_else(|| ShellKind::from_program(&shell_program));

    let path = env::var_os("MIRRORSWITCH_SHELL_PROFILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| default_profile(&home, kind));
    ShellProfile { path, kind }
}

fn default_profile(home: &Path, kind: ShellKind) -> PathBuf {
    match kind {
        ShellKind::Zsh => home.join(".zshrc"),
        ShellKind::Bash => home.join(".bash_profile"),
        ShellKind::Fish => home.join(".config").join("fish").join("config.fish"),
        ShellKind::Posix => home.join(".profile"),
    }
}

pub fn read(profile: &ShellProfile) -> Result<ShellValues> {
    let text = match std::fs::read_to_string(&profile.path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("无法读取 Shell 配置 {}", profile.path.display()))
        }
    };
    Ok(parse(&text, profile.kind))
}

pub fn parse(text: &str, kind: ShellKind) -> ShellValues {
    let mut values = ShellValues::default();
    let mut in_block = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == BLOCK_START {
            in_block = true;
            continue;
        }
        if trimmed == BLOCK_END {
            in_block = false;
            continue;
        }
        let Some((name, value)) = parse_assignment(line, kind) else {
            continue;
        };
        let target = match name.as_str() {
            RUSTUP_DIST_SERVER => &mut values.dist,
            RUSTUP_UPDATE_ROOT => &mut values.update,
            _ => continue,
        };
        if !in_block {
            values.unmanaged = true;
        }
        *target = Some(value);
    }
    values
}

/// Apply mirror values, or remove the managed block when both are `None`.
/// Returns user-facing warnings about pre-existing unmanaged assignments.
pub fn apply(
    profile: &ShellProfile,
    dist: Option<&str>,
    update: Option<&str>,
) -> Result<Vec<String>> {
    let old = match std::fs::read_to_string(&profile.path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("无法读取 Shell 配置 {}", profile.path.display()))
        }
    };
    let old_text = old.clone().unwrap_or_default();
    let parsed = parse(&old_text, profile.kind);
    let mut base = strip_managed_blocks(&old_text);
    base = base.trim_end().to_owned();

    let wanted = dist.is_some() || update.is_some();
    let new_text = if wanted {
        let mut block = String::new();
        if !base.is_empty() {
            block.push('\n');
        }
        block.push_str(BLOCK_START);
        block.push('\n');
        block.push_str(
            "# Managed by MirrorSwitch. Remove this block to restore official Rustup mirrors.\n",
        );
        if let Some(dist) = dist {
            block.push_str(&assignment(RUSTUP_DIST_SERVER, dist, profile.kind)?);
            block.push('\n');
        }
        if let Some(update) = update {
            block.push_str(&assignment(RUSTUP_UPDATE_ROOT, update, profile.kind)?);
            block.push('\n');
        }
        block.push_str(BLOCK_END);
        block.push('\n');
        format!("{base}{block}")
    } else {
        if base.is_empty() {
            if let Some(parent) = profile.path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if old.is_some() {
                atomic_write(&profile.path, b"")?;
            }
            return Ok(unmanaged_warning(parsed.unmanaged, false));
        }
        format!("{base}\n")
    };

    if old.as_deref() != Some(new_text.as_str()) {
        atomic_write(&profile.path, new_text.as_bytes())?;
    }
    Ok(unmanaged_warning(parsed.unmanaged, wanted))
}

fn unmanaged_warning(unmanaged: bool, wanted: bool) -> Vec<String> {
    if !unmanaged {
        return Vec::new();
    }
    if wanted {
        vec![
            "Shell 配置中已有未托管的 RUSTUP_* 设置。MirrorSwitch 会把托管块追加到文件末尾以覆盖它们；建议稍后手动清理旧行。"
                .to_owned(),
        ]
    } else {
        vec![
            "检测到未托管的 RUSTUP_* 设置，移除托管块后新终端仍可能使用这些值。请手动删除对应 export 行以完全恢复官方源。"
                .to_owned(),
        ]
    }
}

fn assignment(name: &str, value: &str, kind: ShellKind) -> Result<String> {
    if value.contains('\n') || value.contains('\r') {
        bail!("环境变量值不能包含换行");
    }
    if kind.is_fish() {
        let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
        Ok(format!("set -gx {name} '{escaped}'"))
    } else {
        let escaped = value.replace('\'', "'\\''");
        Ok(format!("export {name}='{escaped}'"))
    }
}

fn strip_managed_blocks(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut in_block = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed == BLOCK_START {
            in_block = true;
            continue;
        }
        if trimmed == BLOCK_END {
            in_block = false;
            continue;
        }
        if !in_block {
            output.push_str(line);
        }
    }
    output
}

fn parse_assignment(line: &str, kind: ShellKind) -> Option<(String, String)> {
    if kind.is_fish() {
        parse_fish_assignment(line)
    } else {
        parse_posix_assignment(line)
    }
}

fn parse_posix_assignment(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("export")?;
    if !rest.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let (name, raw_value) = rest.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some((name.to_owned(), parse_shell_value(raw_value.trim())))
}

fn parse_fish_assignment(line: &str) -> Option<(String, String)> {
    let mut tokens = line.split_whitespace();
    if tokens.next()? != "set" {
        return None;
    }
    let flags = tokens.next()?;
    if !flags.contains('x') {
        return None;
    }
    let name = tokens.next()?.to_owned();
    let mut value = tokens.collect::<Vec<_>>().join(" ");
    if value.is_empty() {
        return None;
    }
    value = parse_shell_value(&value);
    Some((name, value))
}

fn parse_shell_value(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix('\'') {
        if let Some(end) = rest.find('\'') {
            return rest[..end].replace("'\\''", "'");
        }
    }
    if let Some(rest) = raw.strip_prefix('"') {
        let mut escaped = false;
        for (index, character) in rest.char_indices() {
            if character == '"' && !escaped {
                return rest[..index].to_owned();
            }
            escaped = character == '\\' && !escaped;
            if character != '\\' {
                escaped = false;
            }
        }
    }
    raw.split(|character: char| character.is_whitespace() || character == '#')
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::unique_temp_dir;

    fn profile(path: PathBuf, kind: ShellKind) -> ShellProfile {
        ShellProfile { path, kind }
    }

    #[test]
    fn detect_returns_a_usable_profile() {
        let profile = detect();
        assert!(!profile.path.as_os_str().is_empty());
        assert!(!profile.kind.label().is_empty());
    }

    #[test]
    fn reads_posix_and_detects_unmanaged_lines() {
        let text = r#"
export PATH="$HOME/bin:$PATH"
export RUSTUP_DIST_SERVER="https://rsproxy.cn"
export RUSTUP_UPDATE_ROOT='https://rsproxy.cn/rustup/'
"#;
        let values = parse(text, ShellKind::Zsh);
        assert_eq!(values.dist.as_deref(), Some("https://rsproxy.cn"));
        assert_eq!(values.update.as_deref(), Some("https://rsproxy.cn/rustup/"));
        assert!(values.unmanaged);
    }

    #[test]
    fn applies_and_removes_managed_block() {
        let dir = unique_temp_dir("shell-zsh");
        let path = dir.join(".zshrc");
        std::fs::write(&path, "export PATH=\"$HOME/bin:$PATH\"\n").unwrap();
        let profile = profile(path.clone(), ShellKind::Zsh);

        let warnings = apply(
            &profile,
            Some("https://rsproxy.cn"),
            Some("https://rsproxy.cn/rustup"),
        )
        .unwrap();
        assert!(warnings.is_empty());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(BLOCK_START));
        assert!(text.contains("RUSTUP_DIST_SERVER"));
        assert!(text.contains("PATH="));
        let values = read(&profile).unwrap();
        assert_eq!(values.dist.as_deref(), Some("https://rsproxy.cn"));
        assert!(!values.unmanaged);

        apply(&profile, None, None).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains(BLOCK_START));
        assert!(!text.contains("RUSTUP_DIST_SERVER"));
        assert!(text.contains("PATH="));
        assert!(read(&profile).unwrap().dist.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn managed_block_wins_and_is_not_duplicated() {
        let dir = unique_temp_dir("shell-replace");
        let path = dir.join(".zshrc");
        std::fs::write(&path, "export RUSTUP_DIST_SERVER='https://old.example'\n").unwrap();
        let profile = profile(path.clone(), ShellKind::Zsh);
        apply(
            &profile,
            Some("https://new.example"),
            Some("https://new.example/rustup"),
        )
        .unwrap();
        apply(
            &profile,
            Some("https://newer.example"),
            Some("https://newer.example/rustup"),
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches(BLOCK_START).count(), 1);
        let values = read(&profile).unwrap();
        assert_eq!(values.dist.as_deref(), Some("https://newer.example"));
        assert!(values.unmanaged);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fish_syntax_roundtrip() {
        let dir = unique_temp_dir("shell-fish");
        let path = dir.join("config.fish");
        let profile = profile(path.clone(), ShellKind::Fish);
        apply(
            &profile,
            Some("https://mirrors.ustc.edu.cn/rust-static"),
            Some("https://mirrors.ustc.edu.cn/rust-static/rustup"),
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("set -gx RUSTUP_DIST_SERVER"));
        let values = read(&profile).unwrap();
        assert_eq!(
            values.dist.as_deref(),
            Some("https://mirrors.ustc.edu.cn/rust-static")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn official_on_missing_file_does_not_create_it() {
        let dir = unique_temp_dir("shell-missing");
        let path = dir.join(".zshrc");
        let profile = profile(path.clone(), ShellKind::Zsh);
        apply(&profile, None, None).unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_newlines() {
        let dir = unique_temp_dir("shell-newline");
        let path = dir.join(".zshrc");
        let profile = profile(path, ShellKind::Zsh);
        assert!(apply(&profile, Some("a\nb"), None).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
