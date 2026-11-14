use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Margin, RichText, Sense,
    Stroke, Ui, Vec2,
};

use crate::backup::{self, Snapshot, SnapshotKind};
use crate::cargo;
use crate::catalog::{self, mirror_by_id, Mirror, MirrorKind};
use crate::logging::{AppLog, LogLevel};
use crate::paths::{AppPaths, APP_VERSION};
use crate::platform;
use crate::settings::{Settings, ThemeMode};
use crate::speedtest::{self, SpeedEvent, SpeedResult};
use crate::state::ActiveState;
use crate::theme::{self, provider_color, Palette};
use crate::util::{
    ellipsize, format_bytes, format_ms, format_speed, relative_time, reveal_path, short_path,
};
use crate::widgets::{
    self, chip, danger_button, dot, empty_state, ghost_button, metric_bar, nav_item, notice,
    page_header, primary_button, secondary_button, section_title, theme_toggle, NavIcon,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Overview,
    Mirrors,
    Speed,
    Backups,
    Settings,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Overview => "概览",
            Self::Mirrors => "镜像源",
            Self::Speed => "测速对比",
            Self::Backups => "备份恢复",
            Self::Settings => "设置",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::Overview => "集中管理 Rustup 与 Cargo 镜像，让下载稳定而快速。",
            Self::Mirrors => "选择预设镜像并分别应用到工具链或 crates.io，其他配置保持不变。",
            Self::Speed => "测量当前网络下的延迟与真实下载速度，并与官方源对比。",
            Self::Backups => "每次变更前自动留档；随时恢复 Cargo 配置与 Rustup 环境变量。",
            Self::Settings => "路径、测速参数与操作日志。",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        match key.trim().to_ascii_lowercase().as_str() {
            "overview" | "home" | "概览" => Some(Self::Overview),
            "mirrors" | "mirror" | "镜像" => Some(Self::Mirrors),
            "speed" | "test" | "测速" => Some(Self::Speed),
            "backups" | "backup" | "备份" => Some(Self::Backups),
            "settings" | "设置" => Some(Self::Settings),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MirrorFilter {
    All,
    Cargo,
    Rustup,
}

impl MirrorFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Cargo => "Cargo",
            Self::Rustup => "Rustup",
        }
    }

    fn accepts(self, mirror: &Mirror) -> bool {
        match self {
            Self::All => true,
            Self::Cargo => mirror.supports(MirrorKind::Cargo),
            Self::Rustup => mirror.supports(MirrorKind::Rustup),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestScope {
    Both,
    Cargo,
    Rustup,
}

impl TestScope {
    fn kinds(self) -> Vec<MirrorKind> {
        match self {
            Self::Both => vec![MirrorKind::Cargo, MirrorKind::Rustup],
            Self::Cargo => vec![MirrorKind::Cargo],
            Self::Rustup => vec![MirrorKind::Rustup],
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Both => "全部",
            Self::Cargo => "Cargo",
            Self::Rustup => "Rustup",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

impl ToastKind {
    fn color(self, palette: Palette) -> Color32 {
        match self {
            Self::Info => palette.info,
            Self::Success => palette.success,
            Self::Warning => palette.warning,
            Self::Error => palette.danger,
        }
    }
}

#[derive(Clone, Debug)]
struct Toast {
    kind: ToastKind,
    title: String,
    detail: Option<String>,
    created: Instant,
}

#[derive(Clone, Debug)]
enum ConfirmAction {
    Switch {
        mirror_id: Option<String>,
        kinds: Vec<MirrorKind>,
    },
    Restore {
        id: String,
    },
    Delete {
        id: String,
    },
}

impl ConfirmAction {
    fn into_action(self) -> UiAction {
        match self {
            Self::Switch { mirror_id, kinds } => UiAction::Switch {
                mirror_id,
                kinds,
                confirmed: true,
            },
            Self::Restore { id } => UiAction::Restore {
                id,
                confirmed: true,
            },
            Self::Delete { id } => UiAction::Delete {
                id,
                confirmed: true,
            },
        }
    }
}

#[derive(Clone, Debug)]
struct ConfirmDialog {
    title: String,
    message: String,
    details: Vec<String>,
    confirm_label: String,
    danger: bool,
    action: ConfirmAction,
}

#[derive(Clone, Debug)]
enum UiAction {
    Switch {
        mirror_id: Option<String>,
        kinds: Vec<MirrorKind>,
        confirmed: bool,
    },
    RunTests(TestScope),
    CancelTests,
    ThemeToggle,
    Refresh,
    CreateBackup,
    Detail(String),
    Restore {
        id: String,
        confirmed: bool,
    },
    Delete {
        id: String,
        confirmed: bool,
    },
    Reveal(PathBuf),
    OpenUrl(String),
    ClearLog,
}

#[derive(Default)]
struct SpeedState {
    results: Vec<SpeedResult>,
    running: bool,
    cancelling: bool,
    total: usize,
    completed: usize,
    active: HashSet<(String, MirrorKind)>,
    receiver: Option<Receiver<SpeedEvent>>,
    cancel: Arc<AtomicBool>,
    started_at: Option<Instant>,
    last_tested_at: Option<String>,
    last_tested_ms: Option<i64>,
    sample_bytes: u64,
}

pub struct MirrorSwitchApp {
    paths: AppPaths,
    settings: Settings,
    palette: Palette,
    page: Page,
    active: ActiveState,
    mirror_filter: MirrorFilter,
    test_scope: TestScope,
    speed: SpeedState,
    snapshots: Vec<Snapshot>,
    snapshot_errors: Vec<String>,
    confirm: Option<ConfirmDialog>,
    detail: Option<Snapshot>,
    toast: Option<Toast>,
    log: AppLog,
    font_path: Option<String>,
    startup_warnings: Vec<String>,
    last_refresh: Instant,
    screenshot_path: Option<PathBuf>,
    screenshot_started: Instant,
    screenshot_requested: bool,
}

impl MirrorSwitchApp {
    pub fn new(cc: &eframe::CreationContext<'_>, paths: AppPaths) -> Self {
        // Fonts must be installed before the first frame so Chinese text is not
        // rendered with missing glyphs.
        let font_path = theme::install_fonts(&cc.egui_ctx);

        let mut log = AppLog::load(paths.log_path.clone());
        let (settings, settings_warning) = Settings::load(&paths.settings_path);
        let palette = Palette::new(settings.theme);
        theme::apply_style(&cc.egui_ctx, settings.theme);

        let active = ActiveState::read(&paths);
        let (mut snapshots, snapshot_errors) = backup::load_all(&paths);
        let mut startup_warnings = Vec::new();
        if let Some(warning) = settings_warning {
            startup_warnings.push(warning);
        }
        if font_path.is_none() {
            startup_warnings.push(
                "未找到可用的中文字体（Microsoft YaHei / 等线 / 黑体），界面可能显示方框。"
                    .to_owned(),
            );
        }
        startup_warnings.extend(snapshot_errors.iter().cloned());

        log.info(format!(
            "{APP_NAME} {APP_VERSION} 启动 · Cargo={} · Rustup={}",
            active.cargo_label, active.rustup_label
        ));
        if let Some(path) = &font_path {
            log.info(format!("中文字体：{path}"));
        }

        // Create a startup snapshot only when there is something to preserve and
        // the current state differs from the latest snapshot.
        if settings.auto_backup && (paths.cargo_config.exists() || !active.rustup.is_official()) {
            match backup::needs_auto_backup(&paths) {
                Ok(true) => {
                    match backup::create_snapshot(&paths, SnapshotKind::Auto, "应用启动时自动备份")
                    {
                        Ok(outcome) if outcome.created => {
                            log.success(format!("已创建启动备份 {}", outcome.snapshot.id));
                            let (loaded, errors) = backup::load_all(&paths);
                            snapshots = loaded;
                            if !errors.is_empty() {
                                startup_warnings.extend(errors);
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            startup_warnings.push(format!("启动自动备份失败：{error}"));
                            log.error(format!("启动自动备份失败：{error}"));
                        }
                    }
                }
                Ok(false) => {}
                Err(error) => {
                    startup_warnings.push(format!("检查备份状态失败：{error}"));
                }
            }
        }

        if settings.auto_backup {
            let removed = backup::prune_auto(&paths, settings.max_auto_backups);
            if !removed.is_empty() {
                log.info(format!("按保留策略清理 {} 份自动备份", removed.len()));
                snapshots = backup::load_all(&paths).0;
            }
        }

        let mut speed = SpeedState {
            sample_bytes: settings.sample_bytes(),
            ..Default::default()
        };
        if let Some(cache) = speedtest::load_cache(&paths.speed_cache_path) {
            speed.results = cache.results;
            speed.last_tested_at = Some(cache.tested_at);
            speed.last_tested_ms = Some(cache.tested_at_ms);
            speed.sample_bytes = cache.sample_bytes;
        }

        let page = std::env::var("MIRRORSWITCH_SCREENSHOT_PAGE")
            .ok()
            .and_then(|key| Page::from_key(&key))
            .unwrap_or(Page::Overview);
        let screenshot_path = std::env::var_os("MIRRORSWITCH_SCREENSHOT").map(PathBuf::from);
        if screenshot_path.is_some() {
            cc.egui_ctx
                .request_repaint_after(Duration::from_millis(300));
        }

        let mut app = Self {
            paths,
            settings,
            palette,
            page,
            active,
            mirror_filter: MirrorFilter::All,
            test_scope: TestScope::Both,
            speed,
            snapshots,
            snapshot_errors,
            confirm: None,
            detail: None,
            toast: None,
            log,
            font_path,
            startup_warnings,
            last_refresh: Instant::now(),
            screenshot_path,
            screenshot_started: Instant::now(),
            screenshot_requested: false,
        };

        if app.screenshot_path.is_some() {
            if let Ok(modal) = std::env::var("MIRRORSWITCH_SCREENSHOT_MODAL") {
                match modal.as_str() {
                    "detail" => app.detail = app.snapshots.first().cloned(),
                    "restore" => {
                        if let Some(snapshot) = app.snapshots.first() {
                            app.confirm = Some(restore_confirm_dialog(snapshot));
                        }
                    }
                    _ => {}
                }
            }
        }
        app
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot_path.clone() else {
            return;
        };
        let captured = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                if let egui::Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = captured {
            match crate::screenshot::save_png(&path, &image) {
                Ok(()) => self
                    .log
                    .success(format!("已保存界面截图 {}", path.display())),
                Err(error) => self.log.error(format!("保存截图失败：{error:#}")),
            }
            self.screenshot_path = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !self.screenshot_requested
            && self.screenshot_started.elapsed() > Duration::from_millis(1200)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.screenshot_requested = true;
            ctx.request_repaint_after(Duration::from_millis(150));
        } else if self.screenshot_requested {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        if self.screenshot_started.elapsed() > Duration::from_secs(15) {
            self.log.error("截图请求超时，已关闭窗口");
            self.screenshot_path = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(150));
    }

    fn refresh_active(&mut self) {
        self.active = ActiveState::read(&self.paths);
        self.last_refresh = Instant::now();
    }

    fn reload_snapshots(&mut self) {
        let (snapshots, errors) = backup::load_all(&self.paths);
        if !errors.is_empty() {
            for error in &errors {
                self.log.error(error.clone());
            }
        }
        self.snapshot_errors = errors;
        self.snapshots = snapshots;
    }

    fn set_toast(&mut self, kind: ToastKind, title: impl Into<String>, detail: Option<String>) {
        self.toast = Some(Toast {
            kind,
            title: title.into(),
            detail,
            created: Instant::now(),
        });
    }

    fn toast_error(&mut self, title: impl Into<String>, detail: impl Into<String>) {
        self.set_toast(ToastKind::Error, title, Some(detail.into()));
    }

    fn save_settings(&mut self) {
        if let Err(error) = self.settings.save(&self.paths.settings_path) {
            self.log.error(format!("保存设置失败：{error}"));
        }
    }

    fn handle_actions(&mut self, actions: Vec<UiAction>, ctx: &egui::Context) {
        for action in actions {
            match action {
                UiAction::Switch {
                    mirror_id,
                    kinds,
                    confirmed,
                } => self.request_switch(mirror_id, kinds, confirmed),
                UiAction::RunTests(scope) => self.start_tests(scope),
                UiAction::CancelTests => {
                    if self.speed.running {
                        self.speed.cancel.store(true, Ordering::Relaxed);
                        self.speed.cancelling = true;
                        self.set_toast(
                            ToastKind::Info,
                            "正在停止测速",
                            Some("当前请求结束后会保留已得到的结果。".to_owned()),
                        );
                    }
                }
                UiAction::ThemeToggle => {
                    self.settings.theme = self.settings.theme.toggled();
                    self.palette = Palette::new(self.settings.theme);
                    theme::apply_style(ctx, self.settings.theme);
                }
                UiAction::Refresh => {
                    self.refresh_active();
                    self.reload_snapshots();
                    self.set_toast(
                        ToastKind::Success,
                        "已重新读取配置",
                        Some("Cargo 配置文件与 Rustup 环境变量已刷新。".to_owned()),
                    );
                }
                UiAction::CreateBackup => self.create_manual_backup(),
                UiAction::Detail(id) => {
                    self.detail = self.snapshots.iter().find(|s| s.id == id).cloned();
                }
                UiAction::Restore { id, confirmed } => {
                    if confirmed {
                        self.perform_restore(&id);
                    } else if let Some(snapshot) = self.snapshots.iter().find(|s| s.id == id) {
                        self.confirm = Some(restore_confirm_dialog(snapshot));
                    }
                }
                UiAction::Delete { id, confirmed } => {
                    if confirmed {
                        self.perform_delete(&id);
                    } else if let Some(snapshot) = self.snapshots.iter().find(|s| s.id == id) {
                        self.confirm = Some(ConfirmDialog {
                            title: "删除这份备份？".to_owned(),
                            message: format!(
                                "将永久删除 {} 的备份文件，无法在应用内撤销。",
                                snapshot.display_time()
                            ),
                            details: vec![backup::summary(snapshot)],
                            confirm_label: "删除备份".to_owned(),
                            danger: true,
                            action: ConfirmAction::Delete { id: id.clone() },
                        });
                    }
                }
                UiAction::Reveal(path) => {
                    if let Err(error) = reveal_path(&path) {
                        self.toast_error("无法打开路径", error.to_string());
                    }
                }
                UiAction::OpenUrl(url) => {
                    if let Err(error) = crate::util::open_url(&url) {
                        self.toast_error("无法打开链接", error.to_string());
                    }
                }
                UiAction::ClearLog => {
                    self.log.clear();
                    self.log.info("操作日志已清空");
                    self.set_toast(ToastKind::Success, "日志已清空", None);
                }
            }
        }
    }

    fn request_switch(
        &mut self,
        mirror_id: Option<String>,
        kinds: Vec<MirrorKind>,
        confirmed: bool,
    ) {
        if kinds.is_empty() {
            return;
        }
        if kinds.contains(&MirrorKind::Cargo) && self.active.cargo.parse_error.is_some() {
            self.toast_error(
                "无法修改 Cargo 配置",
                "现有 config.toml 不是合法 TOML。请先修复文件或从备份恢复。",
            );
            return;
        }
        if !confirmed && self.switch_needs_confirm(mirror_id.as_deref(), &kinds) {
            self.confirm = Some(self.build_switch_confirm(mirror_id, kinds));
            return;
        }
        self.perform_switch(mirror_id, kinds);
    }

    fn switch_needs_confirm(&self, mirror_id: Option<&str>, kinds: &[MirrorKind]) -> bool {
        if mirror_id.is_none() {
            let cargo_change =
                kinds.contains(&MirrorKind::Cargo) && !self.active.cargo.is_official();
            let rustup_change =
                kinds.contains(&MirrorKind::Rustup) && !self.active.rustup.is_official();
            return cargo_change || rustup_change;
        }
        if kinds.contains(&MirrorKind::Cargo) && self.active.cargo.is_custom() {
            return true;
        }
        if kinds.contains(&MirrorKind::Rustup)
            && !self.active.rustup.is_official()
            && self.active.rustup_mirror_id.is_none()
        {
            return true;
        }
        false
    }

    fn build_switch_confirm(
        &self,
        mirror_id: Option<String>,
        kinds: Vec<MirrorKind>,
    ) -> ConfirmDialog {
        let mirror = mirror_id.as_deref().and_then(mirror_by_id);
        let target_name = mirror
            .map(|m| m.name.to_owned())
            .unwrap_or_else(|| "官方源".to_owned());
        let mut details = Vec::new();
        if kinds.contains(&MirrorKind::Cargo) {
            details.push(format!(
                "Cargo：{}  →  {}",
                self.active.cargo_label,
                mirror
                    .map(|_| target_name.as_str())
                    .unwrap_or("官方 crates.io")
            ));
        }
        if kinds.contains(&MirrorKind::Rustup) {
            details.push(format!(
                "Rustup：{}  →  {}",
                self.active.rustup_label, target_name
            ));
        }
        details.push("变更前会自动创建一份当前配置的备份。".to_owned());
        ConfirmDialog {
            title: format!("切换到 {target_name}？"),
            message: if mirror_id.is_none() {
                "将移除 Cargo 的镜像替换设置，并清除用户级 Rustup 镜像环境变量。".to_owned()
            } else {
                "将更新 Cargo 的 source 替换项与 Rustup 环境变量；Cargo 配置中的其他内容会保留。"
                    .to_owned()
            },
            details,
            confirm_label: "确认切换".to_owned(),
            danger: false,
            action: ConfirmAction::Switch { mirror_id, kinds },
        }
    }

    fn perform_switch(&mut self, mirror_id: Option<String>, kinds: Vec<MirrorKind>) {
        let mirror = match mirror_id.as_deref() {
            Some(id) => match mirror_by_id(id) {
                Some(mirror) => Some(mirror),
                None => {
                    self.toast_error("未知镜像", format!("镜像标识 {id} 不存在"));
                    return;
                }
            },
            None => None,
        };
        let wanted: Vec<MirrorKind> = kinds
            .into_iter()
            .filter(|kind| match mirror {
                Some(mirror) => mirror.supports(*kind),
                None => true,
            })
            .collect();
        if wanted.is_empty() {
            self.toast_error(
                "该镜像不支持所选类型",
                "请选择 Cargo 或 Rustup 中的至少一项。",
            );
            return;
        }

        let cargo_change = wanted.contains(&MirrorKind::Cargo)
            && match mirror {
                Some(mirror) => self.active.cargo_mirror_id != Some(mirror.id),
                None => !self.active.cargo.is_official(),
            };
        let rustup_change = wanted.contains(&MirrorKind::Rustup)
            && match mirror {
                Some(mirror) => self.active.rustup_mirror_id != Some(mirror.id),
                None => !self.active.rustup.is_official(),
            };
        if !cargo_change && !rustup_change {
            self.set_toast(
                ToastKind::Info,
                "当前已经使用该配置",
                Some("没有需要写入的变更。".to_owned()),
            );
            return;
        }

        let target_name = mirror
            .map(|m| m.name.to_owned())
            .unwrap_or_else(|| "官方源".to_owned());
        let note = format!("切换前 · {target_name}");
        match backup::create_snapshot(&self.paths, SnapshotKind::Auto, &note) {
            Ok(outcome) => {
                if outcome.created {
                    self.log
                        .info(format!("切换前自动备份 {}", outcome.snapshot.id));
                }
            }
            Err(error) => {
                self.toast_error("无法创建切换前备份", error.to_string());
                self.log.error(format!("切换中止：无法创建备份：{error}"));
                return;
            }
        }

        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        if cargo_change {
            match cargo::apply(&self.paths.cargo_config, mirror) {
                Ok(report) => {
                    self.log.info(format!(
                        "Cargo 已切换到 {target_name}（{}）",
                        if report.changed {
                            "已写入"
                        } else {
                            "无变化"
                        }
                    ));
                }
                Err(error) => {
                    errors.push(format!("Cargo：{error}"));
                    self.log.error(format!("Cargo 切换失败：{error}"));
                }
            }
        }
        if rustup_change {
            let (dist, update) = match mirror {
                Some(mirror) => match mirror.rustup {
                    Some(endpoints) => (Some(endpoints.dist), Some(endpoints.update)),
                    None => (None, None),
                },
                None => (None, None),
            };
            match platform::set_rustup_env(dist, update) {
                Ok(env_warnings) => {
                    self.log
                        .info(format!("Rustup 环境变量已切换到 {target_name}"));
                    warnings.extend(env_warnings);
                }
                Err(error) => {
                    errors.push(format!("Rustup：{error}"));
                    self.log.error(format!("Rustup 切换失败：{error}"));
                }
            }
        }

        self.refresh_active();
        let _ = backup::prune_auto(&self.paths, self.settings.max_auto_backups);
        self.reload_snapshots();

        if errors.is_empty() {
            let detail = if warnings.is_empty() {
                Some("新开的终端会自动读取新环境变量；已运行的终端需要重启。".to_owned())
            } else {
                Some(warnings.join("\n"))
            };
            self.set_toast(
                if warnings.is_empty() {
                    ToastKind::Success
                } else {
                    ToastKind::Warning
                },
                format!("已切换到 {target_name}"),
                detail,
            );
            self.log.success(format!("切换到 {target_name} 完成"));
        } else {
            self.toast_error("切换未完全成功", errors.join("\n"));
        }
    }

    fn create_manual_backup(&mut self) {
        match backup::create_snapshot(&self.paths, SnapshotKind::Manual, "手动备份") {
            Ok(outcome) => {
                self.reload_snapshots();
                self.log
                    .success(format!("手动备份 {}", outcome.snapshot.id));
                self.set_toast(
                    ToastKind::Success,
                    "备份已创建",
                    Some(format!(
                        "{} · {}",
                        outcome.snapshot.display_time(),
                        backup::summary(&outcome.snapshot)
                    )),
                );
            }
            Err(error) => {
                self.toast_error("创建备份失败", error.to_string());
                self.log.error(format!("创建备份失败：{error}"));
            }
        }
    }

    fn perform_restore(&mut self, id: &str) {
        let Some(snapshot) = self.snapshots.iter().find(|s| s.id == id).cloned() else {
            self.toast_error("备份不存在", "该备份可能已被删除。");
            return;
        };
        match backup::create_snapshot(
            &self.paths,
            SnapshotKind::PreRestore,
            &format!("恢复前 · {}", snapshot.display_time()),
        ) {
            Ok(outcome) => {
                if outcome.created {
                    self.log
                        .info(format!("恢复前自动备份 {}", outcome.snapshot.id));
                }
            }
            Err(error) => {
                self.toast_error("无法创建恢复前备份", error.to_string());
                return;
            }
        }

        let outcome = backup::restore(&self.paths, &snapshot);
        self.refresh_active();
        self.reload_snapshots();
        self.detail = None;

        if outcome.ok() {
            if outcome.warnings.is_empty() {
                self.set_toast(
                    ToastKind::Success,
                    "配置已恢复",
                    Some("请打开新的终端，让 Rustup 环境变量生效。".to_owned()),
                );
            } else {
                self.set_toast(
                    ToastKind::Warning,
                    "配置已恢复，但有提示",
                    Some(outcome.warnings.join("\n")),
                );
            }
            self.log.success(format!("已恢复备份 {}", snapshot.id));
        } else {
            self.toast_error("恢复未完全成功", outcome.errors.join("\n"));
            self.log.error(format!(
                "恢复备份 {} 失败：{}",
                snapshot.id,
                outcome.errors.join("; ")
            ));
        }
    }

    fn perform_delete(&mut self, id: &str) {
        match backup::delete(&self.paths, id) {
            Ok(()) => {
                self.reload_snapshots();
                self.detail = None;
                self.log.info(format!("已删除备份 {id}"));
                self.set_toast(ToastKind::Success, "备份已删除", None);
            }
            Err(error) => {
                self.toast_error("删除备份失败", error.to_string());
            }
        }
    }

    fn start_tests(&mut self, scope: TestScope) {
        if self.speed.running {
            return;
        }
        self.page = Page::Speed;
        self.test_scope = scope;
        let kinds = scope.kinds();
        self.speed
            .results
            .retain(|result| !kinds.contains(&result.kind));
        let targets = speedtest::targets_for(&kinds, self.settings.include_official);
        if targets.is_empty() {
            self.toast_error("没有可测速的镜像", "请检查镜像列表。");
            return;
        }
        let total = targets.len();
        let sample_bytes = self.settings.sample_bytes();
        let cancel = Arc::new(AtomicBool::new(false));
        let receiver = speedtest::spawn_tests(
            targets,
            sample_bytes,
            self.settings.test_concurrency,
            Arc::clone(&cancel),
        );
        self.speed.running = true;
        self.speed.cancelling = false;
        self.speed.total = total;
        self.speed.completed = 0;
        self.speed.active.clear();
        self.speed.receiver = Some(receiver);
        self.speed.cancel = cancel;
        self.speed.started_at = Some(Instant::now());
        self.speed.sample_bytes = sample_bytes;
        self.log.info(format!(
            "开始测速：{}，{} 个目标，样本 {} MiB，并发 {}",
            scope.label(),
            total,
            self.settings.test_sample_mb,
            self.settings.test_concurrency
        ));
    }

    fn poll_speed(&mut self, ctx: &egui::Context) {
        if !self.speed.running {
            return;
        }
        let mut finished = false;
        if let Some(receiver) = &self.speed.receiver {
            loop {
                match receiver.try_recv() {
                    Ok(SpeedEvent::Started { id, kind }) => {
                        self.speed.active.insert((id, kind));
                    }
                    Ok(SpeedEvent::Finished(result)) => {
                        self.speed.completed += 1;
                        self.speed.active.remove(&(result.id.clone(), result.kind));
                        self.speed
                            .results
                            .retain(|r| !(r.id == result.id && r.kind == result.kind));
                        self.speed.results.push(result);
                    }
                    Ok(SpeedEvent::AllFinished) => {
                        finished = true;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                }
            }
        }

        if finished {
            let cancelled = self.speed.cancelling;
            self.speed.running = false;
            self.speed.cancelling = false;
            self.speed.active.clear();
            self.speed.receiver = None;
            self.speed.started_at = None;
            self.speed.last_tested_at = Some(crate::util::now_string());
            self.speed.last_tested_ms = Some(crate::util::epoch_millis());
            if let Err(error) = speedtest::save_cache(
                &self.paths.speed_cache_path,
                &self.speed.results,
                self.speed.sample_bytes,
            ) {
                self.log.error(format!("保存测速结果失败：{error}"));
            }
            let scope_kinds = self.test_scope.kinds();
            let successes = self
                .speed
                .results
                .iter()
                .filter(|r| r.ok && scope_kinds.contains(&r.kind))
                .count();
            let failures = self.speed.total.saturating_sub(successes);
            if successes == 0 {
                self.log.error(format!(
                    "测速{}：没有成功的目标（{} 个失败）",
                    if cancelled { "已取消" } else { "结束" },
                    failures
                ));
                self.set_toast(
                    ToastKind::Error,
                    if cancelled {
                        "测速已取消"
                    } else {
                        "测速没有成功的目标"
                    },
                    Some(
                        "请检查网络、代理或镜像可用性；失败原因可在测速对比页悬停查看。".to_owned(),
                    ),
                );
            } else {
                self.log.success(format!(
                    "测速{}：{} 个成功 / {} 个目标",
                    if cancelled { "已取消" } else { "结束" },
                    successes,
                    self.speed.total
                ));
                self.set_toast(
                    if failures > 0 || cancelled {
                        ToastKind::Warning
                    } else {
                        ToastKind::Success
                    },
                    if cancelled {
                        "测速已取消"
                    } else if failures > 0 {
                        "测速完成，部分目标失败"
                    } else {
                        "测速完成"
                    },
                    Some(format!(
                        "{} 个成功，{} 个失败；可在测速对比页查看排序与错误详情。",
                        successes, failures
                    )),
                );
            }
        } else {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }

    fn worst_startup_warning(&self) -> Option<&str> {
        self.startup_warnings.first().map(String::as_str)
    }

    fn draw_sidebar(&mut self, ctx: &egui::Context) {
        let palette = self.palette;
        egui::SidePanel::left("sidebar")
            .exact_width(230.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(palette.panel).inner_margin(Margin {
                left: 14,
                right: 14,
                top: 18,
                bottom: 14,
            }))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                    widgets::paint_logo(ui, rect, palette);
                    ui.add_space(2.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("MirrorSwitch")
                                .size(16.0)
                                .color(palette.text_strong)
                                .strong(),
                        );
                        ui.label(
                            RichText::new("Rust 镜像管家")
                                .size(10.5)
                                .color(palette.faint),
                        );
                    });
                });
                ui.add_space(24.0);
                ui.label(
                    RichText::new("工作台")
                        .size(10.0)
                        .color(palette.faint)
                        .strong(),
                );
                ui.add_space(6.0);

                let nav = [
                    (Page::Overview, NavIcon::Overview, "概览"),
                    (Page::Mirrors, NavIcon::Mirrors, "镜像源"),
                    (Page::Speed, NavIcon::Speed, "测速对比"),
                    (Page::Backups, NavIcon::Backup, "备份恢复"),
                    (Page::Settings, NavIcon::Settings, "设置"),
                ];
                for (page, icon, label) in nav {
                    if nav_item(ui, palette, icon, label, self.page == page).clicked() {
                        self.page = page;
                    }
                    ui.add_space(2.0);
                }

                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.label(
                        RichText::new(format!("v{APP_VERSION} · egui 0.32"))
                            .size(10.0)
                            .color(palette.faint),
                    );
                    ui.add_space(10.0);
                    egui::Frame::new()
                        .fill(palette.surface)
                        .stroke(Stroke::new(1.0_f32, palette.border))
                        .corner_radius(CornerRadius::same(11))
                        .inner_margin(Margin::same(11))
                        .show(ui, |ui| {
                            ui.set_width(178.0);
                            ui.horizontal(|ui| {
                                dot(
                                    ui,
                                    self.active
                                        .cargo_mirror_id
                                        .map(provider_color)
                                        .unwrap_or(palette.faint),
                                    3.5,
                                );
                                ui.label(RichText::new("CARGO").size(9.5).color(palette.faint));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(ellipsize(&self.active.cargo_label, 15))
                                            .size(10.5)
                                            .color(palette.text),
                                    );
                                });
                            });
                            ui.add_space(5.0);
                            ui.horizontal(|ui| {
                                dot(
                                    ui,
                                    self.active
                                        .rustup_mirror_id
                                        .map(provider_color)
                                        .unwrap_or(palette.faint),
                                    3.5,
                                );
                                ui.label(RichText::new("RUSTUP").size(9.5).color(palette.faint));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(ellipsize(&self.active.rustup_label, 15))
                                            .size(10.5)
                                            .color(palette.text),
                                    );
                                });
                            });
                        });
                    ui.add_space(4.0);
                });
            });
    }

    fn draw_main(&mut self, ctx: &egui::Context, actions: &mut Vec<UiAction>) {
        let palette = self.palette;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(palette.bg)
                    .inner_margin(Margin::symmetric(26, 18)),
            )
            .show(ctx, |ui| {
                page_header(ui, palette, self.page.title(), self.page.subtitle(), |ui| {
                    if theme_toggle(ui, palette).clicked() {
                        actions.push(UiAction::ThemeToggle);
                    }
                    ui.add_space(4.0);
                    if secondary_button(ui, "刷新").clicked() {
                        actions.push(UiAction::Refresh);
                    }
                });
                let mut scroll_area = egui::ScrollArea::vertical()
                    .id_salt("main-scroll")
                    .auto_shrink([false, false]);
                if self.screenshot_path.is_some() {
                    if let Ok(offset) = std::env::var("MIRRORSWITCH_SCROLL") {
                        if let Ok(offset) = offset.parse::<f32>() {
                            scroll_area = scroll_area.vertical_scroll_offset(offset);
                        }
                    }
                }
                scroll_area.show(ui, |ui| {
                    // Leave room for the overlay scrollbar and a little
                    // breathing space; otherwise equal-width card rows can
                    // touch the window edge.
                    let width = (ui.available_width() - 30.0).max(320.0);
                    ui.set_width(width);
                    match self.page {
                        Page::Overview => self.overview_page(ui, actions),
                        Page::Mirrors => self.mirrors_page(ui, actions),
                        Page::Speed => self.speed_page(ui, actions),
                        Page::Backups => self.backups_page(ui, actions),
                        Page::Settings => self.settings_page(ui, actions),
                    }
                    ui.add_space(28.0);
                });
            });
    }

    fn overview_page(&mut self, ui: &mut Ui, actions: &mut Vec<UiAction>) {
        let palette = self.palette;
        if let Some(warning) = self.worst_startup_warning() {
            notice(ui, palette, palette.warning, "启动提示", warning);
            ui.add_space(8.0);
        }
        for warning in self.active.warnings.iter().take(2) {
            notice(ui, palette, palette.warning, "配置提示", warning);
            ui.add_space(8.0);
        }

        self.hero_card(ui, actions);
        ui.add_space(14.0);
        self.stats_row(ui);
        ui.add_space(18.0);
        self.quick_switch(ui, actions);
        ui.add_space(18.0);

        let gap = 12.0;
        let column_width = ((ui.max_rect().width() - gap - 8.0) / 2.0).max(300.0);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(column_width, 0.0),
                    Layout::top_down(Align::LEFT),
                    |ui| {
                        ui.set_width(column_width);
                        self.overview_ranking(ui, actions);
                    },
                );
                ui.add_space(gap);
                ui.allocate_ui_with_layout(
                    Vec2::new(column_width, 0.0),
                    Layout::top_down(Align::LEFT),
                    |ui| {
                        ui.set_width(column_width);
                        self.overview_backups(ui, actions);
                    },
                );
            });
        });
        ui.add_space(14.0);
        egui::Frame::new()
            .fill(palette.surface_2)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("提示")
                            .size(11.0)
                            .color(palette.accent)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(crate::platform::rustup_restart_tip())
                            .size(11.5)
                            .color(palette.muted),
                    );
                });
            });
    }

    fn hero_card(&mut self, ui: &mut Ui, actions: &mut Vec<UiAction>) {
        let palette = self.palette;
        palette.card().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("当前配置")
                        .size(14.0)
                        .color(palette.text_strong)
                        .strong(),
                );
                chip(
                    ui,
                    "自动备份已开启",
                    palette.success,
                    palette
                        .success
                        .gamma_multiply(if palette.dark { 0.14 } else { 0.09 }),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if secondary_button(ui, "立即测速").clicked() {
                        actions.push(UiAction::RunTests(TestScope::Both));
                    }
                    if ghost_button(ui, "查看镜像源 →").clicked() {
                        self.page = Page::Mirrors;
                    }
                });
            });
            ui.add_space(12.0);
            let gap = 12.0;
            let width = ((ui.max_rect().width() - gap) / 2.0).max(260.0);
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(width, 0.0),
                        Layout::top_down(Align::LEFT),
                        |ui| {
                            self.source_summary(ui, MirrorKind::Cargo, actions);
                        },
                    );
                    ui.add_space(gap);
                    ui.allocate_ui_with_layout(
                        Vec2::new(width, 0.0),
                        Layout::top_down(Align::LEFT),
                        |ui| {
                            self.source_summary(ui, MirrorKind::Rustup, actions);
                        },
                    );
                });
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!(
                        "Cargo 配置 · {}",
                        short_path(&self.paths.cargo_config, 62)
                    ))
                    .size(10.5)
                    .color(palette.faint),
                );
                ui.add_space(10.0);
                if ghost_button(ui, "打开配置目录").clicked() {
                    actions.push(UiAction::Reveal(
                        self.paths
                            .cargo_config
                            .parent()
                            .unwrap_or(&self.paths.cargo_home)
                            .to_path_buf(),
                    ));
                }
            });
        });
    }

    fn source_summary(&mut self, ui: &mut Ui, kind: MirrorKind, actions: &mut Vec<UiAction>) {
        let palette = self.palette;
        ui.spacing_mut().item_spacing.x = 8.0;
        let (label, endpoint, mirror_id, official, color) = match kind {
            MirrorKind::Cargo => (
                self.active.cargo_label.clone(),
                self.active.cargo_endpoint(),
                self.active.cargo_mirror_id,
                self.active.cargo.is_official(),
                self.active
                    .cargo_mirror_id
                    .map(provider_color)
                    .unwrap_or(provider_color("official")),
            ),
            MirrorKind::Rustup => (
                self.active.rustup_label.clone(),
                self.active.rustup_endpoint(),
                self.active.rustup_mirror_id,
                self.active.rustup.is_official(),
                self.active
                    .rustup_mirror_id
                    .map(provider_color)
                    .unwrap_or(provider_color("official")),
            ),
        };
        let _ = mirror_id;
        palette.soft_card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(9), color.gamma_multiply(0.16));
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    if kind == MirrorKind::Cargo { "C" } else { "R" },
                    FontId::proportional(14.0),
                    color,
                );
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(kind.label())
                            .size(10.0)
                            .color(palette.faint)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(&label)
                            .size(15.0)
                            .color(palette.text_strong)
                            .strong(),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    chip(
                        ui,
                        if official { "官方" } else { "镜像" },
                        if official { palette.muted } else { color },
                        if official {
                            palette.surface_3
                        } else {
                            color.gamma_multiply(0.15)
                        },
                    );
                });
            });
            ui.add_space(7.0);
            ui.label(
                RichText::new(ellipsize(&endpoint, 54))
                    .size(10.5)
                    .color(palette.muted),
            )
            .on_hover_text(&endpoint);
            ui.add_space(7.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if secondary_button(ui, "管理镜像").clicked() {
                    self.page = Page::Mirrors;
                }
                ui.label(
                    RichText::new(kind.description())
                        .size(10.5)
                        .color(palette.faint),
                );
            });
        });
        let _ = actions;
    }

    fn stats_row(&self, ui: &mut Ui) {
        let palette = self.palette;
        let best_cargo = self
            .speed
            .results
            .iter()
            .filter(|r| r.kind == MirrorKind::Cargo && r.ok)
            .max_by(|a, b| a.speed_bps.total_cmp(&b.speed_bps));
        let best_latency = self
            .speed
            .results
            .iter()
            .filter(|r| r.ok)
            .min_by(|a, b| a.latency_ms.total_cmp(&b.latency_ms));
        let (speed_value, speed_detail, speed_color) = match best_cargo {
            Some(result) => (
                format_speed(result.speed_bps),
                format!("{} · 当前最快 Cargo", result.display_name()),
                provider_color(&result.id),
            ),
            None => ("—".to_owned(), "尚未测速".to_owned(), palette.faint),
        };
        let (latency_value, latency_detail, latency_color) = match best_latency {
            Some(result) => (
                format_ms(result.latency_ms),
                format!("{} · 全部镜像中最低", result.display_name()),
                provider_color(&result.id),
            ),
            None => ("—".to_owned(), "尚未测速".to_owned(), palette.faint),
        };
        let backups_size = backup::backup_dir_size(&self.paths);

        let gap = 10.0;
        let width = ((ui.max_rect().width() - gap * 3.0 - 8.0) / 4.0).max(150.0);
        let last_time = self
            .speed
            .last_tested_at
            .as_deref()
            .map(|time| {
                time.split(' ')
                    .nth(1)
                    .map(|t| t[..t.len().min(5)].to_owned())
                    .unwrap_or_else(|| time.to_owned())
            })
            .unwrap_or_else(|| "—".to_owned());
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal_top(|ui| {
                stat_card(
                    ui,
                    palette,
                    width,
                    "最快下载",
                    &speed_value,
                    &speed_detail,
                    speed_color,
                );
                ui.add_space(gap);
                stat_card(
                    ui,
                    palette,
                    width,
                    "最低延迟",
                    &latency_value,
                    &latency_detail,
                    latency_color,
                );
                ui.add_space(gap);
                stat_card(
                    ui,
                    palette,
                    width,
                    "配置备份",
                    &format!("{} 份", self.snapshots.len()),
                    &format!("占用 {}", format_bytes(backups_size)),
                    palette.info,
                );
                ui.add_space(gap);
                stat_card(
                    ui,
                    palette,
                    width,
                    "最近测速",
                    &last_time,
                    self.speed
                        .last_tested_at
                        .as_deref()
                        .map(|_| "本地时间")
                        .unwrap_or("运行一次测速看看"),
                    palette.warning,
                );
            });
        });
    }

    fn quick_switch(&mut self, ui: &mut Ui, actions: &mut Vec<UiAction>) {
        let palette = self.palette;
        section_title(ui, palette, "快速切换", Some("切换前自动备份"));
        let ids = ["rsproxy", "ustc", "tuna"];
        let gap = 12.0;
        let width = ((ui.max_rect().width() - gap * 2.0 - 8.0) / 3.0).max(180.0);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal_top(|ui| {
                for (index, id) in ids.iter().enumerate() {
                    if index > 0 {
                        ui.add_space(gap);
                    }
                    if let Some(mirror) = mirror_by_id(id) {
                        self.quick_mirror_card(ui, mirror, width, actions);
                    }
                }
            });
        });
    }

    fn quick_mirror_card(
        &self,
        ui: &mut Ui,
        mirror: &'static Mirror,
        width: f32,
        actions: &mut Vec<UiAction>,
    ) {
        let palette = self.palette;
        let color = provider_color(mirror.id);
        let active_both = self.active.mirror_active_for(mirror.id, MirrorKind::Cargo)
            && self.active.mirror_active_for(mirror.id, MirrorKind::Rustup);
        let best = self
            .speed
            .results
            .iter()
            .find(|r| r.id == mirror.id && r.ok);
        ui.allocate_ui_with_layout(
            Vec2::new(width, 138.0),
            Layout::top_down(Align::LEFT),
            |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.set_width(width);
                egui::Frame::new()
                    .fill(palette.surface)
                    .stroke(Stroke::new(
                        1.0_f32,
                        if active_both { color } else { palette.border },
                    ))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(13))
                    .show(ui, |ui| {
                        ui.set_width((width - 26.0).max(80.0));
                        ui.horizontal(|ui| {
                            dot(ui, color, 3.5);
                            ui.label(
                                RichText::new(mirror.name)
                                    .size(13.0)
                                    .color(palette.text_strong)
                                    .strong(),
                            );
                            if active_both {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    chip(
                                        ui,
                                        "启用",
                                        palette.success,
                                        palette.success.gamma_multiply(0.14),
                                    );
                                });
                            }
                        });
                        ui.add_space(3.0);
                        ui.label(
                            RichText::new(mirror.summary)
                                .size(10.5)
                                .color(palette.muted),
                        );
                        ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                            ui.add_space(3.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(
                                        best.map(|r| format_speed(r.speed_bps))
                                            .unwrap_or_else(|| "未测速".to_owned()),
                                    )
                                    .size(10.5)
                                    .color(if best.is_some() {
                                        palette.accent
                                    } else {
                                        palette.faint
                                    }),
                                );
                                if !active_both {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let kinds = if mirror.supports(MirrorKind::Cargo) {
                                            vec![MirrorKind::Cargo, MirrorKind::Rustup]
                                        } else {
                                            vec![MirrorKind::Rustup]
                                        };
                                        let label = if mirror.supports(MirrorKind::Cargo) {
                                            "应用"
                                        } else {
                                            "应用 Rustup"
                                        };
                                        if secondary_button(ui, label).clicked() {
                                            actions.push(UiAction::Switch {
                                                mirror_id: Some(mirror.id.to_owned()),
                                                kinds,
                                                confirmed: false,
                                            });
                                        }
                                    });
                                }
                            });
                        });
                    });
            },
        );
    }

    fn overview_ranking(&mut self, ui: &mut Ui, actions: &mut Vec<UiAction>) {
        let palette = self.palette;
        ui.spacing_mut().item_spacing.x = 8.0;
        palette.card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Cargo 测速排行")
                        .size(13.5)
                        .color(palette.text_strong)
                        .strong(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ghost_button(ui, "完整对比 →").clicked() {
                        self.page = Page::Speed;
                    }
                });
            });
            ui.add_space(8.0);
            let mut results: Vec<&SpeedResult> = self
                .speed
                .results
                .iter()
                .filter(|r| r.kind == MirrorKind::Cargo && r.ok)
                .collect();
            results.sort_by(|a, b| b.score.total_cmp(&a.score));
            if results.is_empty() {
                empty_state(
                    ui,
                    palette,
                    "还没有 Cargo 测速结果",
                    "运行一次测速，比较镜像与官方源。",
                    92.0,
                );
            } else {
                for (index, result) in results.iter().take(4).enumerate() {
                    let color = provider_color(&result.id);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{:02}", index + 1))
                                .size(10.0)
                                .color(palette.faint),
                        );
                        dot(ui, color, 3.0);
                        ui.label(
                            RichText::new(result.display_name())
                                .size(12.0)
                                .color(palette.text),
                        );
                        if result.is_official() {
                            chip(ui, "官方", palette.faint, palette.surface_2);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(format_speed(result.speed_bps))
                                    .size(11.5)
                                    .color(palette.accent)
                                    .strong(),
                            );
                            ui.label(
                                RichText::new(format_ms(result.latency_ms))
                                    .size(10.5)
                                    .color(palette.faint),
                            );
                        });
                    });
                    if index < 3 {
                        ui.add_space(4.0);
                    }
                }
            }
            let _ = actions;
        });
    }

    fn overview_backups(&mut self, ui: &mut Ui, actions: &mut Vec<UiAction>) {
        let palette = self.palette;
        ui.spacing_mut().item_spacing.x = 8.0;
        palette.card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("最近备份")
                        .size(13.5)
                        .color(palette.text_strong)
                        .strong(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ghost_button(ui, "全部备份 →").clicked() {
                        self.page = Page::Backups;
                    }
                });
            });
            ui.add_space(8.0);
            if self.snapshots.is_empty() {
                empty_state(
                    ui,
                    palette,
                    "还没有备份",
                    "切换、恢复前会自动备份，也可以手动创建。",
                    92.0,
                );
            } else {
                for (index, snapshot) in self.snapshots.iter().take(4).enumerate() {
                    ui.horizontal(|ui| {
                        chip(
                            ui,
                            snapshot.cargo_badge(),
                            match snapshot.kind {
                                SnapshotKind::Manual => palette.info,
                                SnapshotKind::PreRestore => palette.warning,
                                SnapshotKind::Auto => palette.accent,
                            },
                            palette.surface_2,
                        );
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(snapshot.display_time())
                                    .size(11.0)
                                    .color(palette.text),
                            );
                            ui.label(
                                RichText::new(ellipsize(&backup::summary(snapshot), 34))
                                    .size(9.5)
                                    .color(palette.faint),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ghost_button(ui, "恢复").clicked() {
                                actions.push(UiAction::Restore {
                                    id: snapshot.id.clone(),
                                    confirmed: false,
                                });
                            }
                        });
                    });
                    if index + 1 < self.snapshots.len().min(4) {
                        ui.add_space(4.0);
                    }
                }
            }
        });
    }

    fn mirrors_page(&mut self, ui: &mut Ui, actions: &mut Vec<UiAction>) {
        let palette = self.palette;
        ui.horizontal_wrapped(|ui| {
            for filter in [MirrorFilter::All, MirrorFilter::Cargo, MirrorFilter::Rustup] {
                let selected = self.mirror_filter == filter;
                let button = egui::Button::new(RichText::new(filter.label()).size(12.0).color(
                    if selected {
                        palette.on_accent
                    } else {
                        palette.muted
                    },
                ))
                .fill(if selected {
                    palette.accent
                } else {
                    palette.surface_2
                })
                .stroke(Stroke::new(
                    1.0_f32,
                    if selected {
                        palette.accent
                    } else {
                        palette.border
                    },
                ))
                .corner_radius(CornerRadius::same(99))
                .min_size(Vec2::new(62.0, 29.0));
                if ui.add(button).clicked() {
                    self.mirror_filter = filter;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if secondary_button(ui, "恢复官方源").clicked() {
                    actions.push(UiAction::Switch {
                        mirror_id: None,
                        kinds: vec![MirrorKind::Cargo, MirrorKind::Rustup],
                        confirmed: false,
                    });
                }
            });
        });
        ui.add_space(12.0);

        palette.card().show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(38.0), Sense::hover());
                ui.painter().rect_filled(
                    rect,
                    CornerRadius::same(10),
                    palette.faint.gamma_multiply(0.12),
                );
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "◎",
                    FontId::proportional(18.0),
                    palette.muted,
                );
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("官方源")
                            .size(14.0)
                            .color(palette.text_strong)
                            .strong(),
                    );
                    ui.label(
                        RichText::new("移除镜像替换，恢复 crates.io 与 static.rust-lang.org。")
                            .size(11.0)
                            .color(palette.muted),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    chip(
                        ui,
                        if self.active.cargo.is_official() {
                            "Cargo 官方"
                        } else {
                            "Cargo 已切换"
                        },
                        if self.active.cargo.is_official() {
                            palette.success
                        } else {
                            palette.faint
                        },
                        palette.surface_2,
                    );
                    chip(
                        ui,
                        if self.active.rustup.is_official() {
                            "Rustup 官方"
                        } else {
                            "Rustup 已切换"
                        },
                        if self.active.rustup.is_official() {
                            palette.success
                        } else {
                            palette.faint
                        },
                        palette.surface_2,
                    );
                });
            });
        });
        ui.add_space(14.0);

        let mirrors: Vec<&'static Mirror> = catalog::MIRRORS
            .iter()
            .filter(|mirror| self.mirror_filter.accepts(mirror))
            .collect();
        let gap = 12.0;
        let width = ((ui.max_rect().width() - gap - 8.0) / 2.0).max(330.0);
        for chunk in mirrors.chunks(2) {
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.horizontal_top(|ui| {
                    for (index, mirror) in chunk.iter().enumerate() {
                        if index > 0 {
                            ui.add_space(gap);
                        }
                        ui.allocate_ui_with_layout(
                            Vec2::new(width, 314.0),
                            Layout::top_down(Align::LEFT),
                            |ui| {
                                self.mirror_card(ui, mirror, width, actions);
                            },
                        );
                    }
                });
            });
            ui.add_space(gap);
        }
    }

    fn mirror_card(
        &self,
        ui: &mut Ui,
        mirror: &'static Mirror,
        width: f32,
        actions: &mut Vec<UiAction>,
    ) {
        let palette = self.palette;
        ui.spacing_mut().item_spacing.x = 8.0;
        let color = provider_color(mirror.id);
        let cargo_active = self.active.mirror_active_for(mirror.id, MirrorKind::Cargo);
        let rustup_active = self.active.mirror_active_for(mirror.id, MirrorKind::Rustup);
        let any_active = cargo_active || rustup_active;
        let both_supported =
            mirror.supports(MirrorKind::Cargo) && mirror.supports(MirrorKind::Rustup);
        let both_active = cargo_active && rustup_active;
        let filter = self.mirror_filter;

        ui.set_width(width);
        egui::Frame::new()
            .fill(palette.surface)
            .stroke(Stroke::new(
                1.0_f32,
                if any_active { color } else { palette.border },
            ))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.set_width((width - 32.0).max(120.0));
                ui.horizontal(|ui| {
                    dot(ui, color, 4.0);
                    ui.label(
                        RichText::new(mirror.name)
                            .size(15.0)
                            .color(palette.text_strong)
                            .strong(),
                    );
                    if mirror.recommended {
                        chip(
                            ui,
                            "推荐",
                            palette.warning,
                            palette.warning.gamma_multiply(0.13),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if both_active {
                            chip(
                                ui,
                                "全部启用",
                                palette.success,
                                palette.success.gamma_multiply(0.14),
                            );
                        } else if cargo_active {
                            chip(ui, "Cargo 启用", color, color.gamma_multiply(0.14));
                        } else if rustup_active {
                            chip(ui, "Rustup 启用", color, color.gamma_multiply(0.14));
                        } else {
                            chip(ui, "预设", palette.faint, palette.surface_2);
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new(mirror.org).size(10.0).color(palette.faint));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ghost_button(ui, "主页 ↗").clicked() {
                            actions.push(UiAction::OpenUrl(mirror.homepage.to_owned()));
                        }
                    });
                });
                ui.add_space(7.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 40.0),
                    Layout::top_down(Align::LEFT),
                    |ui| {
                        ui.label(
                            RichText::new(mirror.summary)
                                .size(11.0)
                                .color(palette.muted),
                        );
                    },
                );
                ui.add_space(7.0);
                ui.horizontal_wrapped(|ui| {
                    for tag in mirror.tags {
                        chip(ui, *tag, palette.muted, palette.surface_2);
                    }
                });
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(palette.surface_2)
                    .corner_radius(CornerRadius::same(9))
                    .inner_margin(Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if let Some(cargo) = mirror.cargo {
                            endpoint_row(ui, palette, "Cargo", cargo.index);
                        }
                        if let Some(rustup) = mirror.rustup {
                            if mirror.cargo.is_some() {
                                ui.add_space(3.0);
                            }
                            endpoint_row(ui, palette, "Rustup", rustup.dist);
                        }
                    });
                ui.add_space(10.0);
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.horizontal(|ui| {
                        if both_supported {
                            if both_active {
                                primary_button(ui, "已全部启用");
                            } else if primary_button(ui, "全部应用").clicked() {
                                actions.push(UiAction::Switch {
                                    mirror_id: Some(mirror.id.to_owned()),
                                    kinds: vec![MirrorKind::Cargo, MirrorKind::Rustup],
                                    confirmed: false,
                                });
                            }
                        } else if filter == MirrorFilter::Cargo
                            || !mirror.supports(MirrorKind::Rustup)
                        {
                            if cargo_active {
                                secondary_button(ui, "Cargo 已启用");
                            } else if primary_button(ui, "应用 Cargo").clicked() {
                                actions.push(UiAction::Switch {
                                    mirror_id: Some(mirror.id.to_owned()),
                                    kinds: vec![MirrorKind::Cargo],
                                    confirmed: false,
                                });
                            }
                        } else if rustup_active {
                            secondary_button(ui, "Rustup 已启用");
                        } else if primary_button(ui, "应用 Rustup").clicked() {
                            actions.push(UiAction::Switch {
                                mirror_id: Some(mirror.id.to_owned()),
                                kinds: vec![MirrorKind::Rustup],
                                confirmed: false,
                            });
                        }
                        if both_supported {
                            ui.add_space(4.0);
                            if !cargo_active && secondary_button(ui, "仅 Cargo").clicked() {
                                actions.push(UiAction::Switch {
                                    mirror_id: Some(mirror.id.to_owned()),
                                    kinds: vec![MirrorKind::Cargo],
                                    confirmed: false,
                                });
                            }
                            if !rustup_active && secondary_button(ui, "仅 Rustup").clicked() {
                                actions.push(UiAction::Switch {
                                    mirror_id: Some(mirror.id.to_owned()),
                                    kinds: vec![MirrorKind::Rustup],
                                    confirmed: false,
                                });
                            }
                        }
                    });
                });
            });
    }

    fn speed_page(&mut self, ui: &mut Ui, actions: &mut Vec<UiAction>) {
        let palette = self.palette;
        palette.card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("测速范围").size(11.0).color(palette.faint));
                    ui.horizontal(|ui| {
                        for scope in [TestScope::Both, TestScope::Cargo, TestScope::Rustup] {
                            let selected = self.test_scope == scope;
                            let button = egui::Button::new(
                                RichText::new(scope.label()).size(12.0).color(if selected {
                                    palette.on_accent
                                } else {
                                    palette.muted
                                }),
                            )
                            .fill(if selected {
                                palette.accent
                            } else {
                                palette.surface_2
                            })
                            .stroke(Stroke::new(
                                1.0_f32,
                                if selected {
                                    palette.accent
                                } else {
                                    palette.border
                                },
                            ))
                            .corner_radius(CornerRadius::same(8))
                            .min_size(Vec2::new(58.0, 30.0));
                            if ui.add(button).clicked() {
                                self.test_scope = scope;
                            }
                        }
                    });
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.speed.running {
                        if danger_button(ui, "取消测速").clicked() {
                            actions.push(UiAction::CancelTests);
                        }
                        ui.spinner();
                    } else if primary_button(ui, "开始测速").clicked() {
                        actions.push(UiAction::RunTests(self.test_scope));
                    }
                });
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!(
                    "样本 {} MiB · 并发 {} · 含官方源对比 · 结果会受当前网络与镜像负载影响",
                    self.settings.test_sample_mb, self.settings.test_concurrency
                ))
                .size(10.5)
                .color(palette.faint),
            );
        });
        ui.add_space(12.0);

        if self.speed.running {
            self.speed_progress(ui);
            ui.add_space(12.0);
        } else if let Some(time) = self.speed.last_tested_at.clone() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("上次测速：{time}"))
                        .size(10.5)
                        .color(palette.faint),
                );
                if self.speed.results.iter().any(|r| r.ok) && ghost_button(ui, "清空结果").clicked()
                {
                    self.speed.results.clear();
                    self.speed.last_tested_at = None;
                    self.speed.last_tested_ms = None;
                    let _ = std::fs::remove_file(&self.paths.speed_cache_path);
                }
            });
            ui.add_space(4.0);
        }

        if self.speed.results.iter().any(|r| r.ok) {
            self.speed_insight(ui);
            ui.add_space(14.0);
        }

        self.speed_kind_section(ui, MirrorKind::Cargo, actions);
        ui.add_space(16.0);
        self.speed_kind_section(ui, MirrorKind::Rustup, actions);

        ui.add_space(14.0);
        egui::Frame::new()
            .fill(palette.surface_2)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("测速方法")
                            .size(11.0)
                            .color(palette.text)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(
                            "先请求索引配置获取真实下载地址，再以 HTTP Range 下载固定大小的 crate / 工具链清单。速度包含连接与首字节时间；同一时刻的其他网络活动会影响结果。",
                        )
                        .size(11.0)
                        .color(palette.muted),
                    );
                });
            });
    }

    fn speed_progress(&self, ui: &mut Ui) {
        let palette = self.palette;
        let fraction = if self.speed.total == 0 {
            0.0
        } else {
            self.speed.completed as f32 / self.speed.total as f32
        };
        let elapsed = self
            .speed
            .started_at
            .map(|start| start.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        palette.card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(if self.speed.cancelling {
                        "正在停止…"
                    } else {
                        "正在测速"
                    })
                    .size(13.0)
                    .color(palette.text_strong)
                    .strong(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "{}/{} · {:.1}s",
                            self.speed.completed, self.speed.total, elapsed
                        ))
                        .size(10.5)
                        .color(palette.faint),
                    );
                });
            });
            ui.add_space(8.0);
            ui.add(
                egui::ProgressBar::new(fraction)
                    .fill(palette.accent)
                    .corner_radius(CornerRadius::same(4))
                    .text(format!("{:.0}%", fraction * 100.0)),
            );
            if !self.speed.active.is_empty() {
                let names: Vec<String> = self
                    .speed
                    .active
                    .iter()
                    .map(|(id, kind)| format!("{} · {}", kind.label(), speedtest::target_name(id)))
                    .collect();
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("进行中：{}", names.join("  /  ")))
                        .size(10.5)
                        .color(palette.accent),
                );
            }
        });
    }

    fn speed_insight(&self, ui: &mut Ui) {
        let palette = self.palette;
        let mut parts = Vec::new();
        for kind in MirrorKind::ALL {
            let results: Vec<&SpeedResult> = self
                .speed
                .results
                .iter()
                .filter(|r| r.kind == kind && r.ok)
                .collect();
            let best = results
                .iter()
                .max_by(|a, b| a.speed_bps.total_cmp(&b.speed_bps));
            let official = results.iter().find(|r| r.is_official());
            if let Some(best) = best {
                if let Some(official) = official {
                    if best.id != official.id && official.speed_bps > 0.0 {
                        let ratio = best.speed_bps / official.speed_bps;
                        parts.push(format!(
                            "{} 最快为 {}（{}），约为官方源的 {:.1} 倍",
                            kind.label(),
                            best.display_name(),
                            format_speed(best.speed_bps),
                            ratio
                        ));
                    } else {
                        parts.push(format!(
                            "{} 官方源当前最快（{}）",
                            kind.label(),
                            format_speed(official.speed_bps)
                        ));
                    }
                } else {
                    parts.push(format!(
                        "{} 最快为 {}（{}）",
                        kind.label(),
                        best.display_name(),
                        format_speed(best.speed_bps)
                    ));
                }
            }
            if let Some(lowest) = results
                .iter()
                .min_by(|a, b| a.latency_ms.total_cmp(&b.latency_ms))
            {
                parts.push(format!(
                    "{} 延迟最低：{} {}",
                    kind.label(),
                    lowest.display_name(),
                    format_ms(lowest.latency_ms)
                ));
            }
        }
        egui::Frame::new()
            .fill(palette.accent_soft)
            .stroke(Stroke::new(1.0_f32, palette.accent.gamma_multiply(0.35)))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::symmetric(15, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    dot(ui, palette.accent, 3.5);
                    ui.label(
                        RichText::new("本次网络结论")
                            .size(12.5)
                            .color(palette.accent)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(parts.join("；"))
                            .size(11.5)
                            .color(palette.text),
                    );
                });
            });
    }

    fn speed_kind_section(&self, ui: &mut Ui, kind: MirrorKind, actions: &mut Vec<UiAction>) {
        let palette = self.palette;
        let mut results: Vec<&SpeedResult> = self
            .speed
            .results
            .iter()
            .filter(|r| r.kind == kind)
            .collect();
        results.sort_by(|a, b| {
            b.ok.cmp(&a.ok)
                .then_with(|| b.score.total_cmp(&a.score))
                .then_with(|| b.speed_bps.total_cmp(&a.speed_bps))
        });
        let best_speed_id = results
            .iter()
            .filter(|r| r.ok)
            .max_by(|a, b| a.speed_bps.total_cmp(&b.speed_bps))
            .map(|r| r.id.as_str());
        let best_latency_id = results
            .iter()
            .filter(|r| r.ok)
            .min_by(|a, b| a.latency_ms.total_cmp(&b.latency_ms))
            .map(|r| r.id.as_str());

        section_title(
            ui,
            palette,
            &format!("{} 镜像", kind.label()),
            Some(kind.description()),
        );
        palette.card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            if results.is_empty() {
                empty_state(
                    ui,
                    palette,
                    &format!("还没有 {} 测速结果", kind.label()),
                    "点击右上角“开始测速”，与官方源一起比较。",
                    110.0,
                );
                return;
            }
            egui::Grid::new(format!("speed-grid-{}", kind.label()))
                .num_columns(6)
                .spacing([14.0, 10.0])
                .striped(true)
                .show(ui, |ui| {
                    header_cell(ui, palette, "#", 26.0);
                    header_cell(ui, palette, "镜像", 190.0);
                    header_cell(ui, palette, "延迟", 76.0);
                    header_cell(ui, palette, "下载速度", 110.0);
                    header_cell(ui, palette, "综合评分", 160.0);
                    header_cell(ui, palette, "", 52.0);
                    ui.end_row();

                    for (index, result) in results.iter().enumerate() {
                        ui.label(
                            RichText::new(if result.ok {
                                format!("{:02}", index + 1)
                            } else {
                                "—".to_owned()
                            })
                            .size(10.0)
                            .color(palette.faint),
                        );
                        ui.allocate_ui_with_layout(
                            Vec2::new(190.0, 28.0),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                dot(ui, provider_color(&result.id), 3.5);
                                ui.label(
                                    RichText::new(result.display_name())
                                        .size(12.0)
                                        .color(palette.text)
                                        .strong(),
                                );
                                if result.is_official() {
                                    chip(ui, "官方", palette.faint, palette.surface_2);
                                }
                                if best_speed_id == Some(result.id.as_str()) && result.ok {
                                    chip(
                                        ui,
                                        "最快",
                                        palette.warning,
                                        palette.warning.gamma_multiply(0.13),
                                    );
                                } else if best_latency_id == Some(result.id.as_str()) && result.ok {
                                    chip(
                                        ui,
                                        "低延迟",
                                        palette.info,
                                        palette.info.gamma_multiply(0.13),
                                    );
                                }
                                if self.active.mirror_active_for(&result.id, kind) {
                                    chip(
                                        ui,
                                        "使用中",
                                        palette.success,
                                        palette.success.gamma_multiply(0.12),
                                    );
                                }
                            },
                        );
                        right_cell(
                            ui,
                            if result.latency_ms > 0.0 {
                                format_ms(result.latency_ms)
                            } else {
                                "—".to_owned()
                            },
                            palette.muted,
                            76.0,
                        );
                        right_cell(
                            ui,
                            if result.ok {
                                format_speed(result.speed_bps)
                            } else {
                                "—".to_owned()
                            },
                            if result.ok {
                                palette.accent
                            } else {
                                palette.faint
                            },
                            110.0,
                        );
                        ui.allocate_ui_with_layout(
                            Vec2::new(160.0, 28.0),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                if result.ok {
                                    ui.allocate_ui_with_layout(
                                        Vec2::new(112.0, 20.0),
                                        Layout::top_down(Align::LEFT),
                                        |ui| {
                                            metric_bar(
                                                ui,
                                                palette,
                                                (result.score / 100.0) as f32,
                                                provider_color(&result.id),
                                            );
                                        },
                                    );
                                    ui.label(
                                        RichText::new(format!("{:.0}", result.score))
                                            .size(10.5)
                                            .color(palette.muted),
                                    );
                                } else {
                                    ui.label(
                                        RichText::new("失败")
                                            .size(10.5)
                                            .color(palette.danger)
                                            .strong(),
                                    )
                                    .on_hover_text(result.error.as_deref().unwrap_or("未知错误"));
                                }
                            },
                        );
                        ui.allocate_ui_with_layout(
                            Vec2::new(52.0, 28.0),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                if result.ok {
                                    if self.active.mirror_active_for(&result.id, kind) {
                                        ui.label(
                                            RichText::new("已用").size(10.0).color(palette.success),
                                        );
                                    } else if ghost_button(ui, "应用").clicked() {
                                        actions.push(UiAction::Switch {
                                            mirror_id: if result.is_official() {
                                                None
                                            } else {
                                                Some(result.id.clone())
                                            },
                                            kinds: vec![kind],
                                            confirmed: false,
                                        });
                                    }
                                }
                            },
                        );
                        ui.end_row();
                    }
                });
            if results.iter().any(|r| !r.ok) {
                let failed = results.iter().filter(|r| !r.ok).count();
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(
                        "{failed} 个目标失败；将鼠标悬停在“失败”上可查看原因。"
                    ))
                    .size(10.0)
                    .color(palette.faint),
                );
            }
        });
    }

    fn backups_page(&mut self, ui: &mut Ui, actions: &mut Vec<UiAction>) {
        let palette = self.palette;
        palette.card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("配置快照")
                            .size(14.0)
                            .color(palette.text_strong)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(
                            "保存 Cargo 配置、Rustup 镜像变量；macOS/Linux 还会保存完整的 Shell 配置文件。恢复前会再备份当前状态。",
                        )
                        .size(11.0)
                        .color(palette.muted),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if primary_button(ui, "立即备份").clicked() {
                        actions.push(UiAction::CreateBackup);
                    }
                    if secondary_button(ui, "打开目录").clicked() {
                        actions.push(UiAction::Reveal(self.paths.backup_dir.clone()));
                    }
                });
            });
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                path_chip(
                    ui,
                    palette,
                    "Cargo",
                    &self.paths.cargo_config.display().to_string(),
                    &self.paths.cargo_config,
                    actions,
                );
                path_chip(
                    ui,
                    palette,
                    "备份",
                    &self.paths.backup_dir.display().to_string(),
                    &self.paths.backup_dir,
                    actions,
                );
            });
        });
        ui.add_space(12.0);

        let automatic = self
            .snapshots
            .iter()
            .filter(|s| s.kind.is_automatic())
            .count();
        let manual = self
            .snapshots
            .iter()
            .filter(|s| s.kind == SnapshotKind::Manual)
            .count();
        let gap = 10.0;
        let width = ((ui.max_rect().width() - gap * 2.0 - 8.0) / 3.0).max(180.0);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal_top(|ui| {
                stat_card(
                    ui,
                    palette,
                    width,
                    "全部备份",
                    &format!("{} 份", self.snapshots.len()),
                    &format!(
                        "占用 {}",
                        format_bytes(backup::backup_dir_size(&self.paths))
                    ),
                    palette.accent,
                );
                ui.add_space(gap);
                stat_card(
                    ui,
                    palette,
                    width,
                    "自动备份",
                    &format!("{automatic} 份"),
                    "启动、切换、恢复前",
                    palette.info,
                );
                ui.add_space(gap);
                stat_card(
                    ui,
                    palette,
                    width,
                    "手动备份",
                    &format!("{manual} 份"),
                    "永久保留，不自动清理",
                    palette.warning,
                );
            });
        });
        ui.add_space(14.0);

        if !self.snapshot_errors.is_empty() {
            notice(
                ui,
                palette,
                palette.danger,
                "备份读取异常",
                &self.snapshot_errors.join("；"),
            );
            ui.add_space(10.0);
        }

        if self.snapshots.is_empty() {
            palette.card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                empty_state(
                    ui,
                    palette,
                    "还没有配置备份",
                    "点击“立即备份”，或直接切换镜像——工具会在写入前自动留档。",
                    160.0,
                );
            });
            return;
        }

        for snapshot in &self.snapshots {
            palette.card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (color, label) = match snapshot.kind {
                        SnapshotKind::Auto => (palette.accent, "自动"),
                        SnapshotKind::Manual => (palette.info, "手动"),
                        SnapshotKind::PreRestore => (palette.warning, "恢复前"),
                    };
                    chip(ui, label, color, color.gamma_multiply(0.13));
                    ui.label(
                        RichText::new(snapshot.display_time())
                            .size(12.5)
                            .color(palette.text_strong)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(relative_time(snapshot.created_at_ms))
                            .size(10.0)
                            .color(palette.faint),
                    );
                    if !snapshot.note.is_empty() {
                        ui.label(
                            RichText::new(&snapshot.note)
                                .size(10.5)
                                .color(palette.muted),
                        );
                    }
                });
                ui.add_space(5.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!("Cargo · {}", snapshot.cargo.label))
                            .size(11.5)
                            .color(palette.text),
                    );
                    ui.label(RichText::new("|").size(11.0).color(palette.border));
                    ui.label(
                        RichText::new(format!("Rustup · {}", snapshot.rustup.label))
                            .size(11.5)
                            .color(palette.text),
                    );
                    ui.label(
                        RichText::new(format!(
                            "· config {}",
                            if snapshot.cargo.existed {
                                format_bytes(snapshot.cargo_bytes() as u64)
                            } else {
                                "不存在".to_owned()
                            }
                        ))
                        .size(10.0)
                        .color(palette.faint),
                    );
                    if let Some(shell) = &snapshot.shell {
                        ui.label(
                            RichText::new(format!(
                                "· Shell {}",
                                if shell.existed {
                                    "已保存"
                                } else {
                                    "无文件"
                                }
                            ))
                            .size(10.0)
                            .color(palette.faint),
                        );
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if primary_button(ui, "恢复此备份").clicked() {
                        actions.push(UiAction::Restore {
                            id: snapshot.id.clone(),
                            confirmed: false,
                        });
                    }
                    if secondary_button(ui, "查看详情").clicked() {
                        actions.push(UiAction::Detail(snapshot.id.clone()));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ghost_button(ui, "删除").clicked() {
                            actions.push(UiAction::Delete {
                                id: snapshot.id.clone(),
                                confirmed: false,
                            });
                        }
                        let file_size =
                            std::fs::metadata(backup::snapshot_file(&self.paths, &snapshot.id))
                                .map(|meta| meta.len())
                                .unwrap_or(0);
                        ui.label(
                            RichText::new(format!("备份文件 {}", format_bytes(file_size)))
                                .size(9.0)
                                .color(palette.faint),
                        );
                    });
                });
            });
            ui.add_space(8.0);
        }
    }

    fn settings_page(&mut self, ui: &mut Ui, actions: &mut Vec<UiAction>) {
        let palette = self.palette;

        settings_card(ui, palette, "外观", |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("主题").size(12.0).color(palette.muted));
                for theme in [ThemeMode::Dark, ThemeMode::Light] {
                    let selected = self.settings.theme == theme;
                    let button = egui::Button::new(RichText::new(theme.label()).size(12.0).color(
                        if selected {
                            palette.on_accent
                        } else {
                            palette.muted
                        },
                    ))
                    .fill(if selected {
                        palette.accent
                    } else {
                        palette.surface_2
                    })
                    .stroke(Stroke::new(1.0_f32, palette.border))
                    .corner_radius(CornerRadius::same(8));
                    if ui.add(button).clicked() {
                        self.settings.theme = theme;
                    }
                }
                ui.label(
                    RichText::new("主题会立即应用并保存。")
                        .size(10.5)
                        .color(palette.faint),
                );
            });
        });
        ui.add_space(10.0);

        settings_card(ui, palette, "测速", |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("下载样本").size(12.0).color(palette.muted));
                ui.add(
                    egui::Slider::new(&mut self.settings.test_sample_mb, 1..=8)
                        .suffix(" MiB")
                        .fixed_decimals(0),
                );
                ui.label(
                    RichText::new("越大越稳定，但耗时和流量更高。")
                        .size(10.5)
                        .color(palette.faint),
                );
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("并发数").size(12.0).color(palette.muted));
                ui.add(
                    egui::Slider::new(&mut self.settings.test_concurrency, 1..=6).fixed_decimals(0),
                );
                ui.label(
                    RichText::new("串行（1）更公平，并发（3-4）更快。")
                        .size(10.5)
                        .color(palette.faint),
                );
            });
            widgets::toggle(
                ui,
                palette,
                &mut self.settings.include_official,
                "将官方源作为对比基线",
            );
            ui.label(
                RichText::new(format!(
                    "当前样本上限：{}；测速会固定请求 crate windows-sys 0.59.0 的前若干 MiB。",
                    format_bytes(self.settings.sample_bytes())
                ))
                .size(10.0)
                .color(palette.faint),
            );
        });
        ui.add_space(10.0);

        settings_card(ui, palette, "备份", |ui| {
            widgets::toggle(
                ui,
                palette,
                &mut self.settings.auto_backup,
                "启动时自动备份（切换、恢复前始终自动备份）",
            );
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("自动备份保留")
                        .size(12.0)
                        .color(palette.muted),
                );
                ui.add(
                    egui::Slider::new(&mut self.settings.max_auto_backups, 5..=100)
                        .suffix(" 份")
                        .fixed_decimals(0),
                );
                ui.label(
                    RichText::new("手动备份永不被自动清理。")
                        .size(10.5)
                        .color(palette.faint),
                );
            });
        });
        ui.add_space(10.0);

        settings_card(ui, palette, "Rustup 环境", |ui| {
            environment_row(ui, palette, "RUSTUP_DIST_SERVER", &self.active.rustup.dist);
            ui.add_space(4.0);
            environment_row(
                ui,
                palette,
                "RUSTUP_UPDATE_ROOT",
                &self.active.rustup.update,
            );
            if let Some(profile) = crate::platform::shell_profile_path() {
                ui.add_space(4.0);
                settings_path_row(
                    ui,
                    palette,
                    "Shell 配置",
                    &profile.display().to_string(),
                    &profile,
                    actions,
                );
            }
            ui.add_space(6.0);
            if self.active.rustup_has_system_override() {
                notice(
                    ui,
                    palette,
                    palette.warning,
                    "系统级覆盖",
                    "HKLM 中的系统变量优先于这里的用户变量。若要完全恢复官方源，请以管理员身份清理系统环境变量。",
                );
            } else {
                ui.label(
                    RichText::new(format!(
                        "持久化位置：{}。{}",
                        crate::platform::storage_note(),
                        crate::platform::rustup_restart_tip()
                    ))
                    .size(10.0)
                    .color(palette.faint),
                );
            }
        });
        ui.add_space(10.0);

        settings_card(ui, palette, "路径", |ui| {
            settings_path_row(
                ui,
                palette,
                "Cargo 配置",
                &self.paths.cargo_config.display().to_string(),
                &self.paths.cargo_config,
                actions,
            );
            settings_path_row(
                ui,
                palette,
                "Cargo Home",
                &self.paths.cargo_home.display().to_string(),
                &self.paths.cargo_home,
                actions,
            );
            settings_path_row(
                ui,
                palette,
                "Rustup Home",
                &self.paths.rustup_home.display().to_string(),
                &self.paths.rustup_home,
                actions,
            );
            settings_path_row(
                ui,
                palette,
                "数据目录",
                &self.paths.data_dir.display().to_string(),
                &self.paths.data_dir,
                actions,
            );
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("界面字体")
                        .size(10.0)
                        .color(palette.faint)
                        .strong(),
                );
                ui.label(
                    RichText::new(
                        self.font_path
                            .as_deref()
                            .unwrap_or("未检测到中文字体（界面可能显示方框）"),
                    )
                    .font(FontId::monospace(9.5))
                    .color(palette.text),
                );
            });
        });
        ui.add_space(10.0);

        settings_card(ui, palette, "操作日志", |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{}", self.log.path().display()))
                        .size(10.0)
                        .color(palette.faint),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ghost_button(ui, "清空").clicked() {
                        actions.push(UiAction::ClearLog);
                    }
                    if secondary_button(ui, "打开日志").clicked() {
                        actions.push(UiAction::Reveal(self.log.path().to_path_buf()));
                    }
                });
            });
            ui.add_space(5.0);
            let log_width = (ui.available_width() - 10.0).max(200.0);
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 230.0),
                Layout::top_down(Align::LEFT),
                |ui| {
                    ui.set_width(log_width);
                    egui::ScrollArea::vertical()
                        .id_salt("log-scroll")
                        .max_height(230.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(log_width);
                            let entries = self.log.recent(120);
                            if entries.is_empty() {
                                ui.label(RichText::new("暂无日志").size(11.0).color(palette.faint));
                            }
                            for entry in entries {
                                let color = match entry.level {
                                    LogLevel::Info => palette.muted,
                                    LogLevel::Success => palette.success,
                                    LogLevel::Warn => palette.warning,
                                    LogLevel::Error => palette.danger,
                                };
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(entry.time)
                                            .font(FontId::monospace(9.5))
                                            .color(palette.faint),
                                    );
                                    ui.label(
                                        RichText::new(format!("[{}]", entry.level.label()))
                                            .font(FontId::monospace(9.5))
                                            .color(color),
                                    );
                                });
                                ui.label(
                                    RichText::new(entry.message)
                                        .font(FontId::proportional(10.5))
                                        .color(palette.text),
                                );
                                ui.add_space(3.0);
                            }
                        });
                },
            );
        });
        ui.add_space(10.0);

        if !self.startup_warnings.is_empty() || !self.snapshot_errors.is_empty() {
            settings_card(ui, palette, "启动诊断", |ui| {
                for warning in self
                    .startup_warnings
                    .iter()
                    .chain(self.snapshot_errors.iter())
                {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("•").size(11.0).color(palette.warning));
                        ui.label(RichText::new(warning).size(11.0).color(palette.muted));
                    });
                }
            });
            ui.add_space(10.0);
        }

        palette.card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                widgets::paint_logo(ui, rect, palette);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("MirrorSwitch")
                            .size(14.0)
                            .color(palette.text_strong)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(format!(
                            "v{APP_VERSION} · Rust + egui · MIT License"
                        ))
                        .size(10.5)
                        .color(palette.muted),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if secondary_button(ui, "RsProxy 主页").clicked() {
                        actions.push(UiAction::OpenUrl("https://rsproxy.cn".to_owned()));
                    }
                    if secondary_button(ui, "Rustup 文档").clicked() {
                        actions.push(UiAction::OpenUrl(
                            "https://rust-lang.github.io/rustup/".to_owned(),
                        ));
                    }
                });
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "本工具只修改 Cargo 的 source 替换项与 Rustup 镜像环境配置（Windows 用户变量 / macOS Shell 托管块），不会修改 toolchain、SSH 密钥或 credentials.toml。",
                )
                .size(10.5)
                .color(palette.faint),
            );
        });
    }

    fn show_confirm_dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<UiAction>) {
        let Some(dialog) = self.confirm.take() else {
            return;
        };
        let palette = self.palette;
        let mut close = false;
        let mut confirmed = false;
        let modal = egui::Modal::new(Id::new("confirm-dialog"))
            .backdrop_color(Color32::from_black_alpha(160))
            .frame(
                egui::Frame::new()
                    .fill(palette.panel)
                    .stroke(Stroke::new(1.0_f32, palette.border_strong))
                    .corner_radius(CornerRadius::same(16))
                    .inner_margin(Margin::same(22)),
            )
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.label(
                    RichText::new(&dialog.title)
                        .size(18.0)
                        .color(palette.text_strong)
                        .strong(),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(&dialog.message)
                        .size(12.0)
                        .color(palette.muted),
                );
                if !dialog.details.is_empty() {
                    ui.add_space(10.0);
                    egui::Frame::new()
                        .fill(palette.surface_2)
                        .corner_radius(CornerRadius::same(9))
                        .inner_margin(Margin::symmetric(12, 10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            for detail in &dialog.details {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new("·").size(11.0).color(palette.faint));
                                    ui.label(RichText::new(detail).size(11.0).color(palette.text));
                                });
                            }
                        });
                }
                ui.add_space(16.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = dialog.confirm_label.clone();
                    let clicked = if dialog.danger {
                        danger_button(ui, label).clicked()
                    } else {
                        primary_button(ui, label).clicked()
                    };
                    if clicked {
                        confirmed = true;
                        close = true;
                    }
                    if secondary_button(ui, "取消").clicked() {
                        close = true;
                    }
                });
            });
        if modal.should_close() {
            close = true;
        }
        if confirmed {
            actions.push(dialog.action.clone().into_action());
        }
        if !close {
            self.confirm = Some(dialog);
        }
    }

    fn show_detail_dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<UiAction>) {
        let Some(snapshot) = self.detail.take() else {
            return;
        };
        let palette = self.palette;
        let mut close = false;
        let mut restore = false;
        let mut preview = snapshot
            .cargo
            .content
            .clone()
            .unwrap_or_else(|| "（备份时不存在 Cargo 配置文件）".to_owned());
        let modal = egui::Modal::new(Id::new("snapshot-detail"))
            .backdrop_color(Color32::from_black_alpha(160))
            .frame(
                egui::Frame::new()
                    .fill(palette.panel)
                    .stroke(Stroke::new(1.0_f32, palette.border_strong))
                    .corner_radius(CornerRadius::same(16))
                    .inner_margin(Margin::same(22)),
            )
            .show(ctx, |ui| {
                ui.set_width(640.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("备份详情")
                            .size(18.0)
                            .color(palette.text_strong)
                            .strong(),
                    );
                    chip(
                        ui,
                        snapshot.cargo_badge(),
                        palette.accent,
                        palette.surface_2,
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(snapshot.display_time())
                                .size(10.5)
                                .color(palette.faint),
                        );
                    });
                });
                ui.add_space(8.0);
                endpoint_detail(
                    ui,
                    palette,
                    "Cargo",
                    &snapshot.cargo.label,
                    &snapshot.cargo.path,
                );
                endpoint_detail(
                    ui,
                    palette,
                    "RUSTUP_DIST_SERVER",
                    snapshot
                        .rustup
                        .dist_effective
                        .as_deref()
                        .unwrap_or("未设置（官方）"),
                    "",
                );
                endpoint_detail(
                    ui,
                    palette,
                    "RUSTUP_UPDATE_ROOT",
                    snapshot
                        .rustup
                        .update_effective
                        .as_deref()
                        .unwrap_or("未设置（官方）"),
                    "",
                );
                if let Some(shell) = &snapshot.shell {
                    endpoint_detail(
                        ui,
                        palette,
                        "Shell 配置",
                        &shell.path,
                        if shell.existed {
                            ""
                        } else {
                            "（备份时不存在）"
                        },
                    );
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Cargo 配置内容")
                        .size(11.0)
                        .color(palette.faint)
                        .strong(),
                );
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(palette.bg)
                    .stroke(Stroke::new(1.0_f32, palette.border))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .id_salt("snapshot-preview")
                            .max_height(250.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut preview)
                                        .font(egui::TextStyle::Monospace)
                                        .desired_width(f32::INFINITY)
                                        .interactive(false),
                                );
                            });
                    });
                ui.add_space(14.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if primary_button(ui, "恢复此备份").clicked() {
                        restore = true;
                        close = true;
                    }
                    if secondary_button(ui, "关闭").clicked() {
                        close = true;
                    }
                });
            });
        if modal.should_close() {
            close = true;
        }
        if restore {
            actions.push(UiAction::Restore {
                id: snapshot.id.clone(),
                confirmed: false,
            });
        }
        if !close {
            self.detail = Some(snapshot);
        }
    }

    fn show_toast(&mut self, ctx: &egui::Context) {
        let Some(toast) = &self.toast else {
            return;
        };
        if toast.created.elapsed() > Duration::from_secs(7) {
            self.toast = None;
            return;
        }
        ctx.request_repaint_after(Duration::from_secs(1));
        let palette = self.palette;
        let color = toast.kind.color(palette);
        let mut dismiss = false;
        egui::Area::new(Id::new("toast-area"))
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-22.0, -22.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(palette.panel)
                    .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.55)))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::symmetric(15, 12))
                    .show(ui, |ui| {
                        ui.set_max_width(430.0);
                        ui.horizontal_top(|ui| {
                            dot(ui, color, 4.0);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(&toast.title)
                                        .size(12.5)
                                        .color(palette.text_strong)
                                        .strong(),
                                );
                                if let Some(detail) = &toast.detail {
                                    ui.add_space(2.0);
                                    ui.label(RichText::new(detail).size(10.5).color(palette.muted));
                                }
                            });
                            ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("×").size(13.0).color(palette.faint),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    dismiss = true;
                                }
                            });
                        });
                    });
            });
        if dismiss {
            self.toast = None;
        } else {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}

