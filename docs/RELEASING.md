# 发布流程

## 1. 更新版本

版本号唯一来源是 `Cargo.toml` 的 `package.version`。构建脚本会自动读取它：

- `scripts/build.ps1` → Windows ZIP 文件名
- `scripts/build-macos.sh` → macOS ZIP / Info.plist 版本
- `build.rs` → Windows exe 文件版本
- UI/关于页 → `CARGO_PKG_VERSION`

发布前同步更新：

- `CHANGELOG.md`（新版本条目）
- `README.md` 中出现的示例版本号
- `packaging/macos/Info.plist` 中的静态版本（脚本运行时会覆盖，但建议保持同步）

## 2. 本地验证

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
```

平台相关：

```bash
# Windows
pwsh ./scripts/build.ps1

# macOS
bash scripts/build-macos.sh
# 或发布通用二进制
UNIVERSAL=1 bash scripts/build-macos.sh
```

检查产物：

- Windows ZIP 内 exe 版本信息、图标、SHA256。
- macOS `.app` 可启动，`CFBundleShortVersionString` 正确，SHA256。
- 在干净环境中测试一次切换与恢复。

## 3. 提交与打 tag

```bash
git add .
git commit -m "release: vX.Y.Z"
git tag vX.Y.Z
git push origin main --tags
```

推送 `v*` tag 后，`.github/workflows/release.yml` 会：

1. `windows-latest` 构建 `MirrorSwitch-vX.Y.Z-win64.zip`；
2. `macos-14` 构建 Universal macOS ZIP；
3. 上传两个 ZIP 与 SHA256 文件到 GitHub Release。

## 4. 发布后检查

- Release 是否包含两个 ZIP 和 SHA256；
- macOS 包是否可解压并启动；
- Windows 包是否通过 SmartScreen 提示（未签名属预期）；
- 用 README 中的命令验证校验和；
- 更新 GitHub Release 说明（自动生成后可手动补充重点）。

## 签名与公证

当前 macOS 脚本使用 ad-hoc 签名。正式分发建议：

- 使用 Developer ID Application 证书替换 `codesign --sign -`；
- 使用 `xcrun notarytool` 公证；
- 使用 `xcrun stapler` 装订公证票据。

Windows 如需消除 SmartScreen，需要代码签名证书，不在本仓库流程内。
