# 邻传技术方案

当前版本为 0.4.0；Tauri 2 + Vue 3 + TypeScript + Rust，覆盖 Android、Linux、Windows，前端同时用于内置 WebView 和浏览器。

主分支在 0.4.0 之后加入 [HTTPS 设备连接](https-connections.md)，下述连接方案描述该开发状态；已发布 0.4.0 的 HTTP 安装包不因此改变。

0.4.0 的[固定接收目录](receive-storage.md)已实现。`receiving.rs` 在完成接收的事务内创建保存队列，后台流式导出文件；原生 IPC 提供目录设置、保存状态和文件打开。Windows／Linux 使用系统下载目录和目录选择器；Android 使用 MediaStore Downloads、持久授权的 SAF 目录及旧系统存储权限。浏览器下载规则保持由浏览器管理。

0.4.0 的[浏览器临时会话](browser-sessions.md)将现有 24 小时凭证保存在当前 origin 的 localStorage 并同步标签页退出，会话列表根据持久化的 peer_kind 和 has_draft 收起离线浏览器历史，不推断物理设备身份或删除传输记录。

[设备自动命名](device-names.md)使用现有 devices、app_settings 和 browser_sessions 关系，不变更数据库 schema 或设备身份。

Windows／Linux 的[开机启动与托盘](desktop-background.md)使用原生生命周期初始化后台服务，关闭窗口默认隐藏至托盘，系统自启动状态由 autostart 插件管理；close_to_tray 使用 app_settings 保存。

## 模块

- `crates/core/src/model.rs`：设备、会话、消息、传输、群和签名数据类型。
- `store.rs` / `migrations/`：SQLite 版本迁移、事务写入、分页查询和启动恢复。
- `identity.rs`：本机 Ed25519 签名和校验；浏览器 token 的 SHA-256。
- `tls.rs`：持久化本地 CA、设备身份绑定、短期服务证书、地址变更续签及身份固定的 HTTPS 客户端。
- `groups.rs`：群主签名名单、成员签名决定、邀请及成员通知、直连地址核对。
- `conversations.rs`：会话创建、历史分页、已读位置、文字草稿和缓存清理。
- `receiving.rs`：接收目录、附件保存队列、保存记录、失败重试和安全重名处理；Tauri 与 Android 插件负责平台目录访问。
- `api.rs`：配对、授权、文件流、直接传输兼容接口、WebSocket。
- `transport.rs` / `discovery.rs`：逐人传输、取消、mDNS、在线探测。
- `src-tauri/`：原生生命周期、私有目录、下载打开、Android 权限、桌面单实例与打包。
- `src/`：三列/移动会话界面、设置、关于及共用 API。

## 持久化

完整数据模型和关系约束见[数据模型文档](data-model.md)，已在实现前单独提交。

应用私有目录包含 `linchuan.db`、`files/` 和运行期 staging。SQLite 使用 WAL、foreign_keys、synchronous=FULL、busy_timeout；schema 通过事务和 user_version 迁移。数据库操作通过 spawn_blocking 和连接互斥执行。

热数据保存在内存，持久化以规范化表事务提交。元数据比较跳过未改变的传输行；进度节流保存在内存，状态变化及最终结果提交后才确认成功。提交失败回滚内存中的持久状态，上传草稿保留以便重试。最多缓存约 500 条近期/活动传输，旧历史通过数据库分页查询；游标为本机递增 position。

重启将活动任务标为 interrupted，重新探测在线状态，保留完成文件、历史和群成员。未提交完成的文件副本与 `.part` 在恢复时清理。文件完成会 flush/sync 后重命名，在 Unix 上同步父目录，再提交完成状态。文件系统和 SQLite 不是同一个事务，不宣称两者具有原子提交。

## 设备发现与协议

HTTPS 服务绑定 `0.0.0.0:0`，从 listener 读取真实端口。mDNS `_linchuan._tcp.local.` 发布设备 ID、名称、公钥、HTTPS 能力、证书指纹及实际端口；验证证书和信息接口后才保存设备资料，手动 HTTPS 地址作为回退。地址和端口是当前连接信息，不是身份；离线设备记录不会被删掉。本机 WebView 使用另一个仅绑定 `127.0.0.1:0` 的 HTTP 桥接接口，不向局域网暴露或广播。

发现和设备信息使用 protocol=2，新增 https-v1 能力；旧 HTTP 设备须升级，不自动降级。消息载荷仍使用 protocol=1，保留 groups-v1、signed-offers-v1、history-v1 语义；通过 message_id、sender_id、target_id、group_id、created_at 和签名识别消息。旧历史继续保留，不凭名称合并身份。

