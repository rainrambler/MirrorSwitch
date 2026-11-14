//! Concurrent mirror latency and download-speed measurement.
//!
//! Cargo tests fetch the sparse `config.json` first, then a 2 MiB range of a
//! real crate tarball from the mirror's advertised `dl` endpoint. Rustup tests
//! fetch `release-stable.toml`, then a range of the stable channel manifest.
//! The official source is included as a baseline.

use std::collections::VecDeque;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT_ENCODING, RANGE};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    self, cargo_config_url, mirror_by_id, rustup_channel_url, rustup_probe_url, Mirror, MirrorKind,
};
use crate::paths::APP_VERSION;
use crate::util::{epoch_millis, now_string};

pub const TEST_CRATE_NAME: &str = "windows-sys";
pub const TEST_CRATE_VERSION: &str = "0.59.0";
pub const OFFICIAL_ID: &str = "official";
pub const CACHE_SCHEMA: u32 = 1;

#[derive(Clone, Debug)]
pub struct TestTarget {
    pub id: String,
    pub kind: MirrorKind,
    pub probe_url: String,
    pub download_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpeedResult {
    pub id: String,
    pub kind: MirrorKind,
    pub tested_at: String,
    pub tested_at_ms: i64,
    pub latency_ms: f64,
    pub speed_bps: f64,
    pub bytes: u64,
    pub ok: bool,
    pub error: Option<String>,
    pub score: f64,
}

impl SpeedResult {
    pub fn display_name(&self) -> String {
        target_name(&self.id)
    }

