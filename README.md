# 奇域评论采集器

`comment_collector` 是 Wonderland Assistant 的独立插件，用于无需登录地采集奇域关卡公开评论、保存在本地并导出。归档和导出会包含评论公开字段，例如昵称、UID、IP 归属地、内容、点赞数和回复关系。插件不会读取或发送用户的米游社账号凭据。

插件使用 [Core 仓库](https://github.com/YueFChen/Wonderland_Assistant)中的 UI SDK、协议 crate 和受控网络/文件服务。Core 不静态依赖本仓库的业务代码。

## 功能与能力

- 采集指定关卡的评论并汇报进度；归档记录最近一次采集是完整完成还是中断，失败或取消时保留已抓取数据。
- 浏览本地归档时由插件按主评论分页读取，并在后端执行筛选、搜索和排序，避免通过 Core UI 通道传输整份归档；仍可导出完整 JSON、CSV 或 Excel 文件。
- 可列出损坏或不兼容的归档并提示重新采集；用户启动重新采集后，新归档直接替换原文件，不额外创建备份。
- 在关卡页收藏或取消收藏，并从入口页快速打开收藏关卡；收藏只保存在本机插件数据中，不同步米游社游戏账号。
- 只注册一个 Workspace Activity；采集进度通过 `collect.progress` 事件发送，并按请求 ID 关联。
- 清单申请 `network.public`、`files.export` 和 `files.reveal_own`，公共网络目标限定为 `bbs-api.miyoushe.com`。请求不携带账号凭据；导出和打开导出目录由 Core 文件服务处理。插件不申请账号能力，也不接收任意文件系统路径。

## 插件服务

插件提供 `wonderland.comments.archive` v1.0.0，供其他已获授权的插件按需查看本机归档。服务提供 `service_archives` 与 `archive_page`，单页最多 50 条。跨插件结果省略 UID、头像 URL、IP 归属地及评论内部 ID；普通本地归档与用户主动导出的文件仍保留其原有字段。服务不会自动采集、推送或复制归档。

安装后需在插件管理页授权所申请的能力。未授权网络能力时，插件仍可浏览本地归档。

## 构建

使用 Core 0.1.8 源码（提供插件 SDK 0.1.8），把本插件检出到 `plugins/comment_collector` 后，从插件目录运行：

```powershell
# Debug 目录包，供 Debug Core 本地安装
pnpm build

# Release .wplug，供 GitHub Release 和目录校验
node scripts/build-plugin.mjs --release --archive
```

首次发布和在线目录登记流程见 [RELEASING.md](RELEASING.md)。

Debug 包位于 `target/comment-collector-plugin`；Release 包名为 `comment_collector-{版本}-windows-x86_64.wplug`。Release 命令会构建优化版后端和 UI，并生成覆盖包内文件的 SHA-256 `checksums.json`。

插件支持 Core `[0.1.8, 1.0.0)` 和插件 UI Bridge `[1.1.0, 2.0.0)`。Debug Core 可安装 Debug 目录包；Release Core 安装经过校验的 `.wplug` 包。插件 UI 在当前 Core 的 Debug 和 Release 构建中均可使用。

## 数据与导出

归档和本地收藏保存在 Core 为插件提供的 `plugin-data/comment_collector/` 数据目录。插件按关卡 ID 保存累积归档；评论按 ID 去重，重复采集时以较新的公开数据更新旧记录。v1 归档会在读取时迁移到当前格式；损坏或不兼容的文件会留在清单中供用户重新采集。CSV 会对可能被表格软件解释为公式的网络文本加前缀转义。Core 的单文件导出上限为 20 MiB；超过上限时插件会提示错误，本地归档仍保留。

## 包结构

- `package/manifest.json`：插件身份、兼容范围、Activity、主题集成和能力声明。
- `package/contract.json`：归档查询、采集、导出及进度事件的数据契约。
- `ui/`：插件页面及宿主 API 适配。
- `src/lib.rs`、`src/bbs.rs`、`src/export.rs`：本地归档、匿名社区接口适配和导出逻辑。

## Core 0.1.8 与远程访问

见 [CORE-COMPATIBILITY.md](CORE-COMPATIBILITY.md)，包括最低版本、远程声明和发布顺序。
