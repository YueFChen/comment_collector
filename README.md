> This is an independently versioned plugin repository. Local builds use the stable Core SDK from the adjacent `Wonderland_Assistant` checkout; see [Core repository boundaries](../../Wonderland_Assistant/docs/REPOSITORY-BOUNDARIES.md).

# 评论采集器插件

`comment_collector` 是独立动态插件，用于匿名采集奇域关卡评论、本地归档和导出。插件包含自己的 manifest、contract、Vite UI 与 Rust stdio 后端；Core 不静态导入其页面、业务 crate 或命令。

## 功能

- 采集指定关卡的评论并汇报进度。
- 浏览本地归档，再导出 JSON、CSV 或 Excel 文件。
- 只注册一个 Workspace Activity：`main`；插件页面直接占用 Core 提供的内容区。
- 采集进度使用 `collect.progress` 事件，并按请求 ID 过滤；取消只作用于对应的在途采集。

## Core 能力

清单申请 `network.public`、`files.export` 和 `files.reveal_own`。匿名 HTTP 请求由 Core 的公共网络服务代理；导出和打开导出目录也由 Core 文件服务处理。插件不申请账号能力，不接收任意文件系统路径。

首次使用前，在插件管理页为该插件授予所申请的能力。若网络能力未授权，采集会返回 Core 的授权错误；本地归档浏览不需要发起网络请求。

## 构建和安装

在仓库根目录运行：

```powershell
pnpm build
```

生成的 Windows x86_64 MSVC debug 包位于 `target/comment-collector-plugin`。启动 debug Core，在“设置 → 插件管理”安装并启用该目录，然后从 Workspace 的“评论采集器”主入口打开。

## 数据与导出

新归档写入 `%APPDATA%\com.wonderland.assistant\plugin-data\comment_collector\archive-v1\`。旧归档是开发测试样本，不导入。导出路径由 Core 选择并返回，插件只接收生成文件的结果路径。

## 目录与契约

- `package/manifest.json`：插件兼容范围、唯一 Activity、主题集成和能力声明。
- `package/contract.json`：归档查询、采集、导出及进度事件的数据契约。
- `ui/src/main.tsx`：UI Host SDK 调用和请求关联适配；交互页面位于 `entry.tsx`、`view.ts`、`result.tsx` 等模块。
- `src/lib.rs`：归档、采集与导出业务逻辑。
- `src/main.rs`：stdio 插件协议及 Core 网络/文件服务适配。

更改方法或事件时，需同步更新 `package/contract.json`、`ui/src/types.generated.ts` 及 UI API 适配。插件包由 `scripts/build-plugin.mjs` 生成；debug 包不包含发布签名。
