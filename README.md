# 奇域评论采集器

`comment_collector` 是 Wonderland Assistant 的独立插件，用于无需登录地采集奇域关卡公开评论、保存在本地并导出。归档和导出会包含评论公开字段，例如昵称、UID、IP 归属地、内容、点赞数和回复关系。插件不会读取或发送用户的米游社账号凭据。

插件使用 [Core 仓库](https://github.com/YueFChen/Wonderland_Assistant)中的 UI SDK、协议 crate 和受控网络/文件服务。Core 不静态依赖本仓库的业务代码。

## 功能与能力

- 采集指定关卡的评论并汇报进度；采集失败或取消时保留已抓取数据。
- 浏览本地归档，再导出 JSON、CSV 或 Excel 文件。
- 只注册一个 Workspace Activity；采集进度通过 `collect.progress` 事件发送，并按请求 ID 关联。
- 清单申请 `network.public`、`files.export` 和 `files.reveal_own`。网络请求不携带账号凭据；导出和打开导出目录由 Core 文件服务处理。插件不申请账号能力，也不接收任意文件系统路径。

安装后需在插件管理页授权所申请的能力。未授权网络能力时，插件仍可浏览本地归档。

## 构建

在 Core 仓库检出本插件到 `plugins/comment_collector` 后，从插件目录运行：

```powershell
# Debug 目录包，供 Debug Core 本地安装
pnpm build

# Release .wplug，供 GitHub Release 和目录校验
node scripts/build-plugin.mjs --release --archive
```

Debug 包位于 `target/comment-collector-plugin`；Release 包名为 `comment_collector-{版本}-windows-x86_64.wplug`。Release 命令会构建优化版后端和 UI，并生成覆盖包内文件的 SHA-256 `checksums.json`。

Debug Core 的插件管理页可安装 Debug 目录包。运行 Core 时需启用其 Debug 专用的 `WONDERLAND_PLUGIN_UI_ISOLATION_TEST=1` 开关。Release Core 当前明确关闭动态插件 UI；正式发布前还需由 Core 完成 WebView 隔离方案验证并开放生产插件 UI。

## 数据与导出

归档保存在 Core 为插件提供的 `plugin-data/comment_collector/` 数据目录。插件按关卡 ID 保存累积归档；评论按 ID 去重，重复采集时以较新的公开数据更新旧记录。CSV 会对可能被表格软件解释为公式的网络文本加前缀转义。

## 包结构

- `package/manifest.json`：插件身份、兼容范围、Activity、主题集成和能力声明。
- `package/contract.json`：归档查询、采集、导出及进度事件的数据契约。
- `ui/`：插件页面及宿主 API 适配。
- `src/lib.rs`、`src/bbs.rs`、`src/export.rs`：本地归档、匿名社区接口适配和导出逻辑。
