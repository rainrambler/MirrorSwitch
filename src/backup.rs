//! Snapshot and restore the configuration owned by this application:
//! Cargo's config file and the two Rustup mirror environment variables.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::cargo;
#[cfg(test)]
use crate::catalog::mirror_for_cargo;
use crate::catalog::{mirror_by_id, mirror_for_rustup};
use crate::paths::AppPaths;
use crate::platform;
use crate::util::{
    atomic_write, epoch_millis, fingerprint, format_local, now_rfc3339, read_optional, today_stamp,
};

pub const SNAPSHOT_SCHEMA: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotKind {
    Auto,
    Manual,
    PreRestore,
}

impl SnapshotKind {
    pub fn is_automatic(self) -> bool {
        matches!(self, Self::Auto | Self::PreRestore)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotCargo {
    pub path: String,
    pub existed: bool,
    pub content: Option<String>,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotRustup {
    pub dist_user: Option<String>,
    pub update_user: Option<String>,
    pub dist_effective: Option<String>,
    pub update_effective: Option<String>,
    pub label: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SnapshotShell {
    pub path: String,
    pub existed: bool,
    pub content: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema: u32,
    pub id: String,
    pub created_at: String,
    pub created_at_ms: i64,
    pub kind: SnapshotKind,
    pub note: String,
    pub cargo: SnapshotCargo,
    pub rustup: SnapshotRustup,
    #[serde(default)]
    pub shell: Option<SnapshotShell>,
    pub fingerprint: String,
    pub app_version: String,
}

impl Snapshot {
    pub fn cargo_badge(&self) -> &'static str {
        match self.kind {
            SnapshotKind::Auto => "自动",
            SnapshotKind::Manual => "手动",
            SnapshotKind::PreRestore => "恢复前",
        }
    }

    pub fn display_time(&self) -> String {
        format_local(self.created_at_ms)
    }

    pub fn cargo_bytes(&self) -> usize {
        self.cargo
            .content
            .as_ref()
            .map(|content| content.len())
            .unwrap_or(0)
    }
}

#[derive(Clone, Debug)]
pub struct CreateOutcome {
    pub snapshot: Snapshot,
    pub created: bool,
}

#[derive(Clone, Debug, Default)]
pub struct RestoreOutcome {
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

impl RestoreOutcome {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

pub fn capture(paths: &AppPaths, kind: SnapshotKind, note: &str) -> Result<Snapshot> {
    let exists = paths.cargo_config.exists();
    let content = read_optional(&paths.cargo_config)?;
    let inspection = cargo::inspect(&paths.cargo_config);
    let cargo_label = if inspection.parse_error.is_some() {
        "配置无法解析".to_owned()
    } else if let Some(id) = inspection.mirror_id {
        mirror_by_id(id)
            .map(|mirror| mirror.name.to_owned())
            .unwrap_or_else(|| "预设镜像".to_owned())
    } else if let Some(source) = inspection.replace_with.as_deref() {
        format!("自定义 · {source}")
    } else {
        "官方 crates.io".to_owned()
    };

    let env = platform::read_rustup_env();
    let rustup_label =
        if let Some(mirror) = env.dist.effective.as_deref().and_then(mirror_for_rustup) {
            mirror.name.to_owned()
        } else if env.is_official() {
            "官方源".to_owned()
        } else {
            "自定义源".to_owned()
        };

    let shell = match platform::shell_profile_path() {
        Some(path) => {
            let existed = path.exists();
            let content = read_optional(&path)?;
            Some(SnapshotShell {
                path: path.display().to_string(),
                existed,
                content,
            })
        }
        None => None,
    };

    let cargo_fingerprint = content.as_deref().unwrap_or("<missing>");
    let shell_path = shell
        .as_ref()
        .map(|shell| shell.path.as_str())
        .unwrap_or("<no-shell>");
    let shell_fingerprint = shell
        .as_ref()
        .and_then(|shell| shell.content.as_deref())
        .unwrap_or("<missing>");
    let fp = fingerprint(&[
        paths.cargo_config.to_string_lossy().as_ref(),
        cargo_fingerprint,
        env.dist.user.as_deref().unwrap_or("<unset>"),
        env.update.user.as_deref().unwrap_or("<unset>"),
        shell_path,
        shell_fingerprint,
    ]);

    Ok(Snapshot {
        schema: SNAPSHOT_SCHEMA,
        id: today_stamp(),
        created_at: now_rfc3339(),
        created_at_ms: epoch_millis(),
        kind,
        note: note.to_owned(),
        cargo: SnapshotCargo {
            path: paths.cargo_config.display().to_string(),
            existed: exists,
            content,
            label: cargo_label,
        },
        rustup: SnapshotRustup {
            dist_user: env.dist.user.clone(),
            update_user: env.update.user.clone(),
            dist_effective: env.dist.effective.clone(),
            update_effective: env.update.effective.clone(),
            label: rustup_label,
        },
        shell,
        fingerprint: fp,
        app_version: crate::paths::APP_VERSION.to_owned(),
    })
}

/// Create a snapshot. Automatic snapshots are skipped when the current state
/// is byte-for-byte identical to the latest snapshot.
pub fn create_snapshot(paths: &AppPaths, kind: SnapshotKind, note: &str) -> Result<CreateOutcome> {
    let mut snapshot = capture(paths, kind, note)?;
    let (existing, _) = load_all(paths);
    if kind.is_automatic() {
        if let Some(latest) = existing.first() {
            if latest.fingerprint == snapshot.fingerprint {
                return Ok(CreateOutcome {
                    snapshot: latest.clone(),
                    created: false,
                });
            }
        }
    }

    fs::create_dir_all(&paths.backup_dir)
        .with_context(|| format!("无法创建备份目录 {}", paths.backup_dir.display()))?;
    let mut file = paths.backup_dir.join(format!("{}.json", snapshot.id));
    let mut suffix = 1;
    while file.exists() {
        file = paths
            .backup_dir
            .join(format!("{}-{suffix}.json", snapshot.id));
        suffix += 1;
        if suffix > 100 {
            bail!("无法生成唯一的备份文件名");
        }
    }
    if let Some(name) = file.file_stem().and_then(|s| s.to_str()) {
        snapshot.id = name.to_owned();
    }
    let json = serde_json::to_string_pretty(&snapshot)?;
    atomic_write(&file, json.as_bytes())?;
    Ok(CreateOutcome {
        snapshot,
        created: true,
    })
}

pub fn load_all(paths: &AppPaths) -> (Vec<Snapshot>, Vec<String>) {
    let mut snapshots = Vec::new();
    let mut errors = Vec::new();
    let entries = match fs::read_dir(&paths.backup_dir) {
        Ok(entries) => entries,
        Err(_) => return (snapshots, errors),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        match read_snapshot(&path) {
            Ok(snapshot) if snapshot.schema <= SNAPSHOT_SCHEMA => snapshots.push(snapshot),
            Ok(snapshot) => errors.push(format!(
                "备份 {} 使用较新的格式（schema {}），已跳过",
                path.display(),
                snapshot.schema
            )),
            Err(error) => errors.push(format!("无法读取备份 {}：{error}", path.display())),
        }
    }
    snapshots.sort_by_key(|snapshot| std::cmp::Reverse(snapshot.created_at_ms));
    (snapshots, errors)
}

fn read_snapshot(path: &Path) -> Result<Snapshot> {
    let text = fs::read_to_string(path)?;
    let snapshot: Snapshot = serde_json::from_str(&text)?;
    Ok(snapshot)
}

pub fn needs_auto_backup(paths: &AppPaths) -> Result<bool> {
    let current = capture(paths, SnapshotKind::Auto, "探测")?;
    let (existing, _) = load_all(paths);
    Ok(existing
        .first()
        .map(|latest| latest.fingerprint != current.fingerprint)
        .unwrap_or(true))
}

pub fn prune_auto(paths: &AppPaths, keep: usize) -> Vec<String> {
    let (snapshots, _) = load_all(paths);
    let mut removed = Vec::new();
    let mut automatic_seen = 0usize;
    for snapshot in snapshots {
        if !snapshot.kind.is_automatic() {
            continue;
        }
        automatic_seen += 1;
        if automatic_seen <= keep.max(1) {
            continue;
        }
        let path = paths.backup_dir.join(format!("{}.json", snapshot.id));
        if fs::remove_file(&path).is_ok() {
            removed.push(snapshot.id);
        }
    }
    removed
}

pub fn delete(paths: &AppPaths, id: &str) -> Result<()> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        bail!("备份标识不合法");
    }
    let path = paths.backup_dir.join(format!("{id}.json"));
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("无法删除 {}", path.display()))?;
    }
    Ok(())
}

/// Restore a snapshot. Cargo is restored to the *current* resolved path rather
/// than a path recorded on another machine/session; the UI warns if they differ.
pub fn restore(paths: &AppPaths, snapshot: &Snapshot) -> RestoreOutcome {
    let mut outcome = RestoreOutcome::default();
    if snapshot.schema > SNAPSHOT_SCHEMA {
        outcome.errors.push("备份格式版本高于当前程序".to_owned());
        return outcome;
    }
    let recorded_path = PathBuf::from(&snapshot.cargo.path);
    if recorded_path != paths.cargo_config {
        outcome.warnings.push(format!(
            "备份中的 Cargo 路径为 {}，当前路径为 {}；将恢复到当前路径。",
            snapshot.cargo.path,
            paths.cargo_config.display()
        ));
    }

    match restore_cargo(paths, snapshot) {
        Ok(()) => {}
        Err(error) => outcome.errors.push(error.to_string()),
    }

    let mut shell_restored = false;
    let current_shell = platform::shell_profile_path();
    if let Some(shell) = &snapshot.shell {
        match &current_shell {
            Some(current) => {
                if Path::new(&shell.path) != current.as_path() {
                    outcome.warnings.push(format!(
                        "备份中的 Shell 配置路径为 {}，当前为 {}；将恢复到当前路径。",
                        shell.path,
                        current.display()
                    ));
                }
                match restore_shell(current, shell) {
                    Ok(()) => shell_restored = true,
                    Err(error) => outcome
                        .errors
                        .push(format!("恢复 Shell 配置失败：{error:#}")),
                }
            }
            None => outcome.warnings.push(
                "此备份包含 macOS/Linux Shell 配置，但当前系统不管理 Shell 文件；将仅尝试恢复 Rustup 环境变量。"
                    .to_owned(),
            ),
        }
    }

    if shell_restored {
        outcome
            .warnings
            .extend(platform::sync_session_env_from_profile());
    } else if snapshot.shell.is_none() || current_shell.is_none() {
        match platform::set_rustup_env(
            snapshot.rustup.dist_user.as_deref(),
            snapshot.rustup.update_user.as_deref(),
        ) {
            Ok(warnings) => outcome.warnings.extend(warnings),
            Err(error) => outcome
                .errors
                .push(format!("恢复 Rustup 环境变量失败：{error}")),
        }
    }
    outcome
}

pub fn restore_shell(path: &Path, shell: &SnapshotShell) -> Result<()> {
    if shell.existed {
        let content = shell
            .content
            .as_deref()
            .context("备份标记为存在 Shell 配置，但未包含内容")?;
        atomic_write(path, content.as_bytes())
    } else if path.exists() {
        fs::remove_file(path).with_context(|| format!("无法删除 Shell 配置 {}", path.display()))
    } else {
        Ok(())
    }
}

pub fn restore_cargo(paths: &AppPaths, snapshot: &Snapshot) -> Result<()> {
    if snapshot.cargo.existed {
        let content = snapshot
            .cargo
            .content
            .as_deref()
            .context("备份标记为存在配置文件，但未包含内容")?;
        atomic_write(&paths.cargo_config, content.as_bytes())
    } else if paths.cargo_config.exists() {
        fs::remove_file(&paths.cargo_config)
            .with_context(|| format!("无法删除 {}", paths.cargo_config.display()))
    } else {
        Ok(())
    }
}

pub fn backup_dir_size(paths: &AppPaths) -> u64 {
    fs::read_dir(&paths.backup_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.metadata().ok())
                .map(|meta| meta.len())
                .sum()
        })
        .unwrap_or(0)
}

