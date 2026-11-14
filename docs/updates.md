# 在线更新

自 0.3.0 提供。0.2.0 及更早版本需要先手动覆盖安装 0.3.0 一次，之后才可通过应用内更新升级。已有旧版发布资产继续保留。

## 使用方式

客户端“关于”页提供“检查更新”。仅在用户点击时联网访问 GitHub 最新正式版，不在启动时后台检查，不获取预发布版本或降级版本。检查失败显示可重试的错误，不把网络故障当作已是最新版本。

- Windows NSIS、Linux AppImage／DEB：发现支持当前安装格式的更新后，先下载，显示进度；签名与版本校验成功后提供“安装并重启”。离开“关于”页不取消下载。安装前检查本机所有身份的实际上传及已接受／传输中的任务，存在这些工作时暂不安装；已取消上传或空闲草稿不阻止更新。安装失败可重试，安装完成但重启失败时可单独重启，避免重复安装。DEB 安装由系统包管理器处理，可能需要系统授权。
- Android：发现新版本后打开对应 APK 下载地址，由系统浏览器下载、系统安装器确认覆盖安装，继续使用既有 Android APK 发布签名。不绕过系统安装授权。
- 未打包的 Linux 开发程序或缺少对应更新清单的版本：提供正式发布页／安装包下载入口。
- 普通浏览器：保留“下载客户端”，不能发起宿主应用的安装或重启。网页界面随宿主应用升级。

## 客户端实现

`crates/core/src/updates.rs` 请求固定仓库的 GitHub Releases API，使用语义化版本比较；只接受当前仓库、对应 tag 的标准安装包 URL。该功能通过原生 Tauri IPC 调用，不向局域网 HTTP API 开放更新操作。API 请求设连接超时、总超时，并处理服务限流。

桌面采用 Tauri updater 2.14.1 及 process 插件。公钥和 HTTPS 更新端点固定在 tauri.conf.json；启用 requireSignedVersion，禁止降级。更新器在下载与安装时验证包签名，并检查签名的版本与清单版本一致。安装前的 prepare_update 检查已有任务；检查后到退出之间仍可能有新网络请求到达，不宣称更新过程是跨设备事务或提供离线传输保证。未完成任务沿用已有中断恢复规则，升级不删除数据库和历史。

端点为 `https://github.com/whoarei/linchuan/releases/latest/download/latest.json`。清单按安装格式区分 windows-x86_64-nsis、linux-x86_64-appimage、linux-x86_64-deb，不将 DEB 用户切换到 AppImage。更新内容以纯文本展示，未在应用内执行远端 Markdown／HTML。

## 签名与发布

更新签名密钥与 Android APK 签名密钥、本机传输身份密钥分开。仓库只包含更新公钥；私钥和密码保存在仓库外的私有目录，并已有独立备份。更换更新公钥会影响已安装客户端的信任，因此正常发布沿用同一把更新签名密钥。

正式版共发布九个文件：四个安装包、三个桌面 `.sig` 文件、`latest.json`、`SHA256SUMS`。Tauri v2 直接使用已构建的 NSIS、AppImage、DEB 作为更新包；构建验证无需持有发布私钥。签名使用 `--app-version` 绑定版本，发布前验证完整字节、Minisign 全局签名、版本、公钥、目标类型和下载地址。

离线签名路径适用于当前账号无法配置 GitHub Secrets 的情形。先将同一源代码构建的四个安装包放入 release 目录，再运行：

```sh
python3 -m pip install cryptography
python3 scripts/updates.py sign release --tag vX.Y.Z \
  --key-path /private/path/linchuan-updater.key \
  --password-file /private/path/password.txt
python3 scripts/checksums.py release
```

tag 必须与当前 package.json 版本一致。签名结果是分离文件，不修改安装包。把三个 `.sig`、`latest.json`、新的 `SHA256SUMS` 与原有签名 APK、source.json、release-notes.md 一起放入发布用的暂存提交。暂存提交不得包含私钥或密码。`publish-prebuilt.yml` 验证构建来源、APK 签名、实际下载到的桌面安装包签名、清单和校验和后，上传完整资产，再把草稿设为最新正式版。

另一条 `release.yml` 路径使用 GitHub Secrets 中的 `TAURI_SIGNING_PRIVATE_KEY`（私钥文件内容）、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 和已有 Android 签名 Secrets，在发布任务中签名；没有这些密钥时会在发布前失败。私钥内容不得放入普通仓库变量、工作流参数、日志或公开构建资产。

## 验证

本地通过前端构建、Rust fmt／clippy、24 项 Rust 测试、15 项 Playwright 流程、6 项发布签名测试，以及 Android arm64 编译检查。测试包含版本的数值比较、预发布／降级过滤、安装包 URL 回退、真实接收任务和实际上传时的安装阻止、取消上传后解除阻止、更新 UI 状态及重启重试、跨页面下载、Android 下载路径、网络失败、错误签名不允许安装，以及离线发布签名、篡改、重放版本、清单地址、目标完整性和发布目录私密文件保护。

发布时对本版实际桌面包使用真实 Tauri signer 签名和验证，公开清单随完整安装包一起上传，不改动旧版发布。Windows／Linux 系统安装和覆盖升级仍需实机验证；浏览器模拟和离线签名验证不能替代真实安装流程。详见[0.3 验证范围](validation-0.3.md)。
