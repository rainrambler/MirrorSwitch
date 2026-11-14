//! Safe, comment-preserving edits to Cargo's `config.toml`.
//!
//! Only the managed `source` / `registries` entries are changed. Everything
//! else in the file is left untouched, and invalid TOML is never overwritten.

use std::path::Path;

use anyhow::{Context, Result};
use toml_edit::{table, value, DocumentMut, Item, Table};

use crate::catalog::{mirror_for_cargo, Mirror};
use crate::util::{atomic_write, read_optional};

pub const MANAGED_PREFIX: &str = "mirrorswitch-";

#[derive(Clone, Debug, Default)]
pub struct CargoInspection {
    pub exists: bool,
    pub parse_error: Option<String>,
    pub replace_with: Option<String>,
    pub registry: Option<String>,
    pub mirror_id: Option<&'static str>,
}

impl CargoInspection {
    pub fn is_official(&self) -> bool {
        self.replace_with.is_none()
    }

    pub fn is_custom(&self) -> bool {
        self.replace_with.is_some() && self.mirror_id.is_none()
    }
}

pub fn inspect(path: &Path) -> CargoInspection {
    let raw = match read_optional(path) {
        Ok(raw) => raw,
        Err(error) => {
            return CargoInspection {
                exists: path.exists(),
                parse_error: Some(error.to_string()),
                ..Default::default()
            };
        }
    };
    let exists = raw.is_some();
    let Some(text) = raw else {
        return CargoInspection {
            exists: false,
            ..Default::default()
        };
    };
    let doc = match text.parse::<DocumentMut>() {
        Ok(doc) => doc,
        Err(error) => {
            return CargoInspection {
                exists,
                parse_error: Some(error.to_string()),
                ..Default::default()
            };
        }
    };
    let replace_with = get_str(&doc, &["source", "crates-io", "replace-with"]);
    let registry = replace_with
        .as_deref()
        .and_then(|name| get_str(&doc, &["source", name, "registry"]));
    let mirror_id = registry
        .as_deref()
        .and_then(mirror_for_cargo)
        .map(|mirror| mirror.id);
    CargoInspection {
        exists,
        parse_error: None,
        replace_with,
        registry,
        mirror_id,
    }
}

#[derive(Clone, Debug)]
pub struct ApplyReport {
    pub changed: bool,
}

/// Apply a preset mirror, or `None` to restore the official crates.io source.
pub fn apply(path: &Path, mirror: Option<&Mirror>) -> Result<ApplyReport> {
    let old = read_optional(path)?;
    let old_existed = old.is_some();
    let text = old.clone().unwrap_or_default();
    let mut doc = if text.trim().is_empty() {
        DocumentMut::new()
    } else {
        text.parse::<DocumentMut>()
            .context("现有 Cargo 配置不是合法的 TOML，已取消写入（原文件未被修改）")?
    };

    match mirror {
        Some(mirror) => {
            let endpoint = mirror.cargo.context("该镜像没有提供 Cargo 端点")?;
            let managed = managed_key(mirror.id);

            let source = ensure_table(&mut doc, "source")?;
            let crates_io = ensure_subtable(source, "crates-io")?;
            crates_io.insert("replace-with", value(managed.as_str()));

            let source = ensure_table(&mut doc, "source")?;
            let managed_source = ensure_subtable(source, &managed)?;
            managed_source.insert("registry", value(endpoint.index));

            let registries = ensure_table(&mut doc, "registries")?;
            let named = ensure_subtable(registries, &managed)?;
            named.insert("index", value(endpoint.index));
        }
        None => {
            if let Some(source) = doc.get_mut("source").and_then(Item::as_table_mut) {
                if let Some(crates_io) = source.get_mut("crates-io").and_then(Item::as_table_mut) {
                    crates_io.remove("replace-with");
                }
                remove_managed_entries(source);
            }
            if let Some(registries) = doc.get_mut("registries").and_then(Item::as_table_mut) {
                remove_managed_entries(registries);
            }
        }
    }

    let mut new_content = doc.to_string();
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    // Refuse to write anything that would not parse back cleanly.
    new_content
        .parse::<DocumentMut>()
        .context("生成后的 Cargo 配置校验失败，已取消写入")?;

    let changed = old.as_deref() != Some(new_content.as_str());
    if changed {
        if mirror.is_none() && !old_existed && new_content.trim().is_empty() {
            return Ok(ApplyReport { changed: false });
        }
        atomic_write(path, new_content.as_bytes())?;
    }
    Ok(ApplyReport { changed })
}

