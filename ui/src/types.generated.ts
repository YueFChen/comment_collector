// Generated from Rust by examples/generate_bindings.rs. Do not edit.
export type CollectionMode = "full" | "incremental";
export type MonitorConfig = { enabled: boolean, level_ids: Array<string>, incremental_interval_secs: number, full_interval_secs: number, max_parallel: number, request_interval_ms: number, overlap_pages: number, incremental_max_pages: number, };
export type MonitorLevelStatus = { level_id: string, running: boolean, last_attempt_at: number, last_success_at: number, last_full_success_at: number, consecutive_failures: number, last_error: string, last_mode: CollectionMode | null,
/**
 * A paused full traversal must finish before returning to incremental scans.
 */
needs_full_recovery: boolean,
/**
 * Computed from the current schedule; null while paused or running.
 */
next_collect_at: number | null, };
export type MonitorStatus = { config: MonitorConfig, levels: Array<MonitorLevelStatus>, };
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
export type NewCommentCounts = { recommended: number, not_recommended: number, };
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
 * 最近一次采集的完成状态；v1 归档读取时默认为 unknown。
 */
collection_state: CollectionState,
/**
 * 最近一次未完成采集的原因。
 */
collection_message: string,
/**
 * 旧归档无最近新增统计，读取时不从历史数据推算。
 */
last_new_counts: NewCommentCounts | null,
/**
 * 主评论与楼中楼按采集顺序展开存放。
 */
comments: Array<CommentItem>, };
export type CollectionState = "unknown" | "complete" | "partial";
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
count: number, collection_state: CollectionState,
/**
 * 文件存在但不能作为归档读取，需要重新采集。
 */
needs_recollect: boolean, last_new_counts: NewCommentCounts | null, };
export type FavoriteLevel = { level_id: string, level_name: string, cover_url: string,
/**
 * 加入本地收藏的时间（Unix 秒）。
 */
added_at: number, };
export type ArchiveOverview = { level: LevelInfo, updated_at: number, fetch_count: number, last_pages: number, collection_state: CollectionState, collection_message: string, comment_count: number, };
export type CommentGroup = { main: CommentItem, subs: Array<CommentItem>, };
export type ArchiveFilter = "all" | "recommend" | "notRecommend" | "owner";
export type ArchiveSort = "default" | "newest" | "oldest" | "likes" | "floor";
export type ArchiveViewQuery = { level_id: string, offset: number, limit: number, filter: ArchiveFilter, sort: ArchiveSort, keyword: string, };
export type ArchiveViewPage = { overview: ArchiveOverview, total_groups: number, offset: number, groups: Array<CommentGroup>, };
export type CommentArchiveServiceSummary = { level_id: string, level_name: string, updated_at: number, count: number, };
export type CommentArchiveServiceItem = { floor_id: string, nickname: string, content: string, is_recommend: boolean | null, like_count: number, reply_count: number, created_at: number, is_sub: boolean, reply_to: string, };
export type CommentArchivePage = { level_id: string, level_name: string, updated_at: number, total_count: number, offset: number, comments: Array<CommentArchiveServiceItem>, };
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
