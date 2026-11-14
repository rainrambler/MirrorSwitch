# 架构说明

MirrorSwitch 是单二进制 egui 桌面应用。核心原则是：**先快照，后写入；只改托管内容；平台差异隔离。**

## 模块

| 文件 | 职责 |
| --- | --- |
| `src/main.rs` | 窗口、图标、启动错误提示、panic 日志 |
| `src/app.rs` | 五个页面、交互状态、异步测速结果消费 |
| `src/catalog.rs` | 内置镜像目录、官方端点、URL 归一化 |
| `src/cargo.rs` | `config.toml` 安全合并、当前 Cargo 配置探测 |
| `src/platform.rs` | Windows 注册表 / macOS Shell + launchctl 适配 |
| `src/shell.rs` | zsh/bash/fish/sh 托管块解析与写入 |
| `src/backup.rs` | 快照捕获、恢复、去重、清理 |
| `src/speedtest.rs` | 并发测速、URL 构造、评分、结果缓存 |
| `src/state.rs` | 当前 Cargo/Rustup 状态摘要与告警 |
| `src/theme.rs` | 深/浅色主题、CJK 字体检测 |
| `src/widgets.rs` | 自绘控件与图标 |
| `src/util.rs` | 原子写入、格式化、路径操作 |
| `src/settings.rs` | 应用设置持久化 |
| `src/logging.rs` | 文件 + 内存日志 |
| `src/screenshot.rs` | 离屏截图（文档/验收用） |

## 配置写入流程

```text
用户点击应用
  → 校验当前 Cargo TOML / Shell 配置可读
  → 创建切换前自动快照（失败则中止）
  → 写入 Cargo 配置（toml_edit 保留其他键）
  → 写入 Rustup 平台配置
  → 刷新状态、清理自动快照、显示结果
```

恢复流程类似，但会先创建 `pre_restore` 快照；Cargo/Shell 文件均恢复到**当前解析路径**，路径不一致时给出警告。

## 安全不变量

1. Cargo 配置解析失败时禁止切换，绝不覆盖非法 TOML。
2. 快照失败时禁止切换。
3. Cargo 只修改 `[source.crates-io].replace-with` 和 `mirrorswitch-*` 表。
4. Shell 只修改带标记的托管块；值拒绝换行，并使用安全引号。
5. Windows 不写 `HKLM`；不删除用户未托管的 Shell 行。
6. 恢复前再次快照，恢复操作可逆。
7. 原子写入使用同目录临时文件；遵循符号链接目标并保留权限。

## 备份格式

- `schema = 2`，JSON 明文。
- `cargo`：路径、是否存在、完整内容、解析摘要。
- `shell`（macOS/Linux，可选）：路径、是否存在、完整内容。
- `rustup`：用户值、实际生效值、摘要。
- `fingerprint`：用于自动快照去重，不是安全校验。

`schema = 1` 的旧快照仍可读取，`shell` 字段缺省为 `None`。

## 测速

- 每个目标先请求小元数据文件测延迟，再通过 HTTP Range 下载样本。
- Cargo 从索引 `config.json` 的 `dl` 字段构造真实 crate 下载地址。
- 官方源作为基线。
- 评分：80% 下载速度（20 MiB/s 封顶）+ 20% 延迟（1 s 线性衰减）。
- 测速在后台线程池运行，UI 通过 `mpsc` 接收进度，不阻塞渲染。

## 平台差异

- Windows：注册表 + 广播；静态 CRT；`winresource` 嵌入图标/版本。
- macOS：Shell 托管块 + `launchctl`；`.app` 打包由 `scripts/build-macos.sh` 完成；PingFang 字体。
- 其他 Unix：Shell 托管块；不调用 launchctl。
