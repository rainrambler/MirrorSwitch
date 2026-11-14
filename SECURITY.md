# 安全政策

## 支持范围

| 版本 | 安全更新 |
| --- | --- |
| 1.x | 支持 |
| < 1.0 | 不支持 |

## 私下报告漏洞

请**不要**通过公开 Issue、Discussion 或 PR 报告安全问题。优先使用 GitHub 仓库的 **Security → Report a vulnerability**（Security Advisory）私下提交；若仓库尚未开启私密漏洞报告，请通过维护者 GitHub 资料中的联系方式私下联系。报告请包含：

- 受影响版本与平台；
- 复现步骤或最小 PoC；
- 影响范围（配置损坏、命令执行、信息泄露等）；
- 建议的修复方向（可选）。

维护者会尽快确认并评估；在修复发布前，请避免公开细节。

## 重点关注的威胁

- Cargo 配置合并导致注释、私有源或其他设置损坏；
- 备份/恢复覆盖错误文件，或恢复路径逃逸；
- Shell 托管块注入、转义错误或命令执行；
- Rustup 环境变量更新到错误的注册表/配置文件；
- 备份 JSON 泄露 registry 凭据、token 或本机敏感路径；
- 构建脚本、依赖或 GitHub Actions 供应链问题。

## 已知设计边界

- 备份是**明文 JSON**，会包含 Cargo 配置和 macOS/Linux Shell 配置文件内容。
- Windows 只修改 `HKCU\Environment`，不会修改 `HKLM`。
- macOS `launchctl` 仅影响当前图形会话，不跨重启持久化。
- 工具不管理 `rustup/settings.toml`、`credentials.toml`、SSH 密钥或已安装工具链。
- 镜像地址由第三方提供，工具不保证镜像内容的安全性或可用性。

## 依赖与发布

- 发布包由 GitHub Actions 在 Windows/macOS 上构建。
- Windows 使用静态 CRT；macOS 使用 ad-hoc 签名，正式分发建议使用 Developer ID 签名并公证。
- 请核对 Release 中提供的 SHA256 校验文件。