    pub fn is_official(&self) -> bool {
        self.id == OFFICIAL_ID
    }
}

#[derive(Clone, Debug)]
pub enum SpeedEvent {
    Started { id: String, kind: MirrorKind },
    Finished(SpeedResult),
    AllFinished,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpeedCache {
    pub schema: u32,
    pub tested_at: String,
    pub tested_at_ms: i64,
    pub sample_bytes: u64,
    pub results: Vec<SpeedResult>,
}

pub fn target_name(id: &str) -> String {
    if id == OFFICIAL_ID {
        "官方源".to_owned()
    } else {
        mirror_by_id(id)
            .map(|mirror| mirror.name.to_owned())
            .unwrap_or_else(|| id.to_owned())
    }
}

pub fn targets_for(kinds: &[MirrorKind], include_official: bool) -> Vec<TestTarget> {
    let mut targets = Vec::new();
    for kind in kinds {
        if include_official {
            targets.push(official_target(*kind));
        }
        for mirror in catalog::MIRRORS {
            if mirror.supports(*kind) {
                targets.push(mirror_target(mirror, *kind));
            }
        }
    }
    targets
}

fn official_target(kind: MirrorKind) -> TestTarget {
    match kind {
        MirrorKind::Cargo => {
            let index = catalog::OFFICIAL_CARGO_INDEX;
            TestTarget {
                id: OFFICIAL_ID.to_owned(),
                kind,
                probe_url: cargo_config_url(index),
                download_url: String::new(),
            }
        }
        MirrorKind::Rustup => {
            let rustup = catalog::official_rustup();
            TestTarget {
                id: OFFICIAL_ID.to_owned(),
                kind,
                probe_url: rustup_probe_url(rustup.update),
                download_url: rustup_channel_url(rustup.dist),
            }
        }
    }
}

fn mirror_target(mirror: &Mirror, kind: MirrorKind) -> TestTarget {
    match kind {
        MirrorKind::Cargo => {
            let index = mirror.cargo.map(|c| c.index).unwrap_or_default();
            TestTarget {
                id: mirror.id.to_owned(),
                kind,
                probe_url: cargo_config_url(index),
                download_url: String::new(),
            }
        }
        MirrorKind::Rustup => {
            let rustup = mirror.rustup.expect("rustup endpoint");
            TestTarget {
                id: mirror.id.to_owned(),
                kind,
                probe_url: rustup_probe_url(rustup.update),
                download_url: rustup_channel_url(rustup.dist),
            }
        }
    }
}

/// Spawn the worker pool. The returned receiver emits progress and results.
pub fn spawn_tests(
    targets: Vec<TestTarget>,
    sample_bytes: u64,
    concurrency: usize,
    cancel: Arc<AtomicBool>,
) -> mpsc::Receiver<SpeedEvent> {
    let (tx, rx) = mpsc::channel();
    let queue = Arc::new(Mutex::new(VecDeque::from(targets)));
    let workers = concurrency.clamp(1, 6);
    let _ = thread::Builder::new()
        .name("mirrorswitch-speed-coordinator".to_owned())
        .spawn(move || {
            let mut handles = Vec::new();
            for index in 0..workers {
                let queue = Arc::clone(&queue);
                let tx = tx.clone();
                let cancel = Arc::clone(&cancel);
                let handle = thread::Builder::new()
                    .name(format!("mirrorswitch-speed-{index}"))
                    .spawn(move || {
                        let client = build_client().ok();
                        loop {
                            if cancel.load(Ordering::Relaxed) {
                                break;
                            }
                            let target = {
                                let mut queue = queue.lock().expect("speed queue poisoned");
                                queue.pop_front()
                            };
                            let Some(target) = target else {
                                break;
                            };
                            let _ = tx.send(SpeedEvent::Started {
                                id: target.id.clone(),
                                kind: target.kind,
                            });
                            let result = match &client {
                                Some(client) => run_test(client, &target, sample_bytes, &cancel),
                                None => failure_result(&target, "无法初始化 HTTPS 客户端"),
                            };
                            let _ = tx.send(SpeedEvent::Finished(result));
                        }
                    });
                if let Ok(handle) = handle {
                    handles.push(handle);
                }
            }
            for handle in handles {
                let _ = handle.join();
            }
            let _ = tx.send(SpeedEvent::AllFinished);
        });
    rx
}

pub fn build_client() -> Result<Client> {
    Client::builder()
        .user_agent(format!(
            "MirrorSwitch/{APP_VERSION} (+https://github.com/rust-lang/rustup)"
        ))
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()
        .context("无法创建 HTTPS 客户端")
}

pub fn run_test(
    client: &Client,
    target: &TestTarget,
    sample_bytes: u64,
    cancel: &AtomicBool,
) -> SpeedResult {
    let probe_start = Instant::now();
    let probe = probe_target(client, target, cancel);
    let latency_ms = probe_start.elapsed().as_secs_f64() * 1000.0;

    let probe = match probe {
        Ok(probe) => probe,
        Err(error) => {
            return failure_result_with_latency(target, latency_ms, format!("探测失败：{error:#}"))
        }
    };

    if cancel.load(Ordering::Relaxed) {
        return failure_result_with_latency(target, latency_ms, "已取消");
    }

    let download_url = match target.kind {
        MirrorKind::Cargo => {
            let config: CargoIndexConfig = match serde_json::from_slice(&probe.body) {
                Ok(config) => config,
                Err(error) => {
                    return failure_result_with_latency(
                        target,
                        latency_ms,
                        format!("索引 config.json 无法解析：{error}"),
                    )
                }
            };
            match build_cargo_download_url(&config.dl, TEST_CRATE_NAME, TEST_CRATE_VERSION) {
                Ok(url) => url,
                Err(error) => {
                    return failure_result_with_latency(
                        target,
                        latency_ms,
                        format!("下载地址无效：{error}"),
                    )
                }
            }
        }
        MirrorKind::Rustup => target.download_url.clone(),
    };

    let transfer_start = Instant::now();
    let downloaded = download_sample(client, &download_url, sample_bytes, cancel);
    let elapsed = transfer_start.elapsed().as_secs_f64().max(0.001);
    let (bytes, head) = match downloaded {
        Ok(value) => value,
        Err(error) => {
            return failure_result_with_latency(
                target,
                latency_ms,
                format!("下载测试失败：{error:#}"),
            )
        }
    };

    if bytes < 1024 {
        return failure_result_with_latency(
            target,
            latency_ms,
            format!("测试文件过小（{bytes} 字节），镜像可能返回了错误页面"),
        );
    }

    let valid = match target.kind {
        MirrorKind::Cargo => head.starts_with(&[0x1f, 0x8b]),
        MirrorKind::Rustup => {
            let text = String::from_utf8_lossy(&head);
            text.contains("manifest-version") || text.contains("date =")
        }
    };
    if !valid {
        return failure_result_with_latency(
            target,
            latency_ms,
            "下载内容校验失败，镜像可能返回了错误页面或未同步",
        );
    }

    let speed = bytes as f64 / elapsed;
    SpeedResult {
        id: target.id.clone(),
        kind: target.kind,
        tested_at: now_string(),
        tested_at_ms: epoch_millis(),
        latency_ms,
        speed_bps: speed,
        bytes,
        ok: true,
        error: None,
        score: score(latency_ms, speed),
    }
}

struct Probe {
    body: Vec<u8>,
}

fn probe_target(client: &Client, target: &TestTarget, cancel: &AtomicBool) -> Result<Probe> {
    let deadline = Instant::now() + Duration::from_secs(18);
    let mut response = client
        .get(&target.probe_url)
        .header(ACCEPT_ENCODING, "identity")
        .send()
        .with_context(|| format!("请求 {}", target.probe_url))?;
    ensure_success(response.status(), &target.probe_url)?;
    let (total, body) = read_limited(&mut response, 256 * 1024, deadline, cancel)?;
    if total == 0 || body.is_empty() {
        bail!("返回内容为空");
    }
    Ok(Probe { body })
}

fn download_sample(
    client: &Client,
    url: &str,
    sample_bytes: u64,
    cancel: &AtomicBool,
) -> Result<(u64, Vec<u8>)> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let range = format!("bytes=0-{}", sample_bytes.saturating_sub(1));
    let mut response = client
        .get(url)
        .header(ACCEPT_ENCODING, "identity")
        .header(RANGE, range)
        .send()
        .with_context(|| format!("请求 {url}"))?;
    ensure_success(response.status(), url)?;
    let (bytes, head) = read_limited(&mut response, sample_bytes, deadline, cancel)?;
    Ok((bytes, head))
}

fn ensure_success(status: StatusCode, url: &str) -> Result<()> {
    if status.is_success() {
        Ok(())
    } else {
        bail!("HTTP {status}（{url}）")
    }
}

fn read_limited<R: Read>(
    reader: &mut R,
    cap: u64,
    deadline: Instant,
    cancel: &AtomicBool,
) -> Result<(u64, Vec<u8>)> {
    let mut buffer = [0u8; 32 * 1024];
    let mut total = 0u64;
    let mut head = Vec::with_capacity(8192);
    while total < cap {
        if cancel.load(Ordering::Relaxed) {
            bail!("已取消");
        }
        if Instant::now() >= deadline {
            bail!("读取超时");
        }
        let to_read = (buffer.len() as u64).min(cap - total) as usize;
        let read = reader
            .read(&mut buffer[..to_read])
            .context("读取响应内容失败")?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if head.len() < 8192 {
            let remaining = 8192 - head.len();
            head.extend_from_slice(&buffer[..read.min(remaining)]);
        }
    }
    Ok((total, head))
}

#[derive(Debug, Deserialize)]
struct CargoIndexConfig {
    dl: String,
}

/// Cargo's download URL construction: either the template has markers, or the
/// standard `/{crate}/{version}/download` suffix is appended.
pub fn build_cargo_download_url(dl: &str, name: &str, version: &str) -> Result<String> {
    let dl = dl.trim().trim_end_matches('/');
    if dl.is_empty() {
        bail!("config.json 中缺少 dl 字段");
    }
    let has_markers = [
        "{crate}",
        "{version}",
        "{prefix}",
        "{lowerprefix}",
        "{sha256-checksum}",
    ]
    .iter()
    .any(|marker| dl.contains(marker));
    if dl.contains("{sha256-checksum}") {
        bail!("不支持包含 {{sha256-checksum}} 的下载模板");
    }
    if has_markers {
        let prefix = cargo_prefix(name);
        Ok(dl
            .replace("{crate}", name)
            .replace("{version}", version)
            .replace("{lowerprefix}", &prefix.to_lowercase())
            .replace("{prefix}", &prefix))
    } else {
        Ok(format!("{dl}/{name}/{version}/download"))
    }
}

fn cargo_prefix(name: &str) -> String {
    match name.len() {
        0 => String::new(),
        1 => "1".to_owned(),
        2 => format!("2/{name}"),
        3 => format!("3/{}", &name[..1]),
        _ => format!("{}/{}", &name[..2], &name[2..4]),
    }
}

pub fn score(latency_ms: f64, speed_bps: f64) -> f64 {
    if !speed_bps.is_finite() || speed_bps <= 0.0 {
        return 0.0;
    }
    let mbps = speed_bps / (1024.0 * 1024.0);
    let speed_component = (mbps / 20.0).clamp(0.0, 1.0) * 100.0;
    let latency_component = if latency_ms.is_finite() && latency_ms > 0.0 {
        (1.0 - latency_ms / 1000.0).clamp(0.0, 1.0) * 100.0
    } else {
        50.0
    };
    (speed_component * 0.8 + latency_component * 0.2).clamp(0.0, 100.0)
}

fn failure_result(target: &TestTarget, error: impl Into<String>) -> SpeedResult {
    failure_result_with_latency(target, 0.0, error)
}

fn failure_result_with_latency(
    target: &TestTarget,
    latency_ms: f64,
    error: impl Into<String>,
) -> SpeedResult {
    SpeedResult {
        id: target.id.clone(),
        kind: target.kind,
        tested_at: now_string(),
        tested_at_ms: epoch_millis(),
        latency_ms,
        speed_bps: 0.0,
        bytes: 0,
        ok: false,
        error: Some(error.into()),
        score: 0.0,
    }
}

pub fn load_cache(path: &std::path::Path) -> Option<SpeedCache> {
    let text = std::fs::read_to_string(path).ok()?;
    let cache: SpeedCache = serde_json::from_str(&text).ok()?;
    if cache.schema <= CACHE_SCHEMA {
        Some(cache)
    } else {
        None
    }
}

pub fn save_cache(
    path: &std::path::Path,
    results: &[SpeedResult],
    sample_bytes: u64,
) -> Result<()> {
    let cache = SpeedCache {
        schema: CACHE_SCHEMA,
        tested_at: now_string(),
        tested_at_ms: epoch_millis(),
        sample_bytes,
        results: results.to_vec(),
    };
    let json = serde_json::to_string_pretty(&cache)?;
    crate::util::atomic_write(path, json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_download_url_handles_both_formats() {
        assert_eq!(
            build_cargo_download_url("https://rsproxy.cn/api/v1/crates", "serde", "1.0.219")
                .unwrap(),
            "https://rsproxy.cn/api/v1/crates/serde/1.0.219/download"
        );
        assert_eq!(
            build_cargo_download_url(
                "https://mirror.sjtu.edu.cn/crates.io/crates/{crate}/{crate}-{version}.crate",
                "serde",
                "1.0.219"
            )
            .unwrap(),
            "https://mirror.sjtu.edu.cn/crates.io/crates/serde/serde-1.0.219.crate"
        );
    }

    #[test]
    fn cargo_prefix_matches_sparse_index_layout() {
        assert_eq!(cargo_prefix("a"), "1");
        assert_eq!(cargo_prefix("ab"), "2/ab");
        assert_eq!(cargo_prefix("abc"), "3/a");
        assert_eq!(cargo_prefix("serde"), "se/rd");
        assert_eq!(cargo_prefix("windows-sys"), "wi/nd");
    }

    #[test]
    fn score_prefers_faster_downloads() {
        let fast = score(20.0, 10.0 * 1024.0 * 1024.0);
        let slow = score(20.0, 1.0 * 1024.0 * 1024.0);
        assert!(fast > slow);
        assert_eq!(score(20.0, 0.0), 0.0);
    }

    #[test]
    fn targets_include_official_and_all_presets() {
        let expected_cargo = catalog::MIRRORS
            .iter()
            .filter(|m| m.supports(MirrorKind::Cargo))
            .count()
            + 1;
        let cargo = targets_for(&[MirrorKind::Cargo], true);
        assert_eq!(cargo.len(), expected_cargo);
        assert_eq!(cargo[0].id, OFFICIAL_ID);
        assert!(cargo.iter().any(|t| t.id == "rsproxy"));
        assert!(!cargo.iter().any(|t| t.id == "ustc"));
        let rustup = targets_for(&[MirrorKind::Rustup], false);
        assert_eq!(rustup.len(), catalog::MIRRORS.len());
    }

    #[test]
    fn local_http_speed_test_roundtrip() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test server");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = Arc::clone(&stop);
        let server = std::thread::spawn(move || {
            let payload = {
                let mut data = vec![0x1f, 0x8b];
                data.resize(700 * 1024, 0x42);
                data
            };
            let mut served = 0usize;
            let deadline = Instant::now() + Duration::from_secs(8);
            while !server_stop.load(Ordering::Relaxed) && served < 6 && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let mut request = Vec::new();
                        let mut buffer = [0u8; 2048];
                        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                            match stream.read(&mut buffer) {
                                Ok(0) => break,
                                Ok(read) => request.extend_from_slice(&buffer[..read]),
                                Err(_) => break,
                            }
                        }
                        let text = String::from_utf8_lossy(&request);
                        let path = text
                            .lines()
                            .next()
                            .and_then(|line| line.split_whitespace().nth(1))
                            .unwrap_or("/");
                        let (content_type, body): (&str, Vec<u8>) = if path.ends_with("config.json")
                        {
                            let json = format!("{{\"dl\":\"http://127.0.0.1:{port}/crates\"}}");
                            ("application/json", json.into_bytes())
                        } else {
                            ("application/gzip", payload.clone())
                        };
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(header.as_bytes());
                        let _ = stream.write_all(&body);
                        let _ = stream.flush();
                        served += 1;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        let target = TestTarget {
            id: "local".to_owned(),
            kind: MirrorKind::Cargo,
            probe_url: format!("http://127.0.0.1:{port}/config.json"),
            download_url: String::new(),
        };
        let client = build_client().unwrap();
        let cancel = AtomicBool::new(false);
        std::thread::sleep(Duration::from_millis(50));
        let mut result = run_test(&client, &target, 512 * 1024, &cancel);
        if !result.ok {
            // A loopback connection can transiently reset while the parallel
            // test harness is busy; one retry keeps this end-to-end test stable.
            std::thread::sleep(Duration::from_millis(100));
            result = run_test(&client, &target, 512 * 1024, &cancel);
        }
        stop.store(true, Ordering::Relaxed);
        let _ = server.join();
        assert!(result.ok, "{:?}", result.error);
        assert!(result.latency_ms > 0.0);
        assert!(result.speed_bps > 0.0);
        assert!(result.bytes >= 1024);
    }

    /// Live network probe, ignored by default:
    /// `cargo test live_probe -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_probe() {
        let targets = targets_for(&[MirrorKind::Cargo, MirrorKind::Rustup], true);
        let cancel = Arc::new(AtomicBool::new(false));
        let client = build_client().unwrap();
        for target in targets {
            let result = run_test(&client, &target, 1024 * 1024, &cancel);
            if result.ok {
                println!(
                    "{:?} {:<10} {:>8} {:>12} score={:.1}",
                    result.kind,
                    result.display_name(),
                    crate::util::format_ms(result.latency_ms),
                    crate::util::format_speed(result.speed_bps),
                    result.score
                );
            } else {
                println!(
                    "{:?} {:<10} FAILED: {}",
                    result.kind,
                    result.display_name(),
                    result.error.unwrap_or_default()
                );
            }
        }
    }
}
