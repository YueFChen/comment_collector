//! 评论采集器的对外 DTO。
//!
//! 这些类型同时是本地归档格式与前端绑定来源，字段增删视为对外契约变更。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 本地归档的 schema 版本。
///
/// 改动归档里的字段时必须同时把它加一：旧归档会被**明确拒绝**并要求重新采集，
/// 而不是解析出半份数据（读取端先看版本、再看结构，见 `CommentCollector::archive`）。
pub const ARCHIVE_VERSION: u32 = 1;

/// 一次采集的输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct CommentQuery {
    /// 关卡 id（官方为纯数字）。
    pub level_id: String,
}

/// 关卡元信息，取自官方 `level/detail`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct LevelInfo {
    pub level_id: String,
    pub level_name: String,
    pub cover_url: String,
    /// 热度分。官方以字符串下发（实测形如 `872`）。
    pub hot_score: String,
    /// 好评率，官方形如 `96.7%`。
    pub good_rate: String,
    pub play_type: String,
    /// 可游玩人数范围，官方形如 `1-8`。
    pub play_range: String,
    pub desc: String,
    /// 关卡图集（含封面之外的截图）。
    pub images: Vec<String>,
}

/// 一条评论；主评论与楼中楼共用同一形状，用 [`CommentItem::is_sub`] 区分。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct CommentItem {
    pub reply_id: String,
    /// 楼层号。楼中楼另有一套子楼层序号（实测主评论 204 楼下的回复是 1、2 楼），
    /// 与主楼层不同源，放同一列会让人误读，故楼中楼统一留空。
    pub floor_id: String,
    pub uid: String,
    pub nickname: String,
    pub avatar_url: String,
    pub content: String,
    /// 是否推荐。官方对主评论与楼中楼都下发该字段，但「推荐 / 不推荐」只对主评论有意义：
    /// 实测楼中楼恒为 `false`，照实展示等于给每条回复都盖上「不推荐」，故楼中楼留空。
    pub is_recommend: Option<bool>,
    /// 是否关卡作者本人发表。官方下发该字段；实测两个关卡的全部样本均为 `false`。
    pub is_owner: bool,
    pub like_count: u32,
    pub reply_count: u32,
    /// 发表时间（Unix 秒）。
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub created_at: i64,
    /// 发表时的 IP 归属地，形如 `浙江`。
    pub ip_region: String,
    /// 是否为楼中楼回复。
    pub is_sub: bool,
    /// 楼中楼所属的主评论 id；主评论为空串。
    pub parent_id: String,
    /// 楼中楼的回复对象昵称。官方只在「回复楼中楼」时下发 `r_user`（此时 `f_reply_id` 非 0），
    /// 实测样本中全部是对主评论的直接回复，故恒为空。
    pub reply_to: String,
}

/// 逐个关卡的长期归档。
///
/// 官方只给滚动窗口，所以每次采集都并入同一份归档并按 `reply_id` 去重：
/// 已经滑出窗口的评论仍留在本地，重复采集不会产生副本。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct CommentArchive {
    pub schema_version: u32,
    pub level: LevelInfo,
    /// 最近一次采集时间（Unix 秒）。
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub updated_at: u64,
    /// 累计采集次数。
    pub fetch_count: u32,
    /// 最近一次采集翻过的页数。
    pub last_pages: u32,
    /// 主评论与楼中楼按采集顺序展开存放。
    pub comments: Vec<CommentItem>,
}

/// 归档清单项：给界面列出「采过哪些关卡」用。
///
/// 只用于展示，不落盘——归档文件里存的是 [`CommentArchive`]，
/// 因此这里的字段增删不影响 `ARCHIVE_VERSION`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ArchiveSummary {
    pub level_id: String,
    pub level_name: String,
    /// 关卡封面（官方统一 16:9）；列表卡片用它做背景图。
    pub cover_url: String,
    /// 最近一次采集时间（Unix 秒）。
    #[cfg_attr(feature = "bindings", ts(type = "number"))]
    pub updated_at: u64,
    /// 归档内评论总条数（含楼中楼）。
    pub count: u32,
}

/// 采集进度，经宿主事件推给界面。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct CollectProgress {
    /// 已完成的页数。
    pub page: u32,
    /// 已累计抓到的评论条数（含楼中楼）。
    pub fetched: u32,
}

/// 导出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Json,
    Csv,
    Excel,
}

/// 导出结果：把落地路径回给界面显示。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS))]
pub struct ExportOutcome {
    /// 已写出文件的完整路径。
    pub path: String,
    pub format: ExportFormat,
    /// 写出的数据行数（不含表头）。
    pub rows: u32,
}

impl CommentArchive {
    pub fn empty(level: LevelInfo) -> Self {
        Self {
            schema_version: ARCHIVE_VERSION,
            level,
            updated_at: 0,
            fetch_count: 0,
            last_pages: 0,
            comments: Vec::new(),
        }
    }

    /// 只把一批评论并进归档：按 `reply_id` 去重，较新的数据覆盖旧的。
    ///
    /// 与 [`Self::merge`] 分开是为了**中途落盘**：检查点只并数据，
    /// 「累计采集次数」要等整轮跑完才记，否则一次采集会被算成好几次。
    pub fn merge_comments(&mut self, incoming: Vec<CommentItem>) {
        let mut positions: HashMap<String, usize> = self
            .comments
            .iter()
            .enumerate()
            .map(|(index, item)| (item.reply_id.clone(), index))
            .collect();
        for item in incoming {
            match positions.get(&item.reply_id) {
                Some(&index) => self.comments[index] = item,
                None => {
                    positions.insert(item.reply_id.clone(), self.comments.len());
                    self.comments.push(item);
                }
            }
        }
    }

    /// 把一次**完整**采集并入归档：除数据外还记下次数、页数与时间。
    pub fn merge(
        &mut self,
        level: LevelInfo,
        fetched_at: u64,
        pages: u32,
        incoming: Vec<CommentItem>,
    ) {
        self.schema_version = ARCHIVE_VERSION;
        self.level = level;
        self.updated_at = fetched_at;
        self.fetch_count += 1;
        self.last_pages = pages;
        self.merge_comments(incoming);
    }

    /// 主评论条数。
    pub fn main_count(&self) -> u32 {
        self.comments.iter().filter(|item| !item.is_sub).count() as u32
    }
}
