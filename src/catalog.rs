//! Built-in mirror catalogue and endpoint helpers.
//!
//! All addresses below were checked against the providers' public help pages
//! and probed on 2026-11-13. Cargo mirrors use the sparse HTTP index, which is
//! supported by Cargo 1.68+ (all currently supported stable releases).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MirrorKind {
    Cargo,
    Rustup,
}

impl MirrorKind {
    pub const ALL: [Self; 2] = [Self::Cargo, Self::Rustup];

    pub fn label(self) -> &'static str {
        match self {
            Self::Cargo => "Cargo",
            Self::Rustup => "Rustup",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Cargo => "crates.io 稀疏索引与依赖下载",
            Self::Rustup => "Rust 工具链与 rustup 自更新",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RustupEndpoints {
    /// Base for `dist/channel-rust-*.toml` and toolchain archives.
    pub dist: &'static str,
    /// Base for rustup self-update metadata.
    pub update: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct CargoEndpoints {
    /// `sparse+https://.../` index.
    pub index: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct Mirror {
    pub id: &'static str,
    pub name: &'static str,
    pub org: &'static str,
    pub summary: &'static str,
    pub homepage: &'static str,
    pub rustup: Option<RustupEndpoints>,
    pub cargo: Option<CargoEndpoints>,
    pub recommended: bool,
    pub tags: &'static [&'static str],
}

impl Mirror {
    pub fn supports(&self, kind: MirrorKind) -> bool {
        match kind {
            MirrorKind::Cargo => self.cargo.is_some(),
            MirrorKind::Rustup => self.rustup.is_some(),
        }
    }
}

pub const OFFICIAL_CARGO_INDEX: &str = "sparse+https://index.crates.io/";
pub const OFFICIAL_RUSTUP_DIST: &str = "https://static.rust-lang.org";
pub const OFFICIAL_RUSTUP_UPDATE: &str = "https://static.rust-lang.org/rustup";

pub fn official_rustup() -> RustupEndpoints {
    RustupEndpoints {
        dist: OFFICIAL_RUSTUP_DIST,
        update: OFFICIAL_RUSTUP_UPDATE,
    }
}

pub static MIRRORS: &[Mirror] = &[
    Mirror {
        id: "rsproxy",
        name: "RsProxy",
        org: "字节跳动",
        summary: "Rustup 与 crates.io 双加速，国内更新频繁，综合体验稳定。",
        homepage: "https://rsproxy.cn",
        rustup: Some(RustupEndpoints {
            dist: "https://rsproxy.cn",
            update: "https://rsproxy.cn/rustup",
        }),
        cargo: Some(CargoEndpoints {
            index: "sparse+https://rsproxy.cn/index/",
        }),
        recommended: true,
        tags: &["双源", "推荐", "更新快"],
    },
    Mirror {
        id: "ustc",
        name: "中科大 USTC",
        org: "中国科学技术大学",
        summary: "教育网 Rustup 工具链镜像；其 Cargo 下载端点当前异常，因此只启用工具链切换。",
        homepage: "https://mirrors.ustc.edu.cn",
        rustup: Some(RustupEndpoints {
            dist: "https://mirrors.ustc.edu.cn/rust-static",
            update: "https://mirrors.ustc.edu.cn/rust-static/rustup",
        }),
        // Checked 2026-11-13: USTC's index config advertises
        // `/crates/api/v1/crates`, which returns HTTP 404, while the working
        // path is `/crates.io/api/v1/crates`. Do not write a Cargo source that
        // cannot download crates.
        cargo: None,
        recommended: true,
        tags: &["Rustup", "教育网"],
    },
    Mirror {
        id: "tuna",
        name: "清华 TUNA",
        org: "清华大学",
        summary: "索引同步及时；crate 包由官方 CDN 提供，适合索引瓶颈场景。",
        homepage: "https://mirrors.tuna.tsinghua.edu.cn",
        rustup: Some(RustupEndpoints {
            dist: "https://mirrors.tuna.tsinghua.edu.cn/rustup",
            update: "https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup",
        }),
        cargo: Some(CargoEndpoints {
            index: "sparse+https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/",
        }),
        recommended: true,
        tags: &["双源", "教育网"],
    },
    Mirror {
        id: "sjtu",
        name: "上海交大 SJTU",
        org: "上海交通大学",
        summary: "华东地区延迟低；工具链大文件可能重定向到对象存储。",
        homepage: "https://mirrors.sjtug.sjtu.edu.cn",
        rustup: Some(RustupEndpoints {
            dist: "https://mirror.sjtu.edu.cn/rust-static",
            update: "https://mirror.sjtu.edu.cn/rust-static/rustup",
        }),
        cargo: Some(CargoEndpoints {
            index: "sparse+https://mirrors.sjtug.sjtu.edu.cn/crates.io-index/",
        }),
        recommended: false,
        tags: &["双源", "华东"],
    },
    Mirror {
        id: "aliyun",
        name: "阿里云",
        org: "Alibaba Cloud",
        summary: "公网覆盖广，提供 crates 索引与 rustup 工具链镜像。",
        homepage: "https://developer.aliyun.com/mirror/rustup/",
        rustup: Some(RustupEndpoints {
            dist: "https://mirrors.aliyun.com/rustup",
            update: "https://mirrors.aliyun.com/rustup/rustup",
        }),
        cargo: Some(CargoEndpoints {
            index: "sparse+https://mirrors.aliyun.com/crates.io-index/",
        }),
        recommended: false,
        tags: &["双源", "公网"],
    },
    Mirror {
        id: "huawei",
        name: "华为云",
        org: "Huawei Cloud",
        summary: "提供工具链与 crates 索引，适合华为云内网与政企网络。",
        homepage: "https://mirrors.huaweicloud.com/rustup/",
        rustup: Some(RustupEndpoints {
            dist: "https://mirrors.huaweicloud.com/rustup",
            update: "https://mirrors.huaweicloud.com/rustup/rustup",
        }),
        cargo: Some(CargoEndpoints {
            index: "sparse+https://mirrors.huaweicloud.com/repository/rust/crates.io-index/",
        }),
        recommended: false,
        tags: &["双源", "政企"],
    },
];

pub fn mirror_by_id(id: &str) -> Option<&'static Mirror> {
    MIRRORS.iter().find(|mirror| mirror.id == id)
}

pub fn mirror_for_cargo(index: &str) -> Option<&'static Mirror> {
    let needle = normalize_cargo_index(index);
    MIRRORS.iter().find(|mirror| {
        mirror
            .cargo
            .map(|endpoint| normalize_cargo_index(endpoint.index) == needle)
            .unwrap_or(false)
    })
}

pub fn mirror_for_rustup(dist: &str) -> Option<&'static Mirror> {
    let needle = normalize_url(dist);
    MIRRORS.iter().find(|mirror| {
        mirror
            .rustup
            .map(|endpoint| normalize_url(endpoint.dist) == needle)
            .unwrap_or(false)
    })
}

