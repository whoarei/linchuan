# 0.5.1 验证范围

本版恢复 HTTP／WS 与 protocol=1，移除局域网 CA、TLS 服务、证书下载和信任操作，保留随机端口及 SQLite schema 2。

## 自动化检查

- 36 项 Rust 核心测试：既有会话、群聊、文件接收、保存队列及恢复流程；新增无证书 HTTP 接入、随机端口、协议 1 信息契约、证书接口移除、旧 CA 设置无效时仍能启动、身份／名称／浏览器会话与历史保留、WS Origin 和手动地址校验。
- 27 项 Playwright 流程：既有桌面／手机界面、群聊、接收目录、开机启动／托盘设置、窗口和更新模拟；新增直接访问局域网 HTTP 地址后的配对、WS 通知、文件完整下载、重连、证书入口移除和 HTTP 手动地址。
- 23 项 Python 发布脚本测试，TypeScript／Vite、Rust fmt／Clippy、工作流 actionlint，以及 Python 启动检查脚本语法。

浏览器测试不导入 CA、不忽略证书错误。CI 使用服务公告的局域网 IP 并确认 HTTP 的实际安全上下文状态；本地受管浏览器的代理无法访问容器网卡，因此使用回环转发验证完整 HTTP 流程，并通过直连请求核对局域网接口可用。日常 CI 只安装 Chromium，移除 Firefox 和 certutil 测试依赖。

## 发布检查

发布使用 v0.5.1 标签对应提交，先执行完整 Checks，再构建 Windows NSIS、Linux DEB／AppImage 和 Android arm64 APK。Linux 在 Xvfb 中分别检查正常启动和 --autostart 模式下的 HTTP 服务与实际版本，局域网探测不走环境代理。

统一通过既有 GitHub Secrets 签名。Android 验证应用 ID、版本名 0.5.1、版本码 501999、签名及 ZIP／ELF 16 KiB 对齐；桌面更新签名验证完整文件、内置公钥及精确版本。

公开前核对四个安装包、三个桌面签名、latest.json 和 SHA256SUMS，共九个资产。公开后重新下载并校验全部内容、签名、APK 证书与版本，确认 Latest 与在线更新指向 0.5.1。已发布的 0.5.0 及更早版本保持不变。

## 验收边界

自动化不替代真实 Windows／Android 系统安装升级、跨设备 Wi-Fi、桌面托盘与登录启动测试。普通 HTTP 不提供传输加密或浏览器音视频安全上下文；会议功能另行设计。
