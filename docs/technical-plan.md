# 邻传 Linchuan 技术方案

版本：0.1.0 · 技术基线：Tauri 2 + Vue 3 + TypeScript + Rust

## 1. 架构

```text
应用 A                                  应用 B
Vue UI                                  Vue UI
  │                                       │
Rust 核心 ───── HTTP 直接传输 ───────── Rust 核心
  │                                       │
随机 TCP 端口 + 内嵌网页                 随机 TCP 端口 + 内嵌网页
  ↑                                       ↑
浏览器会话 A                            浏览器会话 B

各应用通过 mDNS/DNS-SD 发布和发现实际地址、端口；没有中心注册或中转节点。
```

浏览器访问某应用后，由该应用提供网页及该会话的收发能力。浏览器到宿主是直接连接；浏览器通过宿主发送到其他应用时，宿主作为该次发送的本地代理。应用到应用不经过第三台机器。多目标分别发送，不宣称局域网广播文件。

## 2. 模块与目录

- `src/`：共用 Vue 界面、API 客户端、类型、样式；不引用 CDN。
- `crates/core/`：不依赖 Tauri 的收发核心，便于运行集成测试和无界面实例。
- `src-tauri/`：生命周期、设备数据目录、原生桥接、系统打开下载链接、打包配置。
- `src-tauri/gen/android/`：Android 原生工程、权限与组播锁。
- `tests/`：浏览器端到端测试；Rust 协议测试放在核心 crate。
- `.github/workflows/`：验证、跨平台构建、发布。

## 3. 网络与发现

HTTP 服务采用 Axum/Tokio，绑定 `0.0.0.0:0`，从 listener 获取真实端口后才发布地址。不自行随机挑端口，避免检查与绑定间竞争。服务端口每次启动可变化；设备 ID 在应用数据目录持久化。

使用 mDNS/DNS-SD `_linchuan._tcp.local.`，TXT 包含协议版本、设备 ID、名称与平台；端口在 SRV 中发布。底层 mDNS 使用标准发现端口，与随机业务端口分开。维护带过期时间的设备表，提供手动地址探测作为回退。不把发现信息当作身份认证。

只向本地子网提供功能；不自动配置 UPnP、不启用公网隧道。手动连接只允许局域网/回环 IPv4 地址和合法端口，拒绝 URL 凭据、重定向及非 HTTP 协议，降低本地 API 被当作任意 URL 代理的风险。

## 4. 身份与会话

原生 UI 通过 Tauri command 获取回环地址与仅存内存的 owner token；远程网页无法获得该 token。网页输入本次启动配对码，服务签发随机浏览器 token，所有业务请求携带 Bearer token。配对有速率限制，token 不写日志、不放查询串。

设备 ID 是展示与发现标识，首版不具备密码学身份认证。远端 offer 无需永久信任，但有大小、并发与速率限制；每个 offer 返回随机传输凭证，仅授权该次传输状态查询、上传和取消。接收确认必须由 owner 或目标浏览器会话执行。

下载使用短时、限定到单个文件的随机 ticket，避免普通浏览器下载 API 不能设置 Authorization 的问题。响应加 no-store、nosniff 和安全的 Content-Disposition。图片不直接以内联 HTML/SVG 执行。

HTTP 首版不加密；这是明确的版本限制，不用随机端口或配对码代替加密。TLS 与可信身份配对在后续版本实现。

## 5. 传输模型

文件先以流式 HTTP 上传到发送宿主的 draft 目录，再向一个或多个接收方发送元数据请求。这样浏览器和应用采用同一协议；代价是发送端会额外占用临时磁盘，并非零拷贝。发送前检查文件数、单文件大小及存储预算。

状态机：`pending → accepted → transferring → completed`；另外允许 `rejected / cancelled / failed` 终态。只有所有声明的文件均按期望字节数落盘后才能完成。上传写入 `.part`，完整后重命名；实际路径只使用服务端 UUID，原始文件名仅用于展示与下载。

应用间传输由后台 task 查询接收状态并流式上传，带连接、确认及传输超时，不跟随 HTTP 重定向；多人逐个维护状态。接收方未同意前不接受远程内容上传。取消与完成提交需要检查当前状态，避免并发覆盖终态。

浏览器收件使用本地宿主中的会话级收件箱。文字直接作为受限 JSON，文件通过受保护下载。实时状态使用认证 WebSocket 通知触发重新拉取；断线后重连，定期刷新兜底。WebSocket 连接建立后先发送 token，认证前不发送业务数据。

## 6. API 概览

- `GET /api/info`：公开名称、版本及配对需求，不包含配对码或 token。
- `POST /api/pair`：配对码、浏览器名称换取会话凭证。
- `GET /api/state`：当前身份可见设备与传输记录。
- `POST /api/peers`：手动连接并探测局域网设备。
- `POST /api/drafts`、`PUT /api/drafts/{id}/files/{file}`：创建草稿及流式上传。
- `POST /api/drafts/{id}/send`：创建单个或多个目标传输。
- `POST /api/transfers/{id}/decision`：接收或拒绝。
- `POST /api/transfers/{id}/cancel`：取消。
- `POST /api/transfers/{id}/files/{file}/ticket`：获取短时下载地址。
- `/api/peer/*`：受传输凭证保护的应用间 offer、状态、上传接口。
- `GET /api/events`：WebSocket，先认证后订阅变化。

具体字段以代码中的 serde 类型为准；协议前缀含版本，遇到未知协议版本拒绝传输并提示升级。

## 7. 存储与资源

持久目录只保存设备 ID、名称与设置。临时传输目录按进程启动隔离，上次进程的残留在下次启动清理；运行时定期清理过期草稿、会话和传输，用户手动保存的文件不受影响。使用计数预算限制临时总量，不将所有文件读入内存。文件流并发和 API 请求体均设上限。

## 8. 平台适配

- Windows x64：NSIS 安装包；需要 WebView2，安装流程处理缺失环境；首次运行允许应用通过防火墙。
- Linux x64：DEB + AppImage，基于 Ubuntu 22.04 构建以兼顾兼容性；运行依赖 WebKitGTK 4.1。
- Android arm64：Tauri 2 原生工程，INTERNET、网络状态和 Wi-Fi 组播权限；应用前台持有 multicast lock，退出释放；允许局域网 HTTP。文件通过系统选择器获取，不申请全盘存储权限。保存入口调用系统浏览器的下载功能。

首版不提供后台常驻保证，不把 Android 网页入口包装成单独中心服务。应用关闭后监听与发现一并停止。

## 9. 验证与发布

本地：TypeScript 检查、生产构建、Rust fmt/clippy/test、两个独立核心实例的真实 HTTP 集成测试、Playwright 浏览器测试。集成测试覆盖配对隔离、接收确认、中文文件、多文件字节一致、拒绝、取消、非法路径、超限、随机端口与设备 ID 持久化。

CI：Linux 跑检查与集成测试，Windows/Linux 构建桌面安装包，Linux runner 使用 Android SDK/NDK 构建 arm64 release APK。所有构建成功后汇总产物、生成 SHA256SUMS，再创建 GitHub `v0.1.0` Release；不先发布空 Release。Android 使用持久发布密钥，密钥仅存 GitHub Actions secrets，绝不提交仓库。若环境未提供密钥，先生成专用发布密钥并存入该仓库 secrets，不能用每次变化的临时密钥冒充可升级发布。

发布说明列出功能、安装方法、HTTP/前台运行限制、自动化与真机验证范围。三端编译通过不等价于三端真机验收。
