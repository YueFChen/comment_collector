// Generated from Rust by examples/generate_bindings.rs. Do not edit.
export type CommentQuery = {
/**
 * 关卡 id（官方为纯数字）。
 */
level_id: string, };
export type LevelInfo = { level_id: string, level_name: string, cover_url: string,
/**
 * 热度分。官方以字符串下发（实测形如 `872`）。
 */
hot_score: string,
/**
 * 好评率，官方形如 `96.7%`。
 */
good_rate: string, play_type: string,
/**
 * 可游玩人数范围，官方形如 `1-8`。
 */
play_range: string, desc: string,
/**
 * 关卡图集（含封面之外的截图）。
 */
images: Array<string>, };
export type CommentItem = { reply_id: string,
/**
 * 楼层号。楼中楼另有一套子楼层序号（实测主评论 204 楼下的回复是 1、2 楼），
 * 与主楼层不同源，放同一列会让人误读，故楼中楼统一留空。
 */
floor_id: string, uid: string, nickname: string, avatar_url: string, content: string,
/**
 * 是否推荐。官方对主评论与楼中楼都下发该字段，但「推荐 / 不推荐」只对主评论有意义：
 * 实测楼中楼恒为 `false`，照实展示等于给每条回复都盖上「不推荐」，故楼中楼留空。
 */
is_recommend: boolean | null,
/**
 * 是否关卡作者本人发表。官方下发该字段；实测两个关卡的全部样本均为 `false`。
 */
is_owner: boolean, like_count: number, reply_count: number,
/**
 * 发表时间（Unix 秒）。
 */
created_at: number,
/**
 * 发表时的 IP 归属地，形如 `浙江`。
 */
ip_region: string,
/**
 * 是否为楼中楼回复。
 */
is_sub: boolean,
/**
 * 楼中楼所属的主评论 id；主评论为空串。
 */
parent_id: string,
/**
 * 楼中楼的回复对象昵称。官方只在「回复楼中楼」时下发 `r_user`（此时 `f_reply_id` 非 0），
 * 实测样本中全部是对主评论的直接回复，故恒为空。
 */
reply_to: string, };
export type CommentArchive = { schema_version: number, level: LevelInfo,
/**
 * 最近一次采集时间（Unix 秒）。
 */
updated_at: number,
/**
 * 累计采集次数。
 */
fetch_count: number,
/**
 * 最近一次采集翻过的页数。
 */
last_pages: number,
/**
 * 主评论与楼中楼按采集顺序展开存放。
 */
comments: Array<CommentItem>, };
export type ArchiveSummary = { level_id: string, level_name: string,
/**
 * 关卡封面（官方统一 16:9）；列表卡片用它做背景图。
 */
cover_url: string,
/**
 * 最近一次采集时间（Unix 秒）。
 */
updated_at: number,
/**
 * 归档内评论总条数（含楼中楼）。
 */
count: number, };
export type CollectProgress = {
/**
 * 已完成的页数。
 */
page: number,
/**
 * 已累计抓到的评论条数（含楼中楼）。
 */
fetched: number, };
export type ExportFormat = "json" | "csv" | "excel";
export type ExportOutcome = {
/**
 * 已写出文件的完整路径。
 */
path: string, format: ExportFormat,
/**
 * 写出的数据行数（不含表头）。
 */
rows: number, };
