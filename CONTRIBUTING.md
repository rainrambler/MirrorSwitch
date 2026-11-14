# 贡献指南

感谢你愿意改进 MirrorSwitch。本文档说明开发环境、验证命令和提交规范。

## 开发环境

- Rust 1.88+（`rust-toolchain.toml` 会自动选择 stable，并安装 rustfmt/clippy）
- Windows 10/11：MSVC 工具链
- macOS 11+：Xcode Command Line Tools（`xcode-select --install`）

```bash
git clone <repo-url>
cd MirrorSwitch
cargo run
```

首次构建会下载依赖。Windows 构建使用项目内 `.cargo/config.toml` 静态链接 CRT。

## 提交前检查

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
```

macOS 平台代码还应交叉/本机检查：

```bash
cargo check --target aarch64-apple-darwin
cargo check --target x86_64-apple-darwin
```

真实网络测速测试默认 ignored，避免 CI 依赖外网：

```bash
cargo test live_probe -- --ignored --nocapture
```

## 修改原则

- **安全优先**：切换和恢复配置前必须创建快照；快照失败时中止写入。
- **不做破坏性写入**：非法 TOML 不得覆盖；Cargo 配置只改托管键；Shell 配置只改托管块。
- **跨平台隔离**：平台相关代码使用 `#[cfg]`，不要破坏另一个平台的构建。
- **备份格式兼容**：修改快照结构时使用 `#[serde(default)]`，并补充旧版本读取测试。
- **不收集遥测**；新增网络请求必须说明目标、用途和超时行为。
- **提交中不要包含**：`target/`、`dist/`、真实备份、Shell 配置、token、私钥、含用户名的绝对路径截图。

## 代码风格

- 使用 `cargo fmt` 默认风格。
- 遵循 Clippy `-D warnings`。
- 错误信息面向中文用户，包含可操作的建议；内部错误用 `anyhow::Context` 保留链。
- 新增 UI 文案优先中文，保持简洁。
- 新增镜像时更新 `src/catalog.rs` 与 README 镜像表，并尽量验证端点。

## Pull Request

1. 从 `main` 创建分支。
2. 保持改动聚焦；大改动请先开 Issue 讨论。
3. 填写 PR 模板中的测试与安全清单。
4. 界面变更请附截图；macOS 相关变更请注明验证环境。
5. 维护者可能会要求补充测试或拆分提交。

## 行为准则

参与本项目即表示同意遵守 [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)。

## 安全问题

请勿在公开 Issue 中披露漏洞，按 [SECURITY.md](SECURITY.md) 私下报告。
