# 发布与目录登记

插件发布使用 GitHub Release，并附带 `.wplug` 安装包和 Ed25519 签名的 `comment_collector-update.json`。在线目录只保存插件身份、稳定更新清单地址和签名公钥；目录 CI 会下载最新 Release 并验证签名和安装包。

## 版本与构建基线

- 插件版本在 Rust workspace、根 `package.json`、`ui/package.json` 和 `package/manifest.json` 中保持一致；当前版本为 `0.1.3`。
- 同一轮本地开发、修正和重新打包保持当前版本号；准备推送版本标签并发布 GitHub Release 时再统一确定发布版本，不为每次本地修改连续递增。
- 仓库已配置签名发布；普通版本更新沿用现有 `PLUGIN_UPDATE_SIGNING_KEY` 和 `PLUGIN_UPDATE_SIGNING_PUBLIC_KEY`，不重新生成密钥。
- 工作流构建时固定检出 Core 提交 `dc988400c4d66ec1c25a5221583efaf80c80db0b`，该提交提供插件 SDK 0.1.8。
- 发布标签必须是 `v<manifest.version>`，例如 `v0.1.0`。

## 首次发布

1. 将功能改动和发布脚本合入插件仓库 `main`，确认检查通过，并确保 GitHub 仓库有权创建 Release。
2. 经确认要配置发布签名后，运行 `node scripts/generate-update-signing-key.mjs --register`。脚本会拒绝覆盖已有的同名密钥，只把私钥种子通过标准输入写入 `YueFChen/comment_collector` 的 Actions secret `PLUGIN_UPDATE_SIGNING_KEY`，并写入对应的公开 Actions variable `PLUGIN_UPDATE_SIGNING_PUBLIC_KEY`。私钥不会输出到终端或写入文件。脚本会输出可用于目录登记的 v2 JSON 记录。
3. 从合并到 `main` 的提交创建并推送匹配版本的标签，例如 `v0.1.0`。`Plugin release` 工作流会运行 UI 检查、Rust 格式检查、Clippy、Rust 测试，构建 `.wplug`，生成签名更新清单，再创建 GitHub Release。
4. **Release 成功后**，把上一步输出的身份记录保存为目录仓库 `catalog/v2/plugins/comment_collector.json`，并在 `YueFChen/Wonderland_Plugin_Catalog` 创建登记 PR。记录中的 `signingPublicKey` 必须与 Actions variable 和签名私钥对应。
5. 等待目录 PR 的 `validate` 检查下载并验证最新签名清单和 `.wplug`，再由有权限的维护者审核合并。目录 PR 校验依赖已经公开的 Release，因此首次登记不能先于插件 Release。

更新版本时，先同步修改四处版本号并确认标签不存在，再合入代码、创建 `v<新版本>` 标签。不要在未安排 Catalog 公钥轮换的情况下替换签名密钥。

如果标签工作流在创建 Release 前因 CI 配置失败，先修复并合入工作流，再从 `main` 手动运行 `Plugin release`，传入原有 `release_tag`。手动恢复会检出该标签对应的插件源码，不移动标签；先确认该标签还没有 Release。若源码本身需要修正，则按新版本发布，不复用已推送的标签。

## 本地构建

在 Windows x86_64、Node 24 和 Rust 1.98.1 环境中，从插件目录执行：

```powershell
pnpm install --frozen-lockfile
pnpm run check
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
cargo run --locked --features bindings --example generate_bindings -- --check
pnpm run build:release
node scripts/test-monitor-protocol.mjs target/comment-collector-release-package/backend/wonderland-comment-collector.exe
node scripts/test-monitor-baseline-protocol.mjs target/comment-collector-release-package/backend/wonderland-comment-collector.exe
```

`.wplug` 输出到 `target/comment_collector-<version>-windows-x86_64.wplug`。生成签名更新清单还需要已配置的 Actions secret 与同一密钥的公开 Actions variable；本地不要把私钥写入插件仓库。

每轮发布前维护 [RELEASE-NOTES.md](RELEASE-NOTES.md)，描述相对最新 GitHub Release 的最终改动。标签工作流读取该文件作为 Release 正文，并在签名前检查生成类型绑定与包内后端协议。本轮发布准备记录见 [docs/RELEASE-PREPARATION.md](docs/RELEASE-PREPARATION.md)。
