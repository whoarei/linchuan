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

`rust-embed` 会嵌入 `dist`；先构建前端再构建 Rust。首次 Linux 浏览器测试可使用 `npx playwright install --with-deps chromium` 安装系统依赖。

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

`Publish verified release` 工作流要求传入成功构建的 run ID 与已有版本 tag，先检查所需 Secrets，再校验 run 的提交 SHA 等于 tag。随后分别执行 Android 签名、APK 证书与对齐验证、桌面更新签名和正式发布。临时 keystore 和中间 APK 在结束或失败时移除。没有 Secrets 时仍可使用预签名流程。

此配置已通过 actionlint、YAML 和 Bash 语法检查，并在本地用临时测试 keystore 执行工作流中的实际签名命令：覆盖默认别名、任意别名、独立 key 密码、密码回退、多 APK 拒绝和临时文件清理。测试仅在临时目录重签公开 APK，原正式版文件未修改。

Secrets 填好后，可运行 `Validate signing configuration`，传入已成功的 `Build and verify` run ID。该任务只用 Android 产物测试签名、别名、密码及对齐，输出公开证书信息和是否配置桌面更新密钥的状态；不上传测试 APK、不发布 Release，也不导出密钥。任务只有 contents/actions 读取权限，结束或失败时删除临时密钥及测试 APK。

维护者添加的 Android Secrets 已通过[实际 Actions 验证](https://github.com/whoarei/linchuan/actions/runs/43606279174)：以成功构建 43604913154 的 APK 验证密钥、别名、密码、签名和 16 KiB ZIP 对齐，临时文件清理成功。证书名称为 `CN=Pindou`。该任务未发布版本，也未验证桌面更新私钥与内置公钥是否匹配。

## 发布步骤

1. 更新各处版本（`package.json`、两个 Rust crate、`tauri.conf.json`）及锁文件、发布说明。
2. 等待同一提交的 `Build and verify` 所有 jobs 成功，检查测试报告与原生产物。
3. 将版本 tag 指向该提交；下载对应 run 的产物，核对来源和文件数量。
4. 签名并验证 APK；运行 `scripts/collect-release.py` 汇总桌面包，再用 `scripts/updates.py sign` 生成桌面分离签名和 latest.json，最后运行 `scripts/checksums.py`。
5. 发布 GitHub Release，附四个安装包、三个桌面 `.sig`、latest.json、校验和及对应版本说明；完整上传后再将草稿发布为最新正式版。
6. 再读取 Release，确认非 draft、版本 tag 正确、安装包齐全。真实设备测试范围应如实记录。

## 已签名产物发布入口

如果工作区凭证在 `uploads.github.com` 返回 401，可使用 `Publish pre-signed release`，让仓库 Actions 的发布权限执行上传。应用源码仍取自版本 tag，并校验其对应构建已全部成功。

在临时分支放入已经签名的 APK、三个桌面 `.sig`、latest.json、覆盖全部九个文件的 `SHA256SUMS`、`release-notes.md` 和 `source.json`（tag、source_sha、build_run、apk_name、certificate_sha256），传入该分支的不可变提交 SHA。工作流会核对源码、APK 证书与 ZIP 对齐、桌面包签名、更新清单和所有文件摘要，再上传并正式发布。临时分支只包含将公开的产物，私钥继续由维护者私下保管。签名命令和密钥配置见[在线更新](updates.md)。