每次群发送展开为多个 delivery，发送者直接连接接收者。群主维护名单，不中转数据；接收者缓存已验证名单及加入事件，因此群主离线不阻止其他成员互传。浏览器由自己的宿主代理；其他原生成员不依赖该宿主存在。

## 权限与签名

原生 UI 通过 Tauri command 获得本机 owner token；owner token 和配对码每次启动生成。浏览器配对使用随机 Bearer token，数据库仅存 SHA-256 和期限。退出会话撤销凭证，重启后也不会重新授权。

宿主的 Ed25519 私钥保存在私有数据库，不通过 HTTP 暴露。它与 Android APK 发布签名密钥是两套不同用途的密钥。群邀请绑定群主及不可变成员身份，成员只能签署自己的加入/退出；群名单版本、成员事件序号和 invite_id 防止旧事件覆盖新状态。收到消息时校验签名、宿主公钥固定记录、群成员资格及消息一致性。

历史、草稿、确认和下载按 principal_id 检查；不能仅凭会话或文件 ID 访问其他浏览器记录。下载使用短时单文件 ticket，附件作为下载或受控图片预览，不执行用户 HTML/SVG。

局域网内容通过 TLS 加密。每台设备的 ECDSA CA 公钥由既有 Ed25519 身份签名，原生连接校验证书链、地址、有效期和已记录身份；浏览器须显式信任本地 CA。首次设备发现仍属于首次使用信任，不宣称对首次中间人攻击完全免疫。浏览器经宿主代理时，宿主可处理消息，不能将逐段 HTTPS 描述为浏览器到远端成员的端到端加密。

## 文件与状态

文件先通过 HTTPS 流式上传到发送宿主（本机 WebView 经回环桥接），完整后移动到持久文件区。向多个目标发送复用同一个源文件；每位接收者有独立结果和接收副本。群里纯文字在加入确认后直接收录，文件仍需确认；单设备接收保留既有确认行为。

状态为 pending → accepted → transferring → completed，另有 rejected/cancelled/failed/interrupted。只有声明文件完整后才能完成。文件可用性独立于结果；清理缓存保留历史和状态，跳过活动任务。

## 平台与验证

[在线更新方案](updates.md)使用 GitHub Releases、Tauri updater 和独立更新签名密钥，提供桌面签名更新与 Android APK 下载入口。发布任务验证并上传 latest.json、桌面分离签名和完整校验和；更新操作不通过局域网 HTTP API 开放。

### 桌面窗口

Windows、Linux 关闭原生窗口装饰，使用 `DesktopTitleBar.vue` 自绘标题栏和最小化、最大化／还原、关闭按钮；拖动区支持拖动和双击切换最大化，边缘与四角支持缩放。按钮通过 Tauri 窗口 API 操作窗口，resize 事件同步来自系统快捷键等外部操作的最大化状态，旧查询结果不覆盖较新的状态。失焦时减淡标题文字，组件卸载时清理监听。

`desktop_window_controls` 原生命令按编译目标启用桌面标题栏，独立于核心服务启动；服务启动失败时窗口仍可操作。窗口命令权限只授予 Linux、Windows 的 main 窗口；Android 与普通浏览器不显示标题栏或预留空白。桌面最小化后再次启动应用会还原并聚焦已有窗口。

桌面标题栏占用 36px，工作区、启动页和弹窗据此计算可用高度。窗口接口模拟测试覆盖按钮、拖动、双击、边缘缩放、外部状态变化、异步查询顺序、启动失败、操作失败恢复及 Android／浏览器隔离。前端构建、12 项 Playwright 流程、Tauri 配置 schema 和权限标识校验、Android arm64 `cargo check` 已通过。当前执行环境缺少 GTK／WebKitGTK 图形依赖，未在本地运行 Windows／Linux 原生窗口；模拟测试不替代原生窗口实测。

### 安装与验证范围

Windows 使用 WebView2、NSIS；Linux 使用 WebKitGTK 4.1、DEB/AppImage；Android 使用 JDK 21、SDK 36、NDK 27.2，保留本机 HTTP 桥接、局域网使用 HTTPS，前台持有组播锁，arm64 原生库和 APK 支持 16 KiB 对齐。桌面第二次启动聚焦已有窗口，避免原生应用并发访问同一个数据目录。

本轮运行 Rust 核心与数据库集成测试、clippy、TypeScript/Vite、Playwright 会话流程；CI 构建三平台安装产物，检查 Linux 原生启动和 Android ELF 对齐。真机与复杂网络测试范围单独记录，不以浏览器模拟替代 Android 原生真机验收。
