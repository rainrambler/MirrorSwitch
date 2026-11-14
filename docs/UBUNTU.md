# Ubuntu 兼容设计方案

> 状态：**设计阶段，尚未实现**。本文给出从当前 Windows/macOS 版本扩展到 Ubuntu 的架构、改动点、打包方式和验收标准。

## 1. 目标与非目标

### 目标

- Ubuntu 22.04 / 24.04，x86_64 优先，Wayland 与 X11 都能启动。
- Cargo 镜像切换继续使用现有 `config.toml` 安全合并逻辑，无需重写。
- Rustup 镜像在终端与桌面应用中尽量都生效：
  - 终端：写入正确的 Bash/Zsh/Fish 配置。
  - 图形会话：通过 `environment.d` + systemd/dbus 同步，可选、可回滚。
- 备份/恢复覆盖 Cargo 配置、Shell 配置、Linux 桌面会话环境文件。
- 提供 `.deb` 安装包与便携 tar.gz；GitHub Actions 自动测试和发布。

### 非目标（首期）

- 不打包 Flatpak/Snap（沙箱会改变路径与 rustup 行为）。
- 不支持非 systemd 的 Ubuntu 衍生版自动会话同步；仅给出警告。
- 不自动修改 `/etc/environment` 或系统级配置。
- 不在首期提供 arm64 官方安装包（架构上可行，后续加 runner/交叉构建）。
- 不改变 Windows/macOS 现有行为与备份兼容性。

## 2. 现状差距

| 模块 | 当前状态 | Ubuntu 问题 | 方案 |
| --- | --- | --- | --- |
| 窗口/渲染 | `eframe 0.32.3`，仅启用 `glow`、`default_fonts` | 未启用 `x11`/`wayland`，Linux 无窗口后端 | 增加 `x11`、`wayland` feature |
| TLS | `reqwest` + `native-tls` | Ubuntu 用 OpenSSL，构建/运行依赖 `libssl` | 开发装 `libssl-dev`，`.deb` 声明 `libssl3 \| libssl3t64` |
| Cargo 切换 | 跨平台 | 无 | 直接复用 |
| Rustup 持久化 | Windows 注册表 / macOS Shell + launchctl | Linux 分支只写 Shell，无桌面会话同步；默认 Shell 与 Bash 文件不适用 | 修正 Shell 探测；新增 environment.d/systemd 同步 |
| 数据目录 | `dirs::data_dir` | Linux 为 `~/.local/share` | 直接复用，补文档 |
| 文件打开 | `xdg-open` | 已可用 | 保留 |
| 错误提示 | Windows MessageBox / macOS osascript | Linux 无 GUI 提示 | 增加 zenity/kdialog 后备 |
| 字体 | Noto CJK 候选 | Ubuntu 默认可能未装中文字体 | `.deb` Recommends `fonts-noto-cjk`，界面已有缺字提示 |
| CI | Windows + macOS | 无 Ubuntu | 增加 22.04/24.04 测试矩阵与打包任务 |

## 3. 平台架构

### 3.1 平台能力抽象

在现有 `platform.rs` 的 `cfg` 基础上，新增 Linux 分支，不引入新的 trait 层，避免过度设计：

```text
platform::set_rustup_env()
  Windows  -> HKCU\Environment + WM_SETTINGCHANGE
  macOS    -> shell block + launchctl setenv
  Linux    -> shell block + environment.d + systemd/dbus（可选）
```

新增：

```rust
pub fn desktop_session_path() -> Option<PathBuf>;      // Linux: ~/.config/environment.d/95-mirrorswitch.conf
pub fn sync_desktop_session(dist, update) -> Vec<String>;
pub fn desktop_session_note() -> Option<String>;
```

`DesktopSession` 是否启用由设置项控制：

- `linux_sync_desktop_session: bool`，默认 `true`。
- 检测到 `systemctl --user` 不可用或 `DBUS_SESSION_BUS_ADDRESS` 缺失时，自动降级为仅 Shell 并提示。

### 3.2 Shell 配置探测修正

当前 Linux 上有两个问题：