impl eframe::App for MirrorSwitchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_speed(ctx);
        if self.last_refresh.elapsed() > Duration::from_secs(5) {
            self.refresh_active();
        }
        let settings_before = self.settings.clone();
        let mut actions = Vec::new();
        self.draw_sidebar(ctx);
        self.draw_main(ctx, &mut actions);
        self.handle_actions(actions, ctx);

        let mut follow_up = Vec::new();
        self.show_confirm_dialog(ctx, &mut follow_up);
        self.show_detail_dialog(ctx, &mut follow_up);
        if !follow_up.is_empty() {
            self.handle_actions(follow_up, ctx);
        }
        self.show_toast(ctx);

        if self.settings != settings_before {
            self.settings.normalize();
            self.palette = Palette::new(self.settings.theme);
            theme::apply_style(ctx, self.settings.theme);
            self.save_settings();
            let removed = backup::prune_auto(&self.paths, self.settings.max_auto_backups);
            if !removed.is_empty() {
                self.reload_snapshots();
                self.log
                    .info(format!("按保留策略清理 {} 份自动备份", removed.len()));
            }
        }
        if self.speed.running {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        if self.toast.is_some() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        self.handle_screenshot(ctx);
    }
}

fn stat_card(
    ui: &mut Ui,
    palette: Palette,
    width: f32,
    eyebrow: &str,
    value: &str,
    detail: &str,
    color: Color32,
) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, 92.0),
        Layout::top_down(Align::LEFT),
        |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.set_width(width);
            egui::Frame::new()
                .fill(palette.surface)
                .stroke(Stroke::new(1.0_f32, palette.border))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(13))
                .show(ui, |ui| {
                    ui.set_width((width - 26.0).max(80.0));
                    ui.horizontal(|ui| {
                        dot(ui, color, 3.0);
                        ui.label(RichText::new(eyebrow).size(10.0).color(palette.faint));
                    });
                    ui.label(
                        RichText::new(value)
                            .size(18.0)
                            .color(palette.text_strong)
                            .strong(),
                    );
                    ui.label(
                        RichText::new(ellipsize(detail, 30))
                            .size(9.5)
                            .color(palette.muted),
                    );
                });
        },
    );
}

