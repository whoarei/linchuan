# 0.6.0 验证范围

本版覆盖精简浏览器通信、原文件直接流式发送、接收目录单份存储，以及文字和纯图片自动接收。

## 发布前验证

- 已通过 57 项 Windows Rust 回归，覆盖客户端、浏览器和群聊的自动接收，混合附件确认、磁盘空间不足后重试、原文件变化、接收目录固定、URI 文件描述符、取消清理、数据库提交失败恢复和旧缓存迁移。
- 文件流改造此前已通过完整 55 项核心测试，其中两项使用真实 2 GiB + 17 字节文件核对完整字节数与 SHA-256，并检查原文件直传不会在收发应用缓存目录保留整份副本。本次自动接收的本地回归未重复这两项大文件读写；随后 Release 的 Checks 已在标签提交上完整执行 59 项核心测试，并重新通过两项实际大文件校验。
- 34 项 Playwright 流程通过，包含浏览器桌面／手机单会话布局、键盘快捷键、自动上传图片、小图直接显示、大图通过下载管理器接收、普通附件确认和实际下载状态。
- TypeScript／Vite、Rust fmt、核心与 Windows 原生 Clippy 均通过。文件流改造已通过 Android arm64 Debug 构建；系统文件选择器和平台 UI 仍需真机交互验收。

## 正式发布核验

同一标签提交须通过完整 Checks，构建 Windows NSIS、Linux DEB／AppImage 和 Android arm64 APK。Linux 检查正常启动与 --autostart；Android 核对应用 ID、版本名 0.6.0、版本码 600999、既有发布证书及 ZIP／ELF 16 KiB 对齐。

发布前须验证四个安装包、三个桌面签名、latest.json 和 SHA256SUMS，共九个资产。公开后重新核对文件摘要、桌面签名、Android 证书与版本，并确认 Latest 和在线更新入口返回 0.6.0。实际结果如下。

## 实际发布记录

- [v0.6.0 正式版](https://github.com/whoarei/linchuan/releases/tag/v0.6.0)于 2026-11-17 02:00（北京时间）公开并设为 Latest。源码提交为 `e7580fff020920c7e76c1893d11b84498910d0f1`，远端版本标签与发布运行指向同一提交。
- [Release 流水线](https://github.com/whoarei/linchuan/actions/runs/43921896272)全部成功：完整 Checks、Windows NSIS、Linux DEB／AppImage、Android arm64、签名及公开发布。[main 分支 Checks](https://github.com/whoarei/linchuan/actions/runs/43921896423)也已通过。日志确认 59 项核心测试、34 项 Playwright 流程和 23 项签名／发布策略测试全部通过；Linux 正常启动和 --autostart 检查通过。
- 发布后重新下载全部九个资产，核对 SHA256SUMS、远端文件大小与 SHA-256 摘要，以及三个桌面更新签名及精确版本。公开 Latest API 与自动更新入口均返回 0.6.0，在线更新清单的字节与验签后的发布资产完全一致。Debian 包内部版本为 0.6.0，架构为 amd64。
- Android APK 的应用 ID、版本名 0.6.0、版本码 600999、APK 签名、ZIP 16 KiB 对齐、arm64 ABI 及包内全部原生库的 ELF 16 KiB 页对齐均已本地复核。证书 SHA-256 指纹为 `28c461e4f2f8612f9d7717d727e772177474798fd7c48b03ca3ffbca8c81bf0a`，与 0.4.0 起的既有发布证书一致。
- 本次使用现有发布工作流与 GitHub Secrets；既有公开版本和标签保持不变。

## 验收边界

尚未执行 Android 真机系统文件选择器交互、真实 Windows／Android 系统覆盖升级及跨设备 Wi-Fi 验收。浏览器自动下载可能被自身策略拦截，可逐个点击下载；小图自动显示不代表浏览器已经保存到磁盘。原文件外部改名、移动或删除不自动跟踪，传输中断不提供断点续传。
