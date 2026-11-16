# 开发与发布

## 环境

Node.js 22、Rust stable（首版使用 Rust 1.100，文件锁 API 要求 1.89+）。桌面环境按 Tauri 2 安装系统依赖。

Ubuntu 22.04：

```bash
sudo apt-get install build-essential libwebkit2gtk-4.1-dev libssl-dev librsvg2-dev patchelf libayatana-appindicator3-dev
```

Android 需要 JDK 21、Android SDK Platform 36、Build Tools 35、NDK `27.2.12479018`，配置 `ANDROID_HOME`、`NDK_HOME`，安装 Rust target `aarch64-linux-android`。原生工程已提交，无需重新执行 `tauri android init`，以免覆盖网络权限和组播锁。

## 运行

```bash
npm ci
npm run tauri dev
```

单独运行协议核心与网页（开发辅助工具，不是额外部署节点）：

```bash
npm run build
cargo run -p linchuan-core -- /tmp/linchuan-dev '开发设备'
```

数据目录中的 `ready.json` 含开发实例的配对码和本机 owner 凭证，仅供本机测试，不要分享或提交。正式 Tauri 应用不会生成此文件，配对码从应用的连接面板查看。

## 验证

```bash
npm run build
cargo fmt --all -- --check
cargo clippy -p linchuan-core --all-targets -- -D warnings
cargo test -p linchuan-core --locked
cargo build -p linchuan-core --locked
npx playwright install chromium
npm run test:e2e
python3 -m pip install cryptography
python3 scripts/test_updates.py
```

`rust-embed` 会嵌入 `dist`；先构建前端再构建 Rust。首次 Linux 浏览器测试可使用 `npx playwright install --with-deps chromium` 安装系统依赖。HTTP 测试直接在局域网地址验证无证书配对、WS 和文件完整下载。ready.json 的 address 是本机 HTTP 地址；分享给局域网设备时使用信息接口的 addresses。

全量核心测试包含原文件直传和兼容上传／下载两条实际超过 2 GiB 的校验，需要相应磁盘读写时间；建议测试磁盘至少保留 7 GiB 可用空间。新浏览器模型、原生文件选择 IPC 和浏览器下载状态的回归见 `tests/direct-model.spec.ts`，直接文件流和故障恢复见 `crates/core/tests/direct_transfers.rs`。

## 打包

```bash
npm run tauri build -- --bundles deb,appimage  # Linux
npm run tauri build -- --bundles nsis          # Windows
npm run tauri android build -- --apk --target aarch64 --ci
```

桌面产物位于根目录 `target/release/bundle/`；Android 产物位于 `src-tauri/gen/android/app/build/outputs/apk/`。Android CI 构建的是未签名 release APK，不能把它直接当作正式发布产物。

## Android 签名

历史发布使用别名为 `linchuan` 的密钥。发布工作流支持接入其他 Android keystore、可配置别名和独立的 key 密码。已接受后续版本可卸载重装，Android 换密钥不再作为本轮发布的阻塞条件；桌面在线更新仍使用既有更新公钥对应的独立密钥。

在本机 SDK Build Tools 中执行：

```bash
zipalign -P 16 -f 4 input-unsigned.apk aligned.apk
apksigner sign --v4-signing-enabled false --ks /private/linchuan-release.keystore --ks-key-alias linchuan --ks-pass env:PASSWORD --key-pass env:PASSWORD --out Linchuan_0.3.0_android_arm64.apk aligned.apk
apksigner verify --verbose --print-certs Linchuan_0.3.0_android_arm64.apk
```

不要将密码放进命令行字面量或仓库。`PASSWORD` 由维护者从私有密码管理工具或文件注入。

在仓库的 [Actions Secrets 设置](https://github.com/whoarei/linchuan/settings/secrets/actions) 点击 **New repository secret**，配置：

| Secret | 内容 | 是否必填 |
| --- | --- | --- |
| `ANDROID_KEYSTORE_BASE64` | 你的 `.jks`／`.keystore` 文件的 Base64 文本 | 必填 |
| `ANDROID_KEYSTORE_PASSWORD` | keystore 密码 | 必填 |
| `ANDROID_KEY_ALIAS` | keystore 内要使用的密钥别名 | 建议填写；省略时使用 `linchuan` |
| `ANDROID_KEY_PASSWORD` | 该别名对应的 key 密码 | 与 keystore 密码不同才需填写；省略时复用 keystore 密码 |

Base64 是密钥文件的编码，输出文件也属于私密文件，应在仓库外操作。Linux／macOS 示例：

```bash
base64 < /private/release.keystore | tr -d '\n' > /private/keystore.base64.txt
```

Windows PowerShell 示例：

```powershell
[IO.File]::WriteAllText('C:\private\keystore.base64.txt', [Convert]::ToBase64String([IO.File]::ReadAllBytes('C:\private\release.keystore')))
```

将输出文本填入 `ANDROID_KEYSTORE_BASE64`。别名可在本机执行 `keytool -list -keystore /private/release.keystore` 查看，按提示输入密码；名称不是文件名，也不是证书显示的发布者名称。

完整的三平台发布工作流还需要 `TAURI_SIGNING_PRIVATE_KEY` 和对应的 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`，用于桌面更新包签名，配置方法见[在线更新](updates.md)。它们与 Android keystore 分开；仅配置 Android Secrets 还不能执行完整的三平台发布。

## CI 与发布

当前只保留两个工作流，完整规则见[CI 与 RC 发布](ci-release.md)：

- `Checks`：main 提交、PR 自动执行；纯文档修改跳过。包含前端、Rust、浏览器与签名／发布策略测试，不构建三平台安装包。
- `Release`：推送 `vX.Y.Z` 或 `vX.Y.Z-rc.N` 标签触发，也可手动填写已有标签重试。对标签对应的同一提交先执行 Checks，再构建 Android、Linux、Windows，使用 GitHub Secrets 签名、验签并自动发布。

无需填写旧构建编号，不再维护预签名暂存分支或独立签名验证工作流。签名验证已经并入发布步骤；任何检查、构建或验签失败都不会公开该 Release。发布中断可以重试未公开的草稿，已经发布的版本拒绝覆盖。

## 发布步骤

1. 同步 `package.json`、两个 Rust crate、`tauri.conf.json` 和两个锁文件的版本。RC 统一使用 `X.Y.Z-rc.N`，N 从 1 开始。
2. 添加 `docs/releases/X.Y.Z.md` 或 `docs/releases/X.Y.Z-rc.N.md`，提交并推送源码。
3. 为该提交创建版本标签并推送，例如 `v0.4.0-rc.1`。标签会自动触发检查、三平台构建、签名和发布。
4. 正式版设为 Latest；RC 标记为 Pre-release 且不设 Latest，不影响正式用户的在线更新。

运行 Release 时只需选择标签；GitHub Actions 会完成四个安装包、三个桌面 `.sig`、`latest.json` 和 `SHA256SUMS` 的上传及回读校验。所有文件完整后自动公开，无需另一次人工批准。

Android 的 CI 版本码和 Debian 的 RC 排序由发布脚本自动处理，规则与边界见[CI 与 RC 发布](ci-release.md)。无需手工修改生成的 `tauri.properties`。

## 已有签名验证记录

维护者配置的 Secrets 已通过[三平台实际签名验证](https://github.com/whoarei/linchuan/actions/runs/43608444945)：Android 证书名称为 `CN=Pindou`，APK 签名及 16 KiB 对齐通过；Windows NSIS、Linux AppImage／DEB 的更新签名、版本、清单和客户端内置公钥匹配。该记录来自工作流简化前的验证任务，未发布版本。
