# CI 与 RC 发布

仓库只保留 `Checks` 和 `Release` 两个工作流。日常不打包，签名和三平台构建集中到版本发布。

## 日常检查

`build.yml` 在 main 提交和 PR 时执行，也可手动运行。只有 `docs/**`、Markdown 或 LICENSE 变更时跳过自动检查；这些过滤不影响发布调用检查。

检查包括 TypeScript／Vite、Rust fmt／Clippy、核心与数据库测试、Playwright 浏览器流程，以及 Python 签名和发布策略测试。会编译供测试使用的 Rust 核心程序，不生成 Android／Linux／Windows 安装包。测试报告和截图保存在 Actions。

## 发布

推送 `vX.Y.Z` 或 `vX.Y.Z-rc.N` 标签触发 `release.yml`。也可在 Release 工作流手动填写已有标签重试，不再输入构建编号。标签、前端、Tauri、两个 Rust crate 及两个锁文件的版本必须一致，且存在对应的 `docs/releases/<version>.md`。

流程固定到该标签的提交 SHA：

1. 校验版本、发布说明、Secrets 配置和发布状态。
2. 复用 Checks 执行同一提交的完整检查与测试。
3. 并行构建 Linux DEB／AppImage、Windows NSIS 和 Android arm64 APK。Linux 保留原生启动检查，Android 保留 16 KiB ELF 对齐检查。
4. 下载本次运行的安装包，用 GitHub Secrets 签名 Android APK 和三个桌面更新包，核对 APK 应用 ID、版本码、版本名称、签名及 ZIP 对齐，核对桌面签名、版本与内置更新公钥。
5. 生成更新清单和 SHA256SUMS，上传并回读校验九个资产的文件名、长度和 SHA-256；GitHub 缺少摘要字段时下载对应文件校验。
6. 文件完整后自动公开 Release。内部草稿只用于防止公开不完整产物，无人工批准步骤。失败草稿可以重试，已公开版本拒绝覆盖。

只有发布任务具有 contents:write 权限；检查和平台构建只有读取权限。私钥不提供给平台编译任务，也不放入构建产物。原预签名发布和独立签名验证工作流已删除。

## 正式版与 RC

| 标签 | Release 类型 | Latest | 正式客户端自动更新 |
| --- | --- | --- | --- |
| `v0.4.0-rc.1` | Pre-release | 否 | 不推送 |
| `v0.4.0` | 正式版 | 是 | 正常检查并提示 |

RC 从 1 编号，不接受前导零、beta 或 build metadata。正式版和 RC 都发布四个安装包、三个 `.sig`、latest.json 和 SHA256SUMS。RC 的 latest.json 只属于该预发布；因为 RC 不成为 Latest，正式版更新端点不受影响。RC 客户端仍检查正式版，可在正式版本推出后升级。

Android versionName 使用完整 SemVer；versionCode 为 `major × 10000000 + minor × 100000 + patch × 1000 + 阶段编号`，RC 阶段编号为 N，正式版为 999。minor／patch 小于 100，RC 为 1–998，最终版本码不得超过 Android 的 2100000000 上限；不满足时在构建前明确失败。

| 版本 | Android versionCode |
| --- | --- |
| 0.4.0-rc.1 | 400001 |
| 0.4.0-rc.2 | 400002 |
| 0.4.0 | 400999 |
| 0.4.1-rc.1 | 401001 |

Debian 包内部使用 `0.4.0~rc.1` 确保 dpkg 将 RC 排在 `0.4.0` 之前；应用本身和公开资产名仍使用 `0.4.0-rc.1`。修改发生在签名前，更新签名和校验和覆盖最终包的字节。

## 验证范围

自动化覆盖版本／锁文件一致、发布说明缺失、RC 解析、Android 编码顺序及越界、APK 版本元数据、真实 Debian 包的预发布排序与内容保留、跨平台文件名归一化、签名篡改和版本重放、RC 不设 Latest、已公开版本保护、失败草稿恢复、上传摘要错误阻止公开，以及缺少远端摘要时的下载校验。GitHub 发布操作在测试中模拟，测试不会创建公开 Release。

Tauri CLI 的加密私钥签名已验证可以绑定完整 RC 版本并通过客户端公钥验签；已有 Secrets 的三平台实际签名记录见[开发与发布](development.md)。新流程的完整 RC 安装包构建与公开发布在实际推送版本标签时执行，不以策略单测替代真实平台构建。

本轮本地通过 actionlint、全部工作流 Bash 语法检查、19 项 Python 签名与发布策略测试、32 项 Rust 核心测试及 21 项 Playwright 流程。没有创建新版本标签或公开测试 Release。
