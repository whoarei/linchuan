# 0.5.0 验证范围

本版将局域网连接升级为 HTTPS／WSS，保持随机端口、原生设备身份、历史数据及 SQLite schema 2。

## 自动化检查

- 41 项 Rust 核心测试：既有会话、群聊、文件与保存队列回归，以及正确／错误 CA、地址与有效期校验、设备身份绑定、冒充设备拒绝、CA 跨重启持久化、地址变化及长期运行续签、WebSocket Origin 与明文连接拒绝。
- 27 项 Playwright 流程：既有桌面／移动会话、接收目录、开机启动与托盘设置、更新和窗口模拟；新增 Chromium 拒绝未信任 CA，以及独立 Firefox 配置中真实信任 CA 后的 HTTPS 配对、安全上下文、WSS 通知、文件与证书下载、关页重开。
- 23 项 Python 发布策略测试，TypeScript／Vite、Rust fmt／Clippy、工作流 actionlint，以及 Android arm64 Tauri 编译检查。
- HTTPS 功能提交的 [GitHub Checks](https://github.com/whoarei/linchuan/actions/runs/43660796700)已通过；发布仍以 v0.5.0 标签对应提交的重新检查为准。

浏览器信任测试只修改可丢弃的 Firefox 测试配置，不更改宿主系统或用户浏览器的信任，不使用忽略证书错误来宣称安全上下文可用。原生启动探测已用核心 CLI 和 Python／OpenSSL 验证，含配置 HTTP／HTTPS 代理的环境。

## 发布流水线

同一标签提交通过 Checks 后构建 Windows NSIS、Linux DEB／AppImage 和 Android arm64 APK。Linux 以 Xvfb 分别验证正常启动和 --autostart 静默启动：从回环接口取得公共 CA，再正常验证局域网 HTTPS 服务。

签名统一来自既有 GitHub Secrets。Android 应用 ID、版本名 0.5.0、版本码 500999、证书及 ZIP／ELF 16 KiB 对齐必须通过；桌面更新签名使用内置公钥验证完整文件及精确版本。

上传四个安装包、三个桌面签名、latest.json 和 SHA256SUMS，核对全部九个资产的名称、长度和 SHA-256 后公开，正式版设为 Latest。发布后回读公共下载与更新端点，验证内容、签名、APK 证书和版本。

## 实机边界

真实 Windows／Android 证书导入、系统安装升级、跨设备 Wi-Fi、Linux 桌面托盘及登录启动仍需实机验收。自动化原生服务启动、受信任 CA 的浏览器流程和接口模拟不替代这些验收。

本版不实现会议、远程控制、离线补发或 Android 开机启动／后台常驻。已知旧 HTTP 客户端须同步升级，浏览器需信任 CA 并重新配对；原生身份和历史沿用。