pub fn snapshot_file(paths: &AppPaths, id: &str) -> PathBuf {
    paths.backup_dir.join(format!("{id}.json"))
}

/// Human readable summary for backup cards.
pub fn summary(snapshot: &Snapshot) -> String {
    format!(
        "Cargo · {}   /   Rustup · {}",
        snapshot.cargo.label, snapshot.rustup.label
    )
}

#[cfg(test)]
pub fn mirror_name_for_snapshot(snapshot: &Snapshot) -> Option<&'static str> {
    snapshot
        .cargo
        .content
        .as_deref()
        .and_then(|content| {
            // Lightweight parse: find the managed source name in a temp-free way.
            // The stored label is the primary display; this helper is only used
            // when we need a stable id.
            let doc = content.parse::<toml_edit::DocumentMut>().ok()?;
            let registry = doc
                .get("source")?
                .as_table()?
                .get("crates-io")?
                .as_table()?
                .get("replace-with")?
                .as_str()?;
            let url = doc
                .get("source")?
                .as_table()?
                .get(registry)?
                .as_table()?
                .get("registry")?
                .as_str()?;
            mirror_for_cargo(url)
        })
        .map(|mirror| mirror.id)
        .or_else(|| {
            snapshot
                .rustup
                .dist_effective
                .as_deref()
                .and_then(mirror_for_rustup)
                .map(|mirror| mirror.id)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::mirror_by_id;
    use crate::util::unique_temp_dir;

    fn test_paths() -> AppPaths {
        let root = unique_temp_dir("backup");
        let cargo_home = root.join("cargo");
        std::fs::create_dir_all(&cargo_home).unwrap();
        AppPaths::with_roots(root.join("data"), cargo_home, root.join("rustup"))
    }

    #[test]
    fn snapshot_roundtrip_and_cargo_restore() {
        let paths = test_paths();
        paths.ensure_dirs().unwrap();
        std::fs::write(&paths.cargo_config, "[net]\ngit-fetch-with-cli = true\n").unwrap();
        let outcome = create_snapshot(&paths, SnapshotKind::Manual, "test").unwrap();
        assert!(outcome.created);
        assert_eq!(outcome.snapshot.cargo.label, "官方 crates.io");

        std::fs::write(&paths.cargo_config, "[net]\noffline = true\n").unwrap();
        let (snapshots, errors) = load_all(&paths);
        assert!(errors.is_empty());
        assert_eq!(snapshots.len(), 1);
        restore_cargo(&paths, &snapshots[0]).unwrap();
        assert_eq!(
            std::fs::read_to_string(&paths.cargo_config).unwrap(),
            "[net]\ngit-fetch-with-cli = true\n"
        );
        let _ = std::fs::remove_dir_all(paths.data_dir.parent().unwrap());
    }

    #[test]
    fn automatic_snapshot_is_deduplicated() {
        let paths = test_paths();
        paths.ensure_dirs().unwrap();
        std::fs::write(&paths.cargo_config, "[net]\n").unwrap();
        let first = create_snapshot(&paths, SnapshotKind::Auto, "one").unwrap();
        assert!(first.created);
        let second = create_snapshot(&paths, SnapshotKind::Auto, "two").unwrap();
        assert!(!second.created);
        assert_eq!(first.snapshot.id, second.snapshot.id);
        let _ = std::fs::remove_dir_all(paths.data_dir.parent().unwrap());
    }

    #[test]
    fn restore_deletes_config_when_snapshot_had_none() {
        let paths = test_paths();
        paths.ensure_dirs().unwrap();
        let snapshot = capture(&paths, SnapshotKind::Manual, "none").unwrap();
        std::fs::write(&paths.cargo_config, "[net]\n").unwrap();
        restore_cargo(&paths, &snapshot).unwrap();
        assert!(!paths.cargo_config.exists());
        let _ = std::fs::remove_dir_all(paths.data_dir.parent().unwrap());
    }

    #[test]
    fn prune_keeps_manual_snapshots() {
        let paths = test_paths();
        paths.ensure_dirs().unwrap();
        std::fs::write(&paths.cargo_config, "a = 1\n").unwrap();
        create_snapshot(&paths, SnapshotKind::Auto, "a").unwrap();
        std::fs::write(&paths.cargo_config, "a = 2\n").unwrap();
        create_snapshot(&paths, SnapshotKind::Auto, "b").unwrap();
        create_snapshot(&paths, SnapshotKind::Manual, "keep").unwrap();
        prune_auto(&paths, 1);
        let (snapshots, _) = load_all(&paths);
        assert_eq!(
            snapshots
                .iter()
                .filter(|s| s.kind == SnapshotKind::Manual)
                .count(),
            1
        );
        assert_eq!(
            snapshots
                .iter()
                .filter(|s| s.kind == SnapshotKind::Auto)
                .count(),
            1
        );
        let _ = std::fs::remove_dir_all(paths.data_dir.parent().unwrap());
    }

    #[test]
    fn v1_snapshot_without_shell_field_is_still_readable() {
        let paths = test_paths();
        paths.ensure_dirs().unwrap();
        let snapshot = capture(&paths, SnapshotKind::Manual, "v1").unwrap();
        let mut value = serde_json::to_value(snapshot).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("shell");
        object.insert("schema".to_owned(), serde_json::json!(1));
        let parsed: Snapshot = serde_json::from_value(value).unwrap();
        assert!(parsed.shell.is_none());
        assert_eq!(parsed.schema, 1);
        let _ = std::fs::remove_dir_all(paths.data_dir.parent().unwrap());
    }

    #[test]
    fn restore_shell_file_exactly_or_removes_it() {
        let dir = unique_temp_dir("backup-shell");
        let path = dir.join(".zshrc");
        std::fs::write(&path, "current\n").unwrap();
        let shell = SnapshotShell {
            path: path.display().to_string(),
            existed: true,
            content: Some("snapshot\n".to_owned()),
        };
        restore_shell(&path, &shell).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "snapshot\n");

        let absent = SnapshotShell {
            path: path.display().to_string(),
            existed: false,
            content: None,
        };
        restore_shell(&path, &absent).unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn mirror_name_helper_reads_snapshot_content() {
        let paths = test_paths();
        paths.ensure_dirs().unwrap();
        std::fs::write(&paths.cargo_config, "").unwrap();
        let mirror = mirror_by_id("rsproxy").unwrap();
        crate::cargo::apply(&paths.cargo_config, Some(mirror)).unwrap();
        let snapshot = capture(&paths, SnapshotKind::Manual, "rsproxy").unwrap();
        assert_eq!(mirror_name_for_snapshot(&snapshot), Some("rsproxy"));
        let _ = std::fs::remove_dir_all(paths.data_dir.parent().unwrap());
    }
}
