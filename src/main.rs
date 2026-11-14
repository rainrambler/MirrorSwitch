#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod backup;
mod cargo;
mod catalog;
mod logging;
mod paths;
mod platform;
mod screenshot;
mod settings;
#[cfg(any(unix, test))]
mod shell;
mod speedtest;
mod state;
mod theme;
mod util;
mod widgets;

use std::fs;

use app::MirrorSwitchApp;
use eframe::egui;
use paths::AppPaths;

fn main() {
    install_panic_hook();

    let paths = match AppPaths::discover() {
        Ok(paths) => paths,
        Err(error) => {
            let message = format!("MirrorSwitch 无法启动：{error:#}");
            eprintln!("{message}");
            let _ = fs::write(
                std::env::temp_dir().join("MirrorSwitch-startup-error.txt"),
                &message,
            );
            show_error(&message);
            return;
        }
    };

    let screenshot_mode = std::env::var_os("MIRRORSWITCH_SCREENSHOT").is_some();
    let window_size = std::env::var("MIRRORSWITCH_WINDOW")
        .ok()
        .and_then(|value| {
            let (width, height) = value.split_once('x')?;
            Some([width.parse::<f32>().ok()?, height.parse::<f32>().ok()?])
        })
        .unwrap_or(if screenshot_mode {
            [1280.0, 1000.0]
        } else {
            [1200.0, 820.0]
        });
    let viewport = egui::ViewportBuilder::default()
        .with_title("MirrorSwitch · Rust 镜像管家")
        .with_inner_size(window_size)
        .with_min_inner_size([1000.0, 680.0])
        .with_icon(app_icon());

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        vsync: true,
        ..Default::default()
    };

    if let Err(error) = eframe::run_native(
        "MirrorSwitch · Rust 镜像管家",
        options,
        Box::new(move |cc| Ok(Box::new(MirrorSwitchApp::new(cc, paths)))),
    ) {
        show_error(&format!(
            "MirrorSwitch 无法创建窗口：{error:#}\n\n请确认图形驱动与桌面会话正常。"
        ));
    }
}

#[cfg(windows)]
fn show_error(message: &str) {
    use std::ffi::c_void;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let title: Vec<u16> = "MirrorSwitch"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let body: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut::<c_void>(),
            body.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(target_os = "macos")]
fn show_error(message: &str) {
    use std::process::Command;
    let escaped = message
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let script = format!("display alert \"MirrorSwitch\" message \"{escaped}\" as critical");
    let _ = Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .status();
    eprintln!("{message}");
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn show_error(message: &str) {
    eprintln!("{message}");
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let base = std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)
            .or_else(dirs::data_dir)
            .unwrap_or_else(std::env::temp_dir);
        let path = base.join("MirrorSwitch").join("logs").join("panic.log");
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut file| {
                use std::io::Write;
                writeln!(file, "{} {info}", chrono::Local::now().to_rfc3339())
            });
        default_hook(info);
    }));
}

/// Paint a small RGBA app icon at runtime so the binary has no asset pipeline.
fn app_icon() -> egui::IconData {
    const SIZE: usize = 64;
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    let accent = [45u8, 212, 191];
    let dark = [8u8, 22, 26];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let index = (y * SIZE + x) * 4;
            let inside = rounded_rect_contains(x as f32, y as f32, 2.0, 2.0, 60.0, 60.0, 14.0);
            if !inside {
                continue;
            }
            let mut color = accent;
            // Two arrows: upper points right, lower points left.
            let upper_line =
                (16.0..=46.0).contains(&(x as f32)) && (25.0..=29.0).contains(&(y as f32));
            let lower_line =
                (18.0..=48.0).contains(&(x as f32)) && (35.0..=39.0).contains(&(y as f32));
            let upper_head = triangle(x as f32, y as f32, (46.0, 19.0), (46.0, 35.0), (55.0, 27.0));
            let lower_head = triangle(x as f32, y as f32, (18.0, 31.0), (18.0, 47.0), (9.0, 37.0));
            if upper_line || lower_line || upper_head || lower_head {
                color = dark;
            }
            rgba[index] = color[0];
            rgba[index + 1] = color[1];
            rgba[index + 2] = color[2];
            rgba[index + 3] = 255;
        }
    }
    egui::IconData {
        rgba,
        width: SIZE as u32,
        height: SIZE as u32,
    }
}

fn rounded_rect_contains(
    x: f32,
    y: f32,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    radius: f32,
) -> bool {
    let cx = (left + width / 2.0).clamp(left + radius, left + width - radius);
    let cy = (top + height / 2.0).clamp(top + radius, top + height - radius);
    let dx = (x - cx).abs();
    let dy = (y - cy).abs();
    let corner_x = (left + radius - x).max(0.0) + (x - (left + width - radius)).max(0.0);
    let corner_y = (top + radius - y).max(0.0) + (y - (top + height - radius)).max(0.0);
    if x < left || x > left + width || y < top || y > top + height {
        return false;
    }
    if dx <= width / 2.0 - radius || dy <= height / 2.0 - radius {
        return true;
    }
    (corner_x * corner_x + corner_y * corner_y) <= radius * radius
}

fn triangle(px: f32, py: f32, a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let sign = |p1: (f32, f32), p2: (f32, f32), p3: (f32, f32)| {
        (p1.0 - p3.0) * (p2.1 - p3.1) - (p2.0 - p3.0) * (p1.1 - p3.1)
    };
    let d1 = sign((px, py), a, b);
    let d2 = sign((px, py), b, c);
    let d3 = sign((px, py), c, a);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}