pub fn managed_key(id: &str) -> String {
    format!("{MANAGED_PREFIX}{id}")
}

fn remove_managed_entries(table: &mut Table) {
    let keys: Vec<String> = table
        .iter()
        .filter(|(key, _)| key.starts_with(MANAGED_PREFIX))
        .map(|(key, _)| key.to_owned())
        .collect();
    for key in keys {
        table.remove(&key);
    }
}

fn ensure_table<'a>(doc: &'a mut DocumentMut, key: &str) -> Result<&'a mut Table> {
    if !doc.as_table().contains_key(key) {
        doc.as_table_mut().insert(key, table());
    }
    doc.get_mut(key)
        .and_then(Item::as_table_mut)
        .with_context(|| format!("配置中的 [{key}] 不是表，无法安全修改"))
}

fn ensure_subtable<'a>(parent: &'a mut Table, key: &str) -> Result<&'a mut Table> {
    if !parent.contains_key(key) {
        parent.insert(key, table());
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_mut)
        .with_context(|| format!("配置中的 [{key}] 不是表，无法安全修改"))
}

fn get_str(doc: &DocumentMut, path: &[&str]) -> Option<String> {
    let mut item: &Item = doc.as_item();
    for key in path {
        item = item.as_table()?.get(key)?;
    }
    item.as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::mirror_by_id;
    use crate::util::unique_temp_dir;
    use std::fs;

    #[test]
    fn applying_preserves_unrelated_configuration() {
        let dir = unique_temp_dir("cargo-preserve");
        let path = dir.join("config.toml");
        fs::write(
            &path,
            r#"[net]
git-fetch-with-cli = true

[build]
jobs = 8

[source.my-private]
registry = "sparse+https://example.com/index/"
"#,
        )
        .unwrap();

        let mirror = mirror_by_id("rsproxy").unwrap();
        let report = apply(&path, Some(mirror)).unwrap();
        assert!(report.changed);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("git-fetch-with-cli = true"));
        assert!(text.contains("jobs = 8"));
        assert!(text.contains("my-private"));
        assert!(text.contains("replace-with = \"mirrorswitch-rsproxy\""));
        assert!(text.contains("https://rsproxy.cn/index/"));

        let info = inspect(&path);
        assert_eq!(info.mirror_id, Some("rsproxy"));
        assert!(!info.is_custom());
    }

    #[test]
    fn official_reset_removes_only_managed_entries() {
        let dir = unique_temp_dir("cargo-reset");
        let path = dir.join("config.toml");
        let mirror = mirror_by_id("aliyun").unwrap();
        apply(&path, Some(mirror)).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("crates-io"));

        apply(&path, None).unwrap();
        let info = inspect(&path);
        assert!(info.is_official());
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("mirrorswitch-"));
        assert!(!text.contains("replace-with"));
    }

    #[test]
    fn invalid_toml_is_never_overwritten() {
        let dir = unique_temp_dir("cargo-invalid");
        let path = dir.join("config.toml");
        let original = "[source.crates-io\nreplace-with = ";
        fs::write(&path, original).unwrap();
        let mirror = mirror_by_id("tuna").unwrap();
        let error = apply(&path, Some(mirror)).unwrap_err().to_string();
        assert!(error.contains("不是合法的 TOML"));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn official_on_missing_file_does_not_create_one() {
        let dir = unique_temp_dir("cargo-missing");
        let path = dir.join("config.toml");
        let report = apply(&path, None).unwrap();
        assert!(!report.changed);
        assert!(!path.exists());
    }
}