fn endpoint_row(ui: &mut Ui, palette: Palette, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(9.0).color(palette.faint).strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(ellipsize(value, 45))
                    .font(FontId::monospace(9.5))
                    .color(palette.muted),
            )
            .on_hover_text(value);
        });
    });
}

fn endpoint_detail(ui: &mut Ui, palette: Palette, label: &str, value: &str, path: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(label).size(8.5).color(palette.faint).strong());
        ui.label(
            RichText::new(value)
                .font(FontId::monospace(9.5))
                .color(palette.text),
        );
        if !path.is_empty() {
            ui.label(
                RichText::new(path)
                    .font(FontId::monospace(9.0))
                    .color(palette.faint),
            );
        }
    });
}

fn path_chip(
    ui: &mut Ui,
    palette: Palette,
    label: &str,
    path: &str,
    target: &std::path::Path,
    actions: &mut Vec<UiAction>,
) {
    egui::Frame::new()
        .fill(palette.surface_2)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(9, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).size(9.0).color(palette.faint).strong());
                ui.label(
                    RichText::new(ellipsize(path, 46))
                        .font(FontId::monospace(9.0))
                        .color(palette.muted),
                )
                .on_hover_text(path);
                if ui
                    .add(
                        egui::Button::new(RichText::new("打开").size(9.0).color(palette.accent))
                            .frame(false),
                    )
                    .clicked()
                {
                    actions.push(UiAction::Reveal(target.to_path_buf()));
                }
            });
        });
}

