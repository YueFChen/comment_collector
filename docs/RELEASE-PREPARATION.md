# v0.1.3 发布准备

核验日期：2026-10-07。目标仓库：[YueFChen/comment_collector](https://github.com/YueFChen/comment_collector)。用户已授权提交、推送并发布本次修复为远端 0.1.3。

本记录保存发布前的基线与验证。发布后的最终状态、CI 结果和签名资产以 [v0.1.3 Release](https://github.com/YueFChen/comment_collector/releases/tag/v0.1.3) 为准。

## 远端基线与版本

- 发布前远端 main 为 `393b34591097f5ac4316382a4433b49442d0c219`，与本地开发基线一致。
- 最新正式发布为 v0.1.2；本地和远端标签只有 v0.1.0、v0.1.1、v0.1.2，目标 v0.1.3 尚不存在。
- 发布版本 0.1.3 已同步至 Cargo workspace、插件 Cargo.lock、根/UI package.json 和插件 manifest。
- 最低 Core 0.1.8；发布 workflow 固定 SDK 提交 `dc988400c4d66ec1c25a5221583efaf80c80db0b`。当前 Core 工作目录为 0.1.9，本轮使用固定 SDK 的隔离副本验证，没有升级插件 SDK 或修改 Core。
- 仓库已有签名 secret `PLUGIN_UPDATE_SIGNING_KEY`，公开变量与 Catalog 登记公钥一致：`f96a9698f8e24481df2eff31d6d654636693d53beba24329837ec61f789ce5c7`。只核验 secret 名称，未读取或替换私钥。
- main 没有 required status checks；当前唯一 workflow 在版本标签 push 时执行。先提交并推送已完成本地验证的改动，再推送匹配标签，由该 workflow 完成全套 CI、签名及发布。

## 发布内容

- 普通/全量主评论从 HOT 开始，沿用服务端 next 与排序，允许正常 HOT→FLOOR_DESC 阶段切换；增量监测仍从 FLOOR_DESC 开始，楼中楼保持独立 ASC 分页。
- 成功的手动全量立即保存为监测基线，添加目标与重启后复用；后台过期结果不会覆盖新基线。
- 全量到期独立计算，不再被近期增量推迟；界面使用相同到期规则，失败冷却保持原有行为。
- 发布 workflow 新增包内基线复用及独立全量调度协议门禁。

## 本地门禁与资产

冻结安装、UI 类型检查及 23 个 UI 测试；Rust 格式、全 targets/features Clippy 无警告、54 个 Rust 测试、绑定一致性均通过。Release 包解压后的实际后端通过两组 mock Core 协议验证。ZIP CRC、manifest/contract、全部内部 checksums、Windows x64 PE、包与源码一致性以及 git diff 检查通过。

实际包内后端对风之剑 `54260057081` 做了匿名全量：1446 条主评论、12 条楼中楼，81 次请求，主评论数与接口 total 一致。添加监测未重复全量，立即增量以 3 次请求（详情与 2 个主评论页）完成基线复用。仅保留不含正文或用户资料的元数据证据，临时归档和配置已清理。

- 本地包：`target/comment_collector-0.1.3-windows-x86_64.wplug`
- 字节数：`1468192`
- SHA-256：`4e7a3ce90831e799ca54e76a1c61f937a86164ec0a94fc9c95d62dff8f32ceb4`

GitHub Actions 会重新构建，线上包的字节数和 SHA-256 以正式签名清单为准，不能直接假定与本地 ZIP 相同。

## 发布后核验

标签 workflow 成功后，下载线上 .wplug 和 `comment_collector-update.json`，用 Catalog 当前公钥验证 Ed25519 签名、包大小与 SHA-256、安装包内部 checksums；确认 stable latest 地址指向 0.1.3。普通版本更新不修改 Catalog 身份或公钥。

真实 Core GUI 安装/更新、长期监测、远程会话及休眠/唤醒仍未验收。
