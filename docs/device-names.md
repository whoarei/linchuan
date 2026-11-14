# 设备自动命名

状态：在 main 开发，尚未包含于已发布的 0.2.0 安装包。

## 使用规则

- 客户端首次启动时优先使用系统主机名，例如“小林的电脑”。读取失败或得到 localhost 等通用名称时，使用“系统类型 · 四位短编号”，例如“Android · A7F2”。短编号取设备 ID 的末四位，名称保存后重启保持不变。
- 浏览器首次打开时，按系统、浏览器类型和随机短编号预填名称，例如“Windows · Edge · A7F2”。只填配对码就能连接，名称仍可编辑；清空名称也会采用自动名称。
- 设置页允许客户端和浏览器修改自己的名称。“使用自动名称”填入建议名称，点击“保存名称”后生效。已有安装、迁移数据和用户保存的名称优先沿用，不因升级被覆盖。
- 浏览器在同一 origin 的 localStorage 中分别记住自动建议与最近使用的名称。隐私模式、清理存储或随机服务端口变化后，名称可能重新生成。名称记忆不等于恢复配对身份或历史。

系统名称、User-Agent 只提供可读建议，不保证识别具体硬件型号。短编号用于区分显示，允许重名；设备身份始终由设备 ID、凭证和公钥决定。

## 实现与数据

`crates/core/src/names.rs` 生成客户端默认名以及没有提交名称的浏览器 API 请求的默认名。客户端读取系统主机名，移除控制字符、去除首尾空白并截取至 64 字符；通用名称走回退规则。网页的 `src/names.ts` 生成并记住浏览器名称。

`POST /api/pair` 的 name 字段可省略或留空，返回实际采用的 name。`POST /api/settings` 根据调用凭证修改当前 principal 对应的 devices.name；浏览器同时更新自己的运行期 Session.name。客户端改名仍更新 app_settings.device_name 和 mDNS 公告。系统主机名建议仅通过 owner 的 state.me.auto_name 返回，不在公共 info 或其他浏览器的状态中另行暴露。

使用现有数据库结构，无需新增表或迁移版本。改名不改变 devices.id、browser_sessions.device_id、群成员 ID 或会话 ID，也不改写历史消息中的发送名称快照。事务失败沿用现有回滚机制。

## 验证范围

覆盖系统名称清理与回退、重叠 User-Agent 识别、客户端默认名保存及手动覆盖、浏览器省略/空名称配对、同名不同身份、浏览器仅修改自身名称、改名后会话和历史保留、重启恢复，以及浏览器自动预填、免填名称配对、编辑和同地址记忆。三平台构建沿用 CI；系统名称在具体 Android 厂商设备上的可用性仍需真机验证。

本地检查：`npm run build`、`cargo fmt --all -- --check`、`cargo clippy -p linchuan-core --all-targets --locked -- -D warnings`、`cargo test -p linchuan-core --locked`（3 项单元测试、18 项集成测试）及 `npm run test:e2e`（7 项流程）。
