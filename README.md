# 邻传 · Linchuan

**同一网络，随手互传。**

支持 Android、Linux、Windows 的局域网文字、图片与文件传送工具。应用间直接互传；未安装应用的设备可通过浏览器打开对方地址，配对后双向收发。每台应用使用系统分配的可用端口，无需独立服务器。

[下载 v0.1.0](https://github.com/whoarei/linchuan/releases/tag/v0.1.0) · [产品方案](docs/product-plan.md) · [技术方案](docs/technical-plan.md) · [开发与发布](docs/development.md) · [验证范围](docs/validation.md)

![邻传桌面界面](docs/images/desktop.png)

## 使用

1. 在至少一台设备安装并打开邻传，确保设备连接同一局域网。
2. 应用之间从附近设备列表选择接收者；发现不到时输入对方应用展示的完整地址。
3. 浏览器通过地址或二维码进入，输入对方应用显示的配对码后加入。
4. 添加文字、图片或文件，点击发送；对方确认后接收，可复制文字或下载文件。

单文件最大 2 GiB，每次最多 64 个文件、8 个目标。应用和网页需保持运行。收到的文件先保存在临时目录，需要长期保留时请下载另存；应用中的下载入口会打开系统浏览器。

## 平台

| 平台 | 首版产物 |
| --- | --- |
| Android 7.0+ | arm64 APK，发布签名 |
| Linux x86_64 | DEB / AppImage，Ubuntu 22.04+ 或兼容发行版 |
| Windows 10/11 x64 | NSIS 安装包，需要 WebView2 |
| 浏览器 | 现代 Chrome / Edge / Firefox / Safari，免安装连接运行中的应用 |

Linux AppImage 如遇 FUSE 环境缺失，可使用 `./Linchuan*.AppImage --appimage-extract-and-run`。Windows 首次运行需要允许应用接收局域网连接。

## 当前边界

无账户、无云端、无外部 CDN。首版使用 **HTTP，未实现传输加密，仅用于可信局域网**。不提供公网穿透、断点续传或手机后台常驻保证。随机端口解决端口占用问题，不能绕过防火墙和路由器设备隔离。

Android/Windows 真机兼容性与真实 Wi-Fi 跨设备发现仍需持续验证；详见[验证记录](docs/validation.md)。

## 开发

```bash
npm ci
npm run tauri dev
```

需要 Rust stable、Node.js 22 及对应平台的 Tauri 系统依赖。详见[开发文档](docs/development.md)。