pub fn mirror_for_rustup_update(update: &str) -> Option<&'static Mirror> {
    let needle = normalize_url(update);
    MIRRORS.iter().find(|mirror| {
        mirror
            .rustup
            .map(|endpoint| normalize_url(endpoint.update) == needle)
            .unwrap_or(false)
    })
}

pub fn normalize_url(url: &str) -> String {
    url.trim()
        .trim_end_matches('/')
        .to_ascii_lowercase()
        .replace("http://", "https://")
}

pub fn normalize_cargo_index(index: &str) -> String {
    normalize_url(index.trim().trim_start_matches("sparse+"))
}

/// Turn a sparse index URL into the URL of its `config.json`.
pub fn cargo_config_url(index: &str) -> String {
    let base = index.trim().trim_start_matches("sparse+");
    format!("{}/config.json", base.trim_end_matches('/'))
}

/// Toolchain manifest used for the Rustup download-speed sample.
pub fn rustup_channel_url(dist: &str) -> String {
    format!(
        "{}/dist/channel-rust-stable.toml",
        dist.trim_end_matches('/')
    )
}

/// Small metadata file used to measure Rustup mirror latency.
pub fn rustup_probe_url(update: &str) -> String {
    format!("{}/release-stable.toml", update.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_endpoints_are_well_formed() {
        let mut ids = std::collections::HashSet::new();
        for mirror in MIRRORS {
            assert!(ids.insert(mirror.id), "duplicate id {}", mirror.id);
            if let Some(cargo) = mirror.cargo {
                assert!(cargo.index.starts_with("sparse+https://"));
                assert!(cargo.index.ends_with('/'));
            }
            if let Some(rustup) = mirror.rustup {
                assert!(rustup.dist.starts_with("https://"));
                assert!(rustup.update.starts_with("https://"));
            }
        }
    }

    #[test]
    fn lookup_normalizes_urls() {
        assert_eq!(
            mirror_for_cargo("sparse+https://MIRRORS.ALIYUN.COM/crates.io-index").map(|m| m.id),
            Some("aliyun")
        );
        assert_eq!(
            mirror_for_rustup("https://rsproxy.cn/").map(|m| m.id),
            Some("rsproxy")
        );
        assert_eq!(
            mirror_for_rustup_update("https://rsproxy.cn/rustup/").map(|m| m.id),
            Some("rsproxy")
        );
        assert!(mirror_for_cargo("sparse+https://example.com/").is_none());
    }

    #[test]
    fn endpoint_helpers_join_paths_once() {
        assert_eq!(
            cargo_config_url("sparse+https://rsproxy.cn/index/"),
            "https://rsproxy.cn/index/config.json"
        );
        assert_eq!(
            rustup_channel_url("https://rsproxy.cn/"),
            "https://rsproxy.cn/dist/channel-rust-stable.toml"
        );
        assert_eq!(
            rustup_probe_url("https://rsproxy.cn/rustup"),
            "https://rsproxy.cn/rustup/release-stable.toml"
        );
    }
}