1. `$SHELL` 缺失时默认 `/bin/zsh`，Ubuntu 默认是 Bash。
2. Bash 固定使用 `~/.bash_profile`，但 Ubuntu 终端通常启动非登录交互 Shell，读取 `~/.bashrc`。

建议规则：

| 平台 | Shell | 默认文件 |
| --- | --- | --- |
| macOS | zsh | `~/.zshrc` |
| macOS | bash | `~/.bash_profile` |
| Linux | zsh | `~/.zshrc` |
| Linux | bash | `~/.bashrc` |
| Linux | fish | `~/.config/fish/config.fish` |
| Linux | sh/dash | `~/.profile` |

- `$SHELL` 缺失：macOS 默认 `/bin/zsh`，Linux 默认 `/bin/bash`。
- `MIRRORSWITCH_SHELL_PROFILE` 仍可覆盖。
- 若通过 `sudo` 运行，检测 `SUDO_USER`，提示不要以 root 修改用户配置。

### 3.3 桌面会话同步（Linux 的 launchctl 对应物）

三级机制，按可用性降级：

1. **`~/.config/environment.d/95-mirrorswitch.conf`**  
   登录时由 systemd 用户环境生成器读取，供后续桌面会话和用户服务使用。
   ```ini
   RUSTUP_DIST_SERVER=https://rsproxy.cn
   RUSTUP_UPDATE_ROOT=https://rsproxy.cn/rustup
   ```
   切回官方源时删除该文件。只由 MirrorSwitch 管理，不做块合并。

2. **当前会话立即生效**
   ```bash
   systemctl --user set-environment RUSTUP_DIST_SERVER=... RUSTUP_UPDATE_ROOT=...
   dbus-update-activation-environment --systemd RUSTUP_DIST_SERVER RUSTUP_UPDATE_ROOT
   ```
   - 影响新启动的用户服务/DBus 激活应用。
   - 已运行的应用仍需重启。
   - 命令缺失或失败只产生警告，不阻塞 Shell 配置写入。

3. **终端**：Shell 配置始终是终端环境的来源。

> 限制：不同 Display Manager、非 systemd 会话、从终端 `exec` 启动的桌面环境行为不同。无法保证所有 GUI 应用立即继承；文档需明确。

### 3.4 环境变量读取与状态显示

当前 `RustupEnvState` 只有一组值。Ubuntu 上终端与桌面可能不同步，建议：

```rust
pub struct RustupEnvState {
    pub dist: EnvVarState,        // 终端/Shell 优先
    pub update: EnvVarState,
    pub desktop: Option<EnvVarState>, // Linux 桌面会话值，用于展示与一致性告警
    pub warnings: Vec<String>,
}

pub enum EnvScope {
    User, Shell, Session, System, Process, Unset,
}
```

- `effective` 取值优先级：Shell → Session → Process。
- 若 Shell 与 Session 的值不一致，仪表盘提示“终端与桌面会话镜像不一致”。
- 设置页展示：
  - `Shell 配置` 路径与来源。
  - `桌面会话` 文件路径与状态（Linux）。
  - 当前 `RUSTUP_*` 值与作用域。

## 4. 备份与恢复

### 4.1 Schema 3

在现有 schema 2 基础上新增 Linux 会话文件，保留旧字段：

```rust
pub struct Snapshot {
    // ... schema = 3
    pub shell: Option<SnapshotShell>,          // 已有，macOS/Linux Shell 文件
    pub desktop_env: Option<SnapshotShell>,    // 新增，Linux environment.d 文件
}
```

- `#[serde(default)]` 保证 v1/v2 快照仍可读。
- 指纹纳入两个文件的内容与路径。
- 恢复顺序：
  1. 恢复 Cargo 配置。
  2. 恢复 Shell 文件。
  3. 恢复 `environment.d` 文件。
  4. 同步当前进程与 systemd/dbus 会话。
- 恢复前始终创建 `pre_restore` 快照。

### 4.2 设置项

- `linux_sync_desktop_session`（默认 true）
- 修改后立即保存；关闭时删除托管会话文件并保留 Shell 配置。
- 关闭动作也应先自动备份。

## 5. Ubuntu 构建与运行

### 5.1 Cargo feature

```toml
eframe = {
  version = "=0.32.3",
  default-features = false,
  features = ["default_fonts", "glow", "x11", "wayland"]
}
```

