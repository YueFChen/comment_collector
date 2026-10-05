# Core 0.1.8 兼容说明

本插件版本为 **0.1.2**，最低 Core 为 **0.1.8**。构建前运行 `node scripts/check-core-compatibility.mjs`；构建脚本会自动检查此条件。旧 Core 0.1.7 不识别本次 manifest 扩展，不能安装本次新包。

远程访问：**已声明**。

插件只在宿主机运行。Core 根据请求上下文调度远程文件导入、下载和系统交互，本地调用维持原路径。共享前需在 Core 设置中主动勾选；未勾选时，远程设备无权访问。远程用户可调用插件全部已声明方法，应只向可信设备共享。

窄屏增量样式只用于 Core 标记的 WebUI 页面，本机界面保持现状。插件页面中的设备标记仅用于布局，访问权限由 Core 校验，不能靠修改网址获得授权。

`backend.supportsServiceContext: true` 表示后端会原样回传每次调用的上下文。SDK 插件使用每次 dispatch 收到的 `HostClient`；不要缓存一个客户端跨不同请求处理文件操作。未声明远程支持的插件不会出现在共享列表，Core 也会拒绝直接远程调用。

发布顺序为先发布配套 Core，再按插件仓库的检查、签名和发布流程发布插件。已登记插件的普通升级不需要修改 Catalog 身份记录。

## 后台监测的上下文

监测在 `serve_with_startup` 的握手回调中获得稳定 HostClient，仅用于 `core.network.public` 匿名请求。它不缓存任意 dispatch 的上下文，也不调用文件导出/目录显示服务。所有 `export`、`export_dir`、`reveal_dir` 仍使用该次 dispatch 自己收到的 HostClient，保留 Core 的远程会话路由。

监测配置是宿主机上该插件的持久配置。获准访问插件的可信远程用户也可以改变它；关闭远程页面不会撤销已经启用的宿主机监测，应在面板显式关闭。实际 Core GUI/远程会话仍需在 Windows 目标环境验收。