fn settings_card(ui: &mut Ui, palette: Palette, title: &str, add_contents: impl FnOnce(&mut Ui)) {
    palette.card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            RichText::new(title)
                .size(13.0)
                .color(palette.text_strong)
                .strong(),
        );
        ui.add_space(8.0);
        add_contents(ui);
    });
}

fn environment_row(
    ui: &mut Ui,
    palette: Palette,
    label: &str,
    state: &crate::platform::EnvVarState,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(9.5).color(palette.faint).strong());
        ui.label(
            RichText::new(state.effective_display())
                .font(FontId::monospace(9.5))
                .color(if state.is_set() {
                    palette.text
                } else {
                    palette.faint
                }),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let color = match state.scope {
                Some(crate::platform::EnvScope::User) => palette.success,
                Some(crate::platform::EnvScope::Shell) => palette.success,
                Some(crate::platform::EnvScope::System) => palette.warning,
                Some(crate::platform::EnvScope::Process) => palette.info,
                _ => palette.faint,
            };
            chip(ui, state.scope_label(), color, palette.surface_2);
        });
    });
}

fn settings_path_row(
    ui: &mut Ui,
    palette: Palette,
    label: &str,
    value: &str,
    target: &std::path::Path,
    actions: &mut Vec<UiAction>,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(label)
                .size(10.0)
                .color(palette.faint)
                .strong(),
        );
        ui.label(
            RichText::new(value)
                .font(FontId::monospace(9.5))
                .color(palette.text),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ghost_button(ui, "打开").clicked() {
                actions.push(UiAction::Reveal(target.to_path_buf()));
            }
            if ghost_button(ui, "复制").clicked() {
                ui.ctx().copy_text(value.to_owned());
            }
        });
    });
}

