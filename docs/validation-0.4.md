# 0.4.0 验证范围

本版包含固定接收目录、浏览器临时会话重连与历史收起、Windows／Linux 开机启动和托盘，以及两工作流的正式版／RC 发布机制。

## 自动化检查

- Rust 核心 33 项测试：含数据库 schema 1→2 升级、文件保存与同名保护、失败重试、目录变更、浏览器身份隔离、缓存保护、重启去重、保存后数据库失败恢复、桌面关闭偏好持久化和 RC 到正式版更新。
- Playwright 25 项流程：含目录设置与保存动作的原生接口模拟、浏览器关页重连／多标签退出、离线历史和草稿保留、桌面开机启动与关闭策略的设置／拒绝恢复／过时读取、托盘不可用时的退出入口及 Android／浏览器隔离。
- Python 23 项发布策略测试：含版本／锁文件一致、RC 编码、Android 版本元数据、真实 Debian RC 排序和载荷保留、签名篡改／重放、资产归一化、RC 不设 Latest、公开版本不覆盖、分页查找草稿与发布失败恢复和上传摘要校验。
- TypeScript／Vite、Rust fmt／Clippy、工作流 actionlint 和 Bash 语法检查。

## 发布流水线

同一版本标签的提交先执行 Checks，再并行构建 Windows NSIS、Linux DEB／AppImage 和 Android arm64 APK。Linux 在 Xvfb 中验证正常启动和 --autostart 启动时内置服务均可用。Android 检查 ELF／ZIP 的 16 KiB 对齐、应用 ID、版本名称、递增版本码和 APK 证书。桌面签名使用客户端内置公钥验证完整文件和精确版本。

发布任务统一使用 GitHub Secrets，上传四个安装包、三个桌面签名、latest.json 和 SHA256SUMS。全部文件上传并核对名称、长度、SHA-256 后自动公开，正式版设为 Latest，RC 保持 Pre-release。已有正式版和历史资产不覆盖。

## 实际发布记录

- [v0.4.0 正式版](https://github.com/whoarei/linchuan/releases/tag/v0.4.0)于 2026-11-14 发布并设为 Latest；发布源码为 `abaae8f66a129477cc49cd56c3700fe44208ab28`。
- [完整发布流水线](https://github.com/whoarei/linchuan/actions/runs/43615974289)全部通过：Checks、Windows NSIS、Linux DEB／AppImage、Android arm64 和统一签名发布任务。Linux 普通启动与 --autostart 模式的原生服务检查均通过。
- 发布后从公开下载地址重新下载全部九个资产，核对 SHA256SUMS、远端摘要、三个桌面更新签名、精确版本和 Debian 包版本；公开 Latest 与 latest.json 均指向 0.4.0。
- Android APK 的证书验证、应用 ID、版本名称 0.4.0、版本码 400999 和 ZIP 16 KiB 对齐检查通过。证书主题为 `CN=Pindou, OU=Mobile, O=Pindou, L=Shanghai, ST=Shanghai, C=CN`，SHA-256 指纹为 `28c461e4f2f8612f9d7717d727e772177474798fd7c48b03ca3ffbca8c81bf0a`。
- 首次发布在草稿查询阶段中止，草稿未公开。修复 GitHub 按标签接口不返回草稿的问题后，补充草稿分页查找、歧义与失败保护测试，并重新完成全部构建与发布。

## 实机边界

接口模拟不替代 Windows／Linux 真实登录后的开机启动和托盘菜单实测，也不替代真实系统安装器与 Android MediaStore／SAF 目录提供器验收。Linux 托盘取决于桌面环境支持；托盘创建失败会退回普通窗口。Android 本轮不增加后台常驻或开机启动。

多厂商 Wi-Fi、低磁盘、2 GiB 文件与长期运行压力仍需实机验证；局域网传输使用 HTTP，未增加 TLS 或离线消息补发。
