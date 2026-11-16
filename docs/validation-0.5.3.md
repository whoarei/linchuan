# 0.5.3 验证范围

本版增加动态磁盘空间预算、大文件无进度超时、活动草稿清理保护，以及原生文件复制和移动。

## 发布前已通过的检查

- 46 项 Windows Rust 核心测试，覆盖同盘和不同盘预算、并发准入、已写入字节不重复计费、取消后释放预留、活动上传清理保护、持续超过一小时的上传和五分钟无数据超时。
- 真实 2 GiB + 17 字节文件以有界内存完成 HTTP 上传和下载，完整字节数及 SHA-256 一致。最终实现重新执行该检查通过。
- 文件操作回归覆盖复制不更新原位置、重名不覆盖、移动后位置持久化、重启与清理缓存后继续读取，以及源文件删除或 SQLite 写入失败时保留原文件与原记录。
- 27 项 Playwright 流程通过；针对复制／移动追加桌面、手机四个操作图标、取消、失败提示、位置刷新与无横向溢出检查。
- Rust fmt／Clippy、TypeScript／Vite、Windows 原生编译检查和 Android arm64 Debug APK 构建通过。
- 本次发布前核对版本号与两个锁文件一致，发布策略准备检查通过，Android 正式版本码为 503999；23 项 Python 签名与发布脚本测试在 WSL Ubuntu 通过，包含真实 Debian 包版本检查。

浏览器的原生 IPC 行为通过接口模拟验证；核心文件操作测试使用真实磁盘文件和 SQLite。Android APK 构建成功不等于已在真机上运行文件选择器或复制／移动。

## 发布检查要求

v0.5.3 的同一标签提交需通过完整 Checks，再构建 Windows NSIS、Linux DEB／AppImage 和 Android arm64 APK。Linux 分别检查正常启动与 --autostart；Android 核对应用 ID、版本名 0.5.3、版本码 503999、既有发布证书及 ZIP／ELF 16 KiB 对齐。

公开前核对四个安装包、三个桌面签名、latest.json 和 SHA256SUMS，共九个资产。公开后重新下载核对文件摘要、桌面更新签名、Android 证书和版本，并确认 Latest 与公开在线更新入口返回 0.5.3。

## 实际发布记录

- [v0.5.3 正式版](https://github.com/whoarei/linchuan/releases/tag/v0.5.3)于 2026-11-16 公开并设为 Latest。发布源码为 `a3e567671a03231950c3828473ac6ec3845789b8`，远程标签和发布运行指向同一提交。
- [正式 Release 流水线](https://github.com/whoarei/linchuan/actions/runs/43819927532)全部成功：完整 Checks、Windows NSIS、Linux DEB／AppImage、Android arm64、统一签名与发布。[main 分支 Checks](https://github.com/whoarei/linchuan/actions/runs/43819927434)也已通过。云端完整核心测试包含实际超过 2 GiB 的上传和下载，Linux 正常启动及 --autostart 检查通过。
- 发布后重新下载全部九个资产，核对 SHA256SUMS、远端大小和 SHA-256 摘要，以及三个桌面更新签名及精确版本。公开 Latest API 和 `releases/latest/download/latest.json` 均返回 0.5.3；Debian 包内部版本为 0.5.3，架构为 amd64。
- Android APK 的应用 ID、版本名 0.5.3、版本码 503999、APK 签名、ZIP 16 KiB 对齐、arm64 ABI 及所有原生库的 ELF 16 KiB 页对齐均通过本地复核。证书 SHA-256 指纹为 `28c461e4f2f8612f9d7717d727e772177474798fd7c48b03ca3ffbca8c81bf0a`，与 0.4.0 起的既有发布证书一致。
- 发布复用现有 GitHub Actions 与 Secrets，既有公开版本及标签保持不变。

## 验收边界

本轮尚未在 Android 真机操作系统目录选择器或执行文件复制／移动，未执行真实 Windows／Android 系统覆盖升级、跨设备 Wi-Fi、托盘或登录启动验收。无法提供本机容量信息的 Android 文档提供器会明确提示选择本机目录。外部改名或移动不自动跟踪，传输中断不提供断点续传。