- 同时支持 X11 与 Wayland；Wayland 会话下自动选择，X11 后备。
- `glow` 使用 OpenGL，Ubuntu 安装 Mesa 即可。

### 5.2 构建依赖

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential pkg-config libssl-dev \
  libx11-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libwayland-dev libxcursor-dev libxrandr-dev \
  libxi-dev libxinerama-dev libgl1-mesa-dev libfontconfig1-dev
```

（最终列表以 Ubuntu CI 实际构建结果为准，避免加入未使用依赖。）

### 5.3 运行时依赖

- `libc6`
- `libssl3`（22.04）或 `libssl3t64`（24.04）
- `libxkbcommon0`、`libwayland-client0`、`libx11-6`、`libxcb1`
- `libgl1`、`libfontconfig1`
- 推荐：`fonts-noto-cjk`
- 可选：`zenity` 或 `kdialog`（启动错误弹窗后备）

## 6. 打包方案

### 6.1 首选 `.deb`

新增：

```text
packaging/linux/
  mirrorswitch.desktop
  io.mirrorswitch.app.metainfo.xml
  control.in
scripts/build-linux.sh
```

安装布局：

```text
/usr/bin/mirrorswitch
/usr/share/applications/io.mirrorswitch.app.desktop
/usr/share/icons/hicolor/256x256/apps/io.mirrorswitch.app.png
/usr/share/icons/hicolor/1024x1024/apps/io.mirrorswitch.app.png
/usr/share/metainfo/io.mirrorswitch.app.metainfo.xml
/usr/share/doc/mirrorswitch/{README.md,copyright,changelog.gz}
```

产物：

```text
dist/MirrorSwitch_1.0.0_amd64.deb
dist/MirrorSwitch-v1.0.0-linux-x86_64.tar.gz
dist/SHA256SUMS-linux.txt
```

- `dpkg-deb --build --root-owner-group`，无需额外 Rust 打包工具。
- `.desktop` 的 `StartupWMClass=MirrorSwitch`，保证任务栏图标归组。
- AppStream 元数据用于 GNOME Software / KDE Discover。

### 6.2 便携 tar.gz

目录包含二进制、README、LICENSE、desktop 文件和图标，供非 Debian 系发行版使用。

### 6.3 AppImage / Flatpak（后续）

- AppImage 需要处理 OpenGL、OpenSSL、Wayland/X11 动态库；首期不做。
- Flatpak 沙箱会隔离 `$HOME` 与注册表/环境变量，和本项目“修改用户真实配置”的定位冲突；不建议首期支持。

## 7. CI 与发布

### 7.1 CI

在 `.github/workflows/ci.yml` 增加：

```yaml
matrix:
  os: [windows-latest, macos-14, ubuntu-22.04, ubuntu-24.04]
