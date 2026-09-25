# 测试样本

字段结构根据 2026-09-19 对米游社匿名社区接口的**实测**核对（`level/detail` 与 `reply/list` 的响应键逐一对齐），
并与旧版 `modules/comments` 的可用实现交叉验证。关卡名、昵称、UID、头像与内容均为虚构，不含真实用户内容。

各文件用途：

- `detail.json`：`level/detail` 的响应外壳。字段名与实测一致（`show_limit_play_num_str` 是**人数范围** `1-8`，不是游玩人次）；`images` 里留一条空 URL 验证会被滤掉。
- `page1.json`：`reply/list` 第一页。含两条主评论与一条楼中楼，`like_count` 用字符串、`created_at` 用字符串，覆盖官方类型混用；楼中楼带自己的子楼层序号（`floor_id: 1`）与 `r_user: null`，与实测一致；游标 `has_more` 为真。
- `page2.json`：末页。含一条带逗号、引号与换行的内容（验证 CSV 转义）与一条 `reply_id` 为空的脏数据（验证会被跳过）；游标 `has_more` 为假。
- `business.json`：`retcode = -2000431`「关卡不存在」，验证官方业务错误带上文案而不是被当成结构变化。

实测结论（2026-09-19，两个关卡共 2029 条主评论 + 69 条楼中楼）已写进 `src/model.rs` 的字段注释：
`is_owner` 恒为 `false`、楼中楼的 `is_recommend` 恒为 `false`（故归档里对楼中楼留空）、`r_user` 恒为 `null`、
楼中楼不会嵌套更深一层。这些是采样结论而非接口保证，接口若变化需重新实机采样核对。
