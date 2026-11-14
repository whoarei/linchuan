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

密钥别名固定为 `linchuan`，私钥和密码只保存于维护者的私有备份中。首版维护凭证不具备 GitHub Secrets 管理权限，因此先在工作区签名，再发布。后续升级必须沿用相同密钥。

在本机 SDK Build Tools 中执行：

```bash
zipalign -P 16 -f 4 input-unsigned.apk aligned.apk
apksigner sign --ks /private/linchuan-release.keystore --ks-key-alias linchuan --ks-pass env:PASSWORD --key-pass env:PASSWORD --out Linchuan_0.1.0_android_arm64.apk aligned.apk
apksigner verify --verbose --print-certs Linchuan_0.1.0_android_arm64.apk
```

不要将密码放进命令行字面量或仓库。`PASSWORD` 由维护者从私有密码管理工具或文件注入。

具备权限后设置两个 GitHub Actions secrets：

- `ANDROID_KEYSTORE_BASE64`：同一发布 keystore 的 Base64 内容。
- `ANDROID_KEYSTORE_PASSWORD`：keystore 与 key 密码。

`Publish verified release` 工作流要求传入成功构建的 run ID 与已有版本 tag，校验 run 的提交 SHA 等于 tag，再收集产物、签名和发布。没有 Secrets 时使用本地签名流程，不重建临时密钥。

## 发布步骤

1. 更新各处版本（`package.json`、两个 Rust crate、`tauri.conf.json`）及锁文件、发布说明。
2. 等待同一提交的 `Build and verify` 所有 jobs 成功，检查测试报告与原生产物。
3. 将版本 tag 指向该提交；下载对应 run 的产物，核对来源和文件数量。
4. 签名并验证 APK；运行 `scripts/collect-release.py` 和 `scripts/checksums.py` 汇总安装包。
5. 发布 GitHub Release，附安装包、校验和及对应版本说明。
6. 再读取 Release，确认非 draft、版本 tag 正确、安装包齐全。真实设备测试范围应如实记录。

## 已签名产物发布入口

如果工作区凭证在 `uploads.github.com` 返回 401，可使用 `Publish pre-signed release`，让仓库 Actions 的发布权限执行上传。应用源码仍取自版本 tag，并校验其对应构建已全部成功。

在临时分支放入已经签名的 APK、全部安装包的 `SHA256SUMS`、`release-notes.md` 和 `source.json`（tag、source_sha、build_run、apk_name、certificate_sha256），传入该分支的不可变提交 SHA。工作流会核对源码、签名证书、ZIP 对齐和所有安装包摘要，再上传并正式发布。临时分支只包含将公开的安装产物，私钥继续由维护者私下保管。发布完成后可删除该分支。