```

Ubuntu job：

```yaml
- run: sudo apt-get update
- run: sudo apt-get install -y <构建依赖>
- run: cargo fmt --all -- --check
- run: cargo clippy --all-targets --locked -- -D warnings
- run: cargo test --all-targets --locked
- run: bash scripts/build-linux.sh
- uses: actions/upload-artifact@v4
```

可选：`xvfb-run -a ./target/release/mirrorswitch` 启动 3 秒，验证窗口初始化与截图模式。

### 7.2 Release

在 `release.yml` 增加 Linux job：

- `runs-on: ubuntu-24.04`
- 安装构建依赖
- `BUILD_DEB=1 bash scripts/build-linux.sh`
- 上传 `.deb`、`.tar.gz`、SHA256
- 与 Windows/macOS 产物一起发布到 GitHub Release

## 8. 测试计划

### 单元测试

- `shell::default_profile_for(home, kind, platform)`：
  - macOS + bash = `.bash_profile`
  - Linux + bash = `.bashrc`
  - Linux 默认 Bash；macOS 默认 zsh
- `environment.d` 写入/删除/解析测试。
- Schema 2 → 3 的备份兼容读取。
- 指纹包含桌面会话文件。
- Shell 托管块在 Bash/Zsh/Fish 下的幂等性（已有）。

### Ubuntu CI

- Ubuntu 22.04/24.04 `cargo test`。
- `cargo clippy -D warnings`。
- `.deb` 构建成功。
- `dpkg-deb --info` / `--contents` 检查布局。
- 可选：在干净容器安装 `.deb`，`xvfb-run` 启动 3 秒，`xdg-open` 不阻塞。

### 手工验收

1. 全新 Ubuntu 22.04/24.04 虚拟机，Bash 默认 Shell。
2. 安装 `.deb`，从应用菜单启动。
3. 切换 RsProxy Cargo + Rustup。
4. 新终端执行 `cargo -V`、`rustup show`，确认镜像环境变量。
5. 关闭/重开图形应用，确认 environment.d 生效。
6. 手动/自动备份，修改配置，恢复，确认 Cargo/Shell/environment.d 均回到快照。
7. 切回官方源，确认托管内容清理且其他 Shell 行不受影响。

## 9. 风险与缓解

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| Wayland/X11 依赖缺失 | 无法构建/启动 | 同时启用两套 feature；CI 安装 Mesa/XKB/Wayland 依赖 |
| `environment.d` 只在下一次登录生效 | GUI 应用短期不一致 | 同时调用 systemd/dbus；明确提示需重启会话 |
| Shell 文件选错（`.bashrc` vs `.bash_profile`） | 新终端不生效 | Linux 默认 `.bashrc`；允许 `MIRRORSWITCH_SHELL_PROFILE` 覆盖 |
| OpenSSL 运行库版本差异 | `.deb` 依赖失败 | 依赖写 `libssl3 \| libssl3t64`；后续评估 rustls |
| 用户已有未托管 RUSTUP_* 行 | 值被覆盖/官方源无法完全恢复 | 保留已有警告；托管块追加末尾；文档说明 |
| root/sudo 运行 | 写到错误 HOME | 检测 `SUDO_USER` 并警告；不自动提权 |
| 桌面环境非 systemd | 会话同步失败 | 仅警告并保留 Shell 配置；文档说明 |
| 包体积与中文字体 | 缺字/安装体积 | `fonts-noto-cjk` 作为 Recommends，而非硬依赖 |

## 10. 交付阶段

### P0：可构建、可运行（0.5–1 天）

- 增加 `x11`/`wayland` feature。
- Linux 构建依赖、Shell 探测修正、Linux 文案、错误弹窗。
- Ubuntu 加入 CI。
- **验收**：Ubuntu 上 `cargo run` 启动；Cargo 切换可用；Bash 新终端读到 Rustup 变量。

### P1：桌面会话与备份（1–2 天）

- `environment.d` 读写、systemd/dbus 同步、设置开关。
- 备份 schema 3、恢复与测试。
- **验收**：新终端 + 重启后的 GUI 应用均生效；恢复可回滚。

### P2：`.deb` 与发布（0.5–1 天）

- `packaging/linux`、`build-linux.sh`、AppStream、`.desktop`。
- Release workflow 上传 `.deb`/tar.gz。
- **验收**：干净 Ubuntu 安装 `.deb` 可启动。

### P3：可选增强

- arm64 构建、AppImage、Flatpak 评估、自动更新。

## 11. 需要确认的决策

1. **首期发布格式**：建议 `.deb` + tar.gz；AppImage 后置。
2. **桌面会话同步默认值**：建议默认开启，可在设置关闭。
3. **Ubuntu 版本范围**：建议 22.04/24.04，x86_64 优先。
4. **TLS 方案**：建议首期保留 `native-tls` + OpenSSL 依赖，避免改动 Windows/macOS 构建；后续再评估 rustls。

## 12. 验收标准（Definition of Done）

- Ubuntu 22.04/24.04 上 `cargo test`、`cargo clippy`、release 构建通过。
- `.deb` 可安装、可启动，应用菜单图标正常。
- Cargo 与 Rustup 切换在**新终端**生效。
- 桌面会话同步开启时，重启会话后的 GUI 应用继承镜像变量；关闭时不写 environment.d。
- 备份/恢复覆盖 Cargo、Shell、environment.d，旧快照可读。
- 不修改 `/etc/environment`、系统级配置、toolchain、credentials。
- README 增加 Ubuntu 安装、依赖、限制与卸载说明。
