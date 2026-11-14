# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 与 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### Planned

- Ubuntu 22.04/24.04 兼容、`.deb` 打包与桌面会话环境同步（设计见 `docs/UBUNTU.md`）
- 自定义镜像编辑
- macOS 公证发布流程
- 更细粒度的测速历史对比

## [1.0.0] - 2026-11-14

### Added

- Rust + egui 桌面界面：概览、镜像源、测速对比、备份恢复、设置。
- Cargo 稀疏索引镜像切换，使用 `toml_edit` 保留其他配置与注释。
- Rustup 镜像切换：
  - Windows：`HKCU\Environment` + `WM_SETTINGCHANGE`。
  - macOS：托管 Shell 配置块（zsh/bash/fish/sh）+ `launchctl setenv`。
- 镜像测速：延迟、真实 crate/工具链清单下载、综合评分、官方源基线、失败原因。
- 备份与恢复：自动/手动快照、去重、滚动清理、恢复前再备份、完整 Cargo 与 Shell 文件快照。
- 深色/浅色主题，统一采用黑、白、灰、红四色体系；中文字体自动检测、操作日志、单文件 Windows 构建与 macOS `.app` 打包。
- 内置 RsProxy、USTC、TUNA、SJTU、阿里云、华为云镜像；USTC 因 Cargo 下载端点异常仅启用 Rustup。
- GitHub Actions：Windows/macOS CI、Rust 1.88 MSRV、tag 发布工作流。

### Security

- 非法 TOML 不覆盖；切换前备份失败则中止。
- 不修改系统级环境变量、toolchain、SSH 密钥或 `credentials.toml`。
- Shell 托管块转义并拒绝换行注入。
- 备份恢复针对当前解析路径，避免盲目信任旧路径。
