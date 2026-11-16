# 0.5.2 验证范围

本版调整文件接收卡片：确认入口移到卡片上方，增加接收进度与取消操作，多文件共用确认入口，保存后按文件提供操作。传输协议、数据库 schema 2 和文件保存流程保持兼容。

## 已完成的交互检查

- TypeScript／Vite 生产构建通过。
- 12 项文件共享与接收目录 Playwright 流程通过，覆盖浏览器文件完整下载、设备直传、群聊逐人接收、保存失败重试、文件和目录打开，以及清理缓存后打开已保存文件。
- 桌面和手机分别核对待接收、传输中、已保存、已拒绝和多文件五种卡片状态，共十种布局。核对单组确认按钮、接收时隐藏文件操作、进度与取消、完成后三个操作、拒绝后的状态，以及无横向溢出。
- 上述布局检查使用真实前端及接口数据模拟，不代表真实桌面或 Android 文件管理器验收。

## 发布前完整检查

- 36 项 Rust 核心与集成测试、27 项 Playwright 流程、23 项 Python 签名与发布策略测试全部通过。
- TypeScript／Vite 生产构建、Rust fmt 和 Clippy（拒绝警告）通过。为适配当前 Clippy，对四处既有身份或会话校验的布尔表达式作等价简化；授权判断保持一致，完整核心测试通过。
- Python 发布测试在 WSL Ubuntu 中执行，包含真实 Debian 包版本顺序检查；Rust 和浏览器检查在本机 Windows 执行。
- 标签与前端、Tauri、两个 Rust crate 和两个锁文件的 0.5.2 版本一致，Android 发布版本码为 502999。

## 发布检查要求

v0.5.2 的同一标签提交需通过完整 Checks，再构建 Windows NSIS、Linux DEB／AppImage 和 Android arm64 APK。Linux 分别检查正常启动和 --autostart；Android 核对应用 ID、版本名称 0.5.2、版本码 502999、既有签名证书和 ZIP／ELF 16 KiB 对齐。

公开前核对四个安装包、三个桌面签名、latest.json 和 SHA256SUMS，共九个资产。公开后重新下载核对摘要、三个桌面更新签名及精确版本，确认 Latest 和在线更新指向 0.5.2。

## 实际发布记录

- [v0.5.2 正式版](https://github.com/whoarei/linchuan/releases/tag/v0.5.2)于 2026-11-16 公开并设为 Latest，发布源码为 `8afb277b67a731ef748342bf88d467fc3e5311e6`，远程标签与发布运行指向同一提交。
- [正式 Release 流水线](https://github.com/whoarei/linchuan/actions/runs/43807334826)全部成功：同一提交的完整 Checks、Windows NSIS、Linux DEB／AppImage、Android arm64、统一签名与发布。Linux 正常启动与 --autostart 检查、Android 构建产物 16 KiB 对齐检查均通过。[main 分支 Checks](https://github.com/whoarei/linchuan/actions/runs/43807334800)也已成功。
- 发布后重新下载全部九个资产，核对 SHA256SUMS、远端文件大小和 SHA-256 摘要、三个桌面更新签名及精确版本。公开 Latest API 和 `releases/latest/download/latest.json` 均返回 0.5.2；Debian 包内部版本为 0.5.2、架构为 amd64。
- Android APK 的应用 ID、版本名称 0.5.2、版本码 502999、APK 签名、ZIP 16 KiB 对齐、arm64 ABI 及所有原生库的 ELF 16 KiB 页对齐均通过本地复核。证书 SHA-256 指纹为 `28c461e4f2f8612f9d7717d727e772177474798fd7c48b03ca3ffbca8c81bf0a`，与 0.4.0 起的既有发布证书一致。
- 发布复用现有 GitHub Actions 与 Secrets，未覆盖或移动既有公开版本和标签。

## 验收边界

本轮未在真实 Windows／Android 系统上执行安装升级，未重新进行跨设备 Wi-Fi、托盘或登录启动验收。普通 HTTP 不提供传输加密或浏览器音视频安全上下文。
