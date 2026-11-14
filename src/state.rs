use crate::cargo::{self, CargoInspection};
use crate::catalog::{mirror_by_id, mirror_for_rustup, mirror_for_rustup_update, MirrorKind};
use crate::paths::AppPaths;
use crate::platform::{self, RustupEnvState};

#[derive(Clone, Debug)]
pub struct ActiveState {
    pub cargo: CargoInspection,
    pub rustup: RustupEnvState,
    pub cargo_mirror_id: Option<&'static str>,
    pub rustup_mirror_id: Option<&'static str>,
    pub cargo_label: String,
    pub rustup_label: String,
    pub warnings: Vec<String>,
}

impl ActiveState {
    pub fn read(paths: &AppPaths) -> Self {
        let cargo = cargo::inspect(&paths.cargo_config);
        let rustup = platform::read_rustup_env();
        let (cargo_mirror_id, cargo_label) = cargo_summary(&cargo);
        let (rustup_mirror_id, rustup_label, mut warnings) = rustup_summary(&rustup);
        warnings.extend(paths.path_warnings());
        if let Some(error) = &cargo.parse_error {
            warnings.push(format!(
                "Cargo 配置文件无法解析：{error}。为避免破坏配置，切换功能已禁用，请先手动修复。"
            ));
        }
        Self {
            cargo,
            rustup,
            cargo_mirror_id,
            rustup_mirror_id,
            cargo_label,
            rustup_label,
            warnings,
        }
    }

    pub fn mirror_active_for(&self, id: &str, kind: MirrorKind) -> bool {
        match kind {
            MirrorKind::Cargo => self.cargo_mirror_id == Some(id),
            MirrorKind::Rustup => self.rustup_mirror_id == Some(id),
        }
    }

    pub fn cargo_endpoint(&self) -> String {
        self.cargo
            .registry
            .clone()
            .unwrap_or_else(|| "https://index.crates.io/".to_owned())
    }

    pub fn rustup_endpoint(&self) -> String {
        self.rustup
            .dist
            .effective
            .clone()
            .unwrap_or_else(|| "https://static.rust-lang.org".to_owned())
    }

    pub fn rustup_has_system_override(&self) -> bool {
        self.rustup.has_system_override()
    }
}

fn cargo_summary(cargo: &CargoInspection) -> (Option<&'static str>, String) {
    if cargo.parse_error.is_some() {
        return (None, "配置无法解析".to_owned());
    }
    if !cargo.exists {
        return (None, "未配置（官方）".to_owned());
    }
    if let Some(id) = cargo.mirror_id {
        let name = mirror_by_id(id)
            .map(|mirror| mirror.name.to_owned())
            .unwrap_or_else(|| id.to_owned());
        return (Some(id), name);
    }
    if let Some(source) = cargo.replace_with.as_deref() {
        return (None, format!("自定义 · {source}"));
    }
    (None, "官方 crates.io".to_owned())
}

fn rustup_summary(rustup: &RustupEnvState) -> (Option<&'static str>, String, Vec<String>) {
    let mut warnings = rustup.warnings.clone();
    let dist_mirror = rustup
        .dist
        .effective
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .and_then(mirror_for_rustup);
    let update_mirror = rustup
        .update
        .effective
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .and_then(mirror_for_rustup_update);

    if rustup.has_system_override() {
        warnings.push(
            "检测到系统级 RUSTUP_DIST_SERVER / RUSTUP_UPDATE_ROOT。系统变量会覆盖当前用户设置，如需完全切换需管理员清理。"
                .to_owned(),
        );
    }
    match (dist_mirror, update_mirror) {
        (Some(dist), Some(update)) if dist.id != update.id => {
            warnings.push(format!(
                "RUSTUP_DIST_SERVER 指向 {}，RUSTUP_UPDATE_ROOT 指向 {}，两者不一致。",
                dist.name, update.name
            ));
            (None, "自定义（地址不一致）".to_owned(), warnings)
        }
        (Some(dist), Some(_)) => (Some(dist.id), dist.name.to_owned(), warnings),
        (Some(dist), None) => {
            let update_set = rustup.update.is_set();
            if !update_set {
                warnings.push(format!(
                    "{} 的工具链镜像已启用，但 RUSTUP_UPDATE_ROOT 未设置；rustup 自更新仍会走官方源。",
                    dist.name
                ));
                (
                    Some(dist.id),
                    format!("{}（仅工具链）", dist.name),
                    warnings,
                )
            } else {
                (None, "自定义（更新源不同）".to_owned(), warnings)
            }
        }
        (None, Some(update)) => {
            warnings.push(format!(
                "RUSTUP_UPDATE_ROOT 指向 {}，但 RUSTUP_DIST_SERVER 不是预设镜像；工具链下载可能仍走官方源。",
                update.name
            ));
            (None, format!("自定义 · {}", update.name), warnings)
        }
        (None, None) => {
            if rustup.is_official() {
                (None, "官方源".to_owned(), warnings)
            } else if rustup.dist.is_set() {
                (None, "自定义源".to_owned(), warnings)
            } else if rustup.update.is_set() {
                (None, "自定义更新源".to_owned(), warnings)
            } else {
                (None, "官方源".to_owned(), warnings)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_cargo_is_labelled() {
        let cargo = CargoInspection {
            exists: true,
            replace_with: Some("corp".to_owned()),
            registry: Some("sparse+https://corp.example/index/".to_owned()),
            ..Default::default()
        };
        let (id, label) = cargo_summary(&cargo);
        assert!(id.is_none());
        assert!(label.contains("corp"));
    }
}
