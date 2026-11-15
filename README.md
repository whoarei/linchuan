# 邻传 · Linchuan

**同一网络，随手互传。**

当前正式版为 **0.4.0**：新增固定接收目录、浏览器临时会话整理，以及 Windows／Linux 开机启动和系统托盘。

支持 Android、Linux、Windows 的局域网文字、图片与文件传送工具。应用间直接互传；未安装应用的设备可通过浏览器打开对方地址，配对后双向收发。每台应用使用系统分配的可用端口，无需独立服务器。

[下载 v0.4.0](https://github.com/whoarei/linchuan/releases/tag/v0.4.0) · [产品方案](docs/product-plan.md) · [技术方案](docs/technical-plan.md) · [会话与群聊数据模型](docs/data-model.md) · [在线更新](docs/updates.md) · [开发与发布](docs/development.md) · [0.4 验证范围](docs/validation-0.4.md)

0.4.0 提供[固定接收目录](docs/receive-storage.md)：客户端接收文件、图片后自动保存到系统下载目录下的“邻传”文件夹，可在设置中更改、打开或恢复默认目录。同名文件自动编号，保存失败可重试；清理应用缓存不会删除已保存文件。浏览器仍使用自己的下载设置。

0.4.0 同时整理了[浏览器临时会话](docs/browser-sessions.md)：同一浏览器在同一访问地址下复用 24 小时内的有效配对凭证，关页重开或新标签页不会反复创建身份；离线浏览器的传输历史默认收起，空会话不再显示。浏览器与原生客户端保持独立。

[CI 与发布](docs/ci-release.md)已简化为两个流程：日常只做检查和测试，纯文档修改跳过；版本标签触发三平台构建、GitHub Secrets 签名和自动发布。支持 `vX.Y.Z-rc.N` 预发布，RC 不设为 Latest，也不推送给正式版用户。

Windows／Linux 提供[开机启动和托盘](docs/desktop-background.md)，设置可启用登录后后台启动；关闭窗口默认保留在托盘，托盘菜单可恢复窗口或退出。Android 本轮不增加后台常驻。

主分支新增 [HTTPS 设备连接](docs/https-connections.md)，为后续会议能力建设连接基础。设备间与浏览器访问使用随机 HTTPS 端口，客户端核对设备身份；浏览器首次需核对指纹并信任对应设备的本地 CA。此改动尚未包含在已发布的 0.4.0 安装包中，旧 HTTP 客户端需要随下一版升级。

![邻传桌面界面](docs/images/desktop.png)

## 使用

1. 在至少一台设备安装并打开邻传，确保设备连接同一局域网。
2. 从附近设备或会话列表打开设备；发现不到时点击添加，通过地址连接。
3. 浏览器通过地址或二维码进入，输入对方应用显示的配对码后加入。
4. 在会话底部发送文字、图片或文件；多人发送通过新建群聊完成，各成员确认加入。文件可以分别接收和下载。

会话、群成员、文字草稿和附件记录保存到 SQLite，完整文件保存在应用目录，重启后继续保留。设置中可手动改名或“换个名字”、查看配对码和清理文件缓存；清理保留历史卡片。单文件最大 2 GiB，每次最多 64 个文件，每群最多 16 位成员。应用和网页需保持运行；群聊不自动补发离线消息。

Windows／Linux 的 0.3.0 用户可在“关于”中检查更新；更早版本可手动下载安装 0.4.0。Android 本版更换发布密钥，旧版需要卸载后重装；卸载前请保留需要的历史和文件。

## 平台

| 平台 | 安装产物 |
| --- | --- |
| Android 7.0+ | arm64 APK，发布签名 |
| Linux x86_64 | DEB / AppImage，Ubuntu 22.04+ 或兼容发行版 |
| Windows 10/11 x64 | NSIS 安装包，需要 WebView2 |
| 浏览器 | 现代 Chrome / Edge / Firefox / Safari，免安装连接运行中的应用 |

Linux AppImage 下载后先执行 `chmod +x Linchuan_0.4.0_amd64.AppImage`。如遇 FUSE 环境缺失，可使用 `./Linchuan*.AppImage --appimage-extract-and-run`。Windows 首次运行需要允许应用接收局域网连接。

## 当前边界

局域网传输无需账户或云端中转，界面不依赖外部 CDN；手动检查更新会连接 GitHub。已发布的 0.4.0 使用 HTTP；主分支的局域网通信已改为 HTTPS，浏览器的音视频安全上下文取决于证书信任，首次认识设备仍采用首次使用信任。不提供公网穿透、断点续传或手机后台常驻保证。随机端口解决端口占用问题，不能绕过防火墙和路由器设备隔离。

Android/Windows 真机兼容性、实际系统覆盖升级与真实 Wi-Fi 跨设备发现仍需持续验证；详见[本轮验证记录](docs/validation-0.3.md)。

## 开发

```bash
npm ci
npm run tauri dev
```

需要 Rust stable、Node.js 22 及对应平台的 Tauri 系统依赖。详见[开发文档](docs/development.md)。
