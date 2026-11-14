use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use chrono::{Local, TimeZone};

/// Write a file through a sibling temp file so a failed write never leaves a
/// half-written configuration behind. The caller keeps a snapshot separately.
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    // Dotfile managers commonly symlink shell profiles. Follow the link and
    // replace the target instead of replacing the symlink itself.
    let target = if fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
    {
        fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    } else {
        path.to_path_buf()
    };
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).with_context(|| format!("无法创建目录 {}", parent.display()))?;
    }
    let permissions = fs::metadata(&target).ok().map(|meta| meta.permissions());
    let file_name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("config");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = target.with_file_name(format!(".{file_name}.{stamp}.tmp"));
    fs::write(&temp, contents).with_context(|| format!("无法写入临时文件 {}", temp.display()))?;
    if let Ok(file) = fs::File::open(&temp) {
        let _ = file.sync_all();
    }
    if target.exists() {
        fs::remove_file(&target).with_context(|| format!("无法替换旧文件 {}", target.display()))?;
    }
    fs::rename(&temp, &target).with_context(|| {
        let _ = fs::remove_file(&temp);
        format!("无法保存文件 {}", target.display())
    })?;
    if let Some(permissions) = permissions {
        let _ = fs::set_permissions(&target, permissions);
    }
    Ok(())
}

pub fn read_optional(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).with_context(|| format!("无法读取 {}", path.display()))?;
    // Cargo config files must be UTF-8. Use lossy conversion for display and
    // backups so an unusual encoding still gets a recoverable backup.
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

pub fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GB", b / GIB)
    } else if b >= MIB {
        format!("{:.2} MB", b / MIB)
    } else if b >= KIB {
        format!("{:.1} KB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

pub fn format_speed(bytes_per_sec: f64) -> String {
    if !bytes_per_sec.is_finite() || bytes_per_sec <= 0.0 {
        return "—".to_owned();
    }
    let mib = bytes_per_sec / (1024.0 * 1024.0);
    if mib >= 1.0 {
        format!("{mib:.2} MB/s")
    } else {
        format!("{:.0} KB/s", bytes_per_sec / 1024.0)
    }
}

pub fn format_ms(ms: f64) -> String {
    if !ms.is_finite() || ms <= 0.0 {
        return "—".to_owned();
    }
    if ms >= 1000.0 {
        format!("{:.2} s", ms / 1000.0)
    } else {
        format!("{ms:.0} ms")
    }
}

pub fn now_string() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn now_rfc3339() -> String {
    Local::now().to_rfc3339()
}

pub fn today_stamp() -> String {
    Local::now().format("%Y%m%d-%H%M%S-%3f").to_string()
}

pub fn epoch_millis() -> i64 {
    Local::now().timestamp_millis()
}

pub fn relative_time(epoch_ms: i64) -> String {
    let now = epoch_millis();
    let diff = (now - epoch_ms).max(0);
    if diff < 60_000 {
        "刚刚".to_owned()
    } else if diff < 3_600_000 {
        format!("{} 分钟前", diff / 60_000)
    } else if diff < 86_400_000 {
        format!("{} 小时前", diff / 3_600_000)
    } else if diff < 7 * 86_400_000 {
        format!("{} 天前", diff / 86_400_000)
    } else if let Some(dt) = Local.timestamp_millis_opt(epoch_ms).single() {
        dt.format("%Y-%m-%d").to_string()
    } else {
        "—".to_owned()
    }
}

pub fn format_local(epoch_ms: i64) -> String {
    Local
        .timestamp_millis_opt(epoch_ms)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "—".to_owned())
}

pub fn ellipsize(input: &str, max_chars: usize) -> String {
    let count = input.chars().count();
    if count <= max_chars {
        return input.to_owned();
    }
    if max_chars <= 1 {
        return "…".to_owned();
    }
    let head: String = input.chars().take(max_chars - 1).collect();
    format!("{head}…")
}

pub fn short_path(path: &Path, max_chars: usize) -> String {
    ellipsize(&path.display().to_string(), max_chars)
}

/// A stable, non-cryptographic fingerprint used only to avoid duplicate
/// automatic snapshots. It is not a security primitive.
pub fn fingerprint(parts: &[&str]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
        0xffu8.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

#[cfg(windows)]
fn hidden_command(program: &str) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(windows)]
pub fn reveal_path(path: &Path) -> Result<()> {
    let target = if path.is_file() {
        format!("/select,{}", path.display())
    } else {
        path.display().to_string()
    };
    hidden_command("explorer.exe")
        .arg(target)
        .spawn()
        .with_context(|| "无法打开资源管理器")?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn reveal_path(path: &Path) -> Result<()> {
    let mut command = Command::new("open");
    if path.is_file() {
        command.arg("-R").arg(path);
    } else {
        command.arg(path);
    }
    command.spawn().with_context(|| "无法打开 Finder")?;
    Ok(())
}

#[cfg(all(not(windows), not(target_os = "macos")))]
pub fn reveal_path(path: &Path) -> Result<()> {
    Command::new("xdg-open")
        .arg(path)
        .spawn()
        .with_context(|| "无法打开文件管理器")?;
    Ok(())
}

#[cfg(windows)]
pub fn open_url(url: &str) -> Result<()> {
    hidden_command("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn()
        .with_context(|| "无法打开浏览器")?;
    Ok(())
}

#[cfg(not(windows))]
pub fn open_url(url: &str) -> Result<()> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(program)
        .arg(url)
        .spawn()
        .with_context(|| "无法打开浏览器")?;
    Ok(())
}

/// Return a unique temp directory for tests without adding a dependency.
#[cfg(test)]
pub fn unique_temp_dir(tag: &str) -> PathBuf {
    let name = format!(
        "mirrorswitch-test-{tag}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let workspace_base = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("target")
        .join("test-tmp");
    let candidates = [std::env::temp_dir().join(&name), workspace_base.join(&name)];
    for path in candidates {
        let _ = fs::remove_dir_all(&path);
        if fs::create_dir_all(&path).is_ok() {
            return path;
        }
    }
    panic!("unable to create a temporary directory for tests");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_and_speed_formatting_is_readable() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(2 * 1024 * 1024), "2.00 MB");
        assert_eq!(format_speed(5.5 * 1024.0 * 1024.0), "5.50 MB/s");
        assert_eq!(format_speed(512.0 * 1024.0), "512 KB/s");
        assert_eq!(format_ms(42.2), "42 ms");
    }

    #[test]
    fn ellipsize_keeps_a_single_ellipsis() {
        assert_eq!(ellipsize("abcdef", 4), "abc…");
        assert_eq!(ellipsize("abc", 4), "abc");
        assert_eq!(ellipsize("abc", 1), "…");
    }

    #[test]
    fn atomic_write_replaces_content() {
        let dir = unique_temp_dir("write");
        let path = dir.join("nested").join("config.toml");
        atomic_write(&path, b"one").unwrap();
        atomic_write(&path, b"two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fingerprints_change_with_input() {
        assert_ne!(fingerprint(&["a", "b"]), fingerprint(&["a", "c"]));
        assert_eq!(fingerprint(&["a", "b"]), fingerprint(&["a", "b"]));
    }
}
