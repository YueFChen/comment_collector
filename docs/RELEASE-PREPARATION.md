# v0.1.2 发布准备

核验日期：2026-10-05。目标仓库：[YueFChen/comment_collector](https://github.com/YueFChen/comment_collector)。

本记录保存打标签前的远端基线、本地校验及安装包摘要。发布后的最终状态与 CI 构建资产以 [v0.1.2 Release](https://github.com/YueFChen/comment_collector/releases/tag/v0.1.2) 为准。

## 远端基线与版本

- GitHub 最新正式发布：[v0.1.1](https://github.com/YueFChen/comment_collector/releases/tag/v0.1.1)，发布时间 2026-10-02T10:46:39Z；没有草稿或预发布。
- 远端 `main` 的 manifest 为 0.1.1，提交为 `2ae76691dd87b962b56d69f2aac5218df77e06e6`，与本地开发基线相同。
- 远端标签只有 `v0.1.0`、`v0.1.1`；`v0.1.2` 尚未创建。
- 下一正式版本确定为 **0.1.2**。四处版本声明和本插件 Cargo.lock 已同步；此前未发布的本地开发编号合并到此版本，不作为新增远端版本。
- 最低 Core 0.1.8；本地 Core 提交与发布 CI 固定提交 `dc988400c4d66ec1c25a5221583efaf80c80db0b` 一致。

## 本地发布门禁

- 冻结依赖安装、UI 类型检查、23 个 UI 测试通过。
- Rust 格式、Clippy 全 targets/features（无警告）、49 个 Rust 测试、Rust/TypeScript 绑定一致性通过。
- Release 优化构建与包内后端 mock Core 协议通过。协议覆盖启动自动调度、受控匿名网络、当前归档新增统计、下次采集时间、暂停及目标操作、已移除接口拒绝调用。
- ZIP CRC、包内版本/平台、Windows x64 PE、所有内部文件 SHA-256 覆盖和历史接口清理校验通过；`git diff --check` 通过。
- 发布工作流增加类型绑定及包内协议门禁，使用 [RELEASE-NOTES.md](../RELEASE-NOTES.md) 作为发布正文。该工作流改动尚未在 GitHub Actions 上执行。

## 待发布资产

- 本地包：`target/comment_collector-0.1.2-windows-x86_64.wplug`
- 字节数：`1443979`
- SHA-256：`de1fc46894ae4c44ece6f2a47b029f67c3ec557aec589c5235114105c2b3d9f6`
- 摘要文件：同路径追加 `.sha256`；机器校验记录：`target/release-preparation-0.1.2.json`。

GitHub Actions 将重新构建该版本，再使用已存在的 `PLUGIN_UPDATE_SIGNING_KEY` 和 `PLUGIN_UPDATE_SIGNING_PUBLIC_KEY` 生成 `comment_collector-update.json`。未读取、复制或替换签名私钥。Catalog 已登记该插件，公钥与仓库变量一致，本次普通版本更新无需修改 Catalog。

## 发布状态与后续核验

准备阶段未提交、推送或创建发布标签。2026-10-05 已获授权进入发布步骤：再次确认远端 main 与版本标签未被其他发布改变，提交已验证改动并推送 main 和 `v0.1.2`，由标签工作流完成签名和发布。

工作流成功后，下载线上 `.wplug` 和更新清单，用 Catalog 登记公钥验证 Ed25519 签名、清单中的包大小/SHA-256 与下载内容、安装包内部 checksums，并确认 stable latest 地址指向 0.1.2。打标签前尚未验证 GitHub CI、正式签名或线上更新；没有进行真实 Core GUI 安装、长期监测、远程会话或休眠/唤醒验收。