fn header_cell(ui: &mut Ui, palette: Palette, text: &str, width: f32) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, 20.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            if !text.is_empty() {
                ui.label(RichText::new(text).size(9.5).color(palette.faint).strong());
            }
        },
    );
}

fn right_cell(ui: &mut Ui, text: String, color: Color32, width: f32) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, 26.0),
        Layout::right_to_left(Align::Center),
        |ui| {
            ui.label(RichText::new(text).size(11.5).color(color));
        },
    );
}

fn restore_confirm_dialog(snapshot: &Snapshot) -> ConfirmDialog {
    let mut details = vec![
        format!("Cargo：{}（{}）", snapshot.cargo.label, snapshot.cargo.path),
        format!("Rustup：{}", snapshot.rustup.label),
    ];
    if let Some(shell) = &snapshot.shell {
        details.push(format!("Shell 配置：{}", shell.path));
        details.push("Shell 文件将恢复为快照时的完整内容，会覆盖快照之后的修改。".to_owned());
    }
    details.push("恢复前会自动备份当前配置，因此仍可撤销本次恢复。".to_owned());
    details.push(crate::platform::rustup_restart_tip().to_owned());
    ConfirmDialog {
        title: "恢复这份配置备份？".to_owned(),
        message: format!(
            "将把 {} 的 Cargo 配置与 Rustup 镜像环境配置恢复为备份时的状态。",
            snapshot.display_time()
        ),
        details,
        confirm_label: "确认恢复".to_owned(),
        danger: false,
        action: ConfirmAction::Restore {
            id: snapshot.id.clone(),
        },
    }
}

const APP_NAME: &str = "MirrorSwitch";
