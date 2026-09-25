//! 评论采集器插件。
//!
//! 走米游社匿名社区接口抓单个奇域关卡的全量评论，本地长期累积，并可导出
//! CSV / JSON / Excel。**全程匿名**，不需要账号；出站请求通过 Core 的受控公共网络服务。

mod bbs;
mod export;
pub mod model;
#[cfg(test)]
mod tests;

pub use model::*;

use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bbs::{Bbs, MAX_PAGES};
use tokio::sync::Mutex;
use tracing::{debug, info, instrument, warn};
use wonderland_plugin_sdk::PluginFailure;
pub use bbs::PublicHttpClient;

/// 翻页间隔。
///
/// 实测（2026-09-19，关卡 7257762194，73 页 / 1468 条）：单页请求本身约 114ms，
/// 而官方把单页上限**硬卡在 20 条**——`size` 给 5 / 10 会被尊重，给 50 / 100 也只回 20。
/// 所以加速唯有压缩这个间隔：400ms 时间隔占了单页总耗时的七成以上。
///
/// 端到端 A/B（同一关卡、同一时刻）：400ms → 37.4s，120ms → 17.1s，约 2.2 倍。
/// 不取实测上限：无间隔连发 20 次、按 150ms 连翻 20 页都没被限流，但匿名接口被封没有申诉渠道，
/// 留一倍余量按约 4 次/秒走。
const PAGE_DELAY: Duration = Duration::from_millis(120);

/// 检查点间隔：每抓这么多页就把已抓到的评论落一次盘。
///
/// 链式游标跨会话不可用（下一页游标只存在于上一页的响应里），所以检查点**省不下翻页**——
/// 它保的是**已经拿到的数据**：中途失败、取消或应用被关掉，都不必从头再抓一遍。
const CHECKPOINT_PAGES: u32 = 10;
/// 单页失败的重试次数（不含首次请求）。
const PAGE_RETRIES: u32 = 2;
/// 重试退避基数：第 n 次重试等 `RETRY_BACKOFF * 3^n`，即 300ms、900ms。
const RETRY_BACKOFF: Duration = Duration::from_millis(300);

/// 只有网络类失败值得重试。
///
/// 口径与内核 GET 的有限重试一致。业务错误（关卡不存在、接口返回结构变了）
/// 重试多少次都是同一个答案，重试只会白等。
fn retryable(error: &PluginFailure) -> bool {
    matches!(
        error,
        PluginFailure::Timeout
            | PluginFailure::Connection
            | PluginFailure::Http(429 | 500 | 502 | 503 | 504)
    )
}

/// 采集已取消。
///
/// 取消是用户主动行为、不是故障：整轮不再继续，也**不记这次的采集次数**。
/// 但已经落过检查点的那部分评论留在归档里（见 `collect`），取消不销毁已抓到的数据。
fn cancelled() -> PluginFailure {
    PluginFailure::Other("采集已取消".to_owned())
}

/// 底层 IO 原因带进 `details`，否则只剩一句"读写失败"，排查时无从下手。
fn storage(error: impl std::fmt::Display) -> PluginFailure {
    warn!(reason = %error, "本地归档读写失败");
    PluginFailure::Archive(error.to_string())
}

/// 关卡 id 要落进文件名，只接受官方使用的纯数字。
fn valid_level_id(level_id: &str) -> bool {
    !level_id.is_empty()
        && level_id.len() <= 20
        && level_id.bytes().all(|byte| byte.is_ascii_digit())
}

/// 文件的修改时间（Unix 秒）。取不到就按 0 处理，排序时自然落到末尾。
fn modified_at(path: &std::path::Path) -> u64 {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

pub struct CommentCollector {
    root: PathBuf,
    bbs: Bbs,
    /// 单插件串行采集，避免并发写同一份归档；不阻塞读取与导出。
    collecting: Mutex<()>,
    /// 取消标记：由宿主置位，采集循环在翻页间隙检查。
    cancel: Arc<AtomicBool>,
}

impl CommentCollector {
    pub fn new(root: PathBuf, http: Arc<dyn PublicHttpClient>) -> Result<Self, PluginFailure> {
        fs::create_dir_all(&root).map_err(storage)?;
        Ok(Self {
            root,
            bbs: Bbs::new(http),
            collecting: Mutex::new(()),
            cancel: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Produce an export payload for the controlled Core file service.
    pub fn export_payload(&self, level_id: &str, format: ExportFormat) -> Result<ExportPayload, PluginFailure> {
        let archive = self.archive(level_id)?
            .ok_or_else(|| PluginFailure::LocalData("尚未采集过该关卡".into()))?;
        let (extension, content) = match format {
            ExportFormat::Json => ("json", export::json(&archive)?),
            ExportFormat::Csv => ("csv", export::csv(&archive)),
            ExportFormat::Excel => ("xlsx", export::excel(&archive)?),
        };
        Ok(ExportPayload {
            name: export::file_name(&archive, extension),
            content,
            format,
            rows: archive.comments.len() as u32,
        })
    }
    /// 请求取消正在进行的采集；下一次翻页前生效。
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// 取一页；网络类失败按指数退避重试，仍失败就把最后一次的错误抛出去。
    ///
    /// `attempts` 是出参，回填**实际发出的请求次数**：重试只发生在这里，所以只有这一层
    /// 知道次数（Core 公共网络 POST 路径不重试也不落日志）。调用方放弃时用它写日志，
    /// 失败链上才有「试了几次」这个数。
    async fn page_with_retry(
        &self,
        level_id: &str,
        cursor: Option<&serde_json::Value>,
        attempts: &mut u32,
    ) -> Result<bbs::ReplyListData, PluginFailure> {
        let mut retries = 0;
        loop {
            *attempts += 1;
            match self.bbs.page(level_id, cursor).await {
                Ok(page) => return Ok(page),
                Err(error) if retryable(&error) && retries < PAGE_RETRIES => {
                    debug!(
                        code = error.code(),
                        attempt = *attempts,
                        "翻页失败，退避后重试"
                    );
                    tokio::time::sleep(RETRY_BACKOFF * 3u32.pow(retries)).await;
                    retries += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// 中途落盘：把已抓到的评论并进归档，并推进「最近一次采集时间」。
    ///
    /// **不动 `fetch_count`**：一次采集不管中途落盘几次，界面上都只算一次。
    fn checkpoint(
        &self,
        archive: &mut CommentArchive,
        incoming: Vec<CommentItem>,
        started_at: u64,
    ) -> Result<(), PluginFailure> {
        if incoming.is_empty() {
            return Ok(());
        }
        archive.merge_comments(incoming);
        archive.updated_at = started_at;
        self.save(archive)
    }

    fn archive_path(&self, level_id: &str) -> Result<PathBuf, PluginFailure> {
        if !valid_level_id(level_id) {
            return Err(PluginFailure::InvalidInput);
        }
        Ok(self.root.join(format!("{level_id}.json")))
    }

    /// 读取本地归档（不触发采集）；从未采集过该关卡时为 `None`。
    ///
    /// 这些失败都出在**本地文件**上，不能借用 `InvalidResponse`——
    /// 那条错误的文案说的是"官方接口数据结构已变化"，会把用户指向错误的方向。
    pub fn archive(&self, level_id: &str) -> Result<Option<CommentArchive>, PluginFailure> {
        let path = self.archive_path(level_id)?;
        if !path.exists() {
            return Ok(None);
        }
        // 先用 Value 取出版本、再解析整体：结构变化时旧归档会解析失败，
        // 若直接解析就永远走不到版本判断，只剩一句无从下手的"无法解析"。
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).map_err(storage)?)
            .map_err(|_| {
            warn!(level_id, "本地归档不是有效的 JSON");
            PluginFailure::LocalData("本地归档不是有效的 JSON，请重新采集".into())
        })?;
        if value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            != Some(u64::from(ARCHIVE_VERSION))
        {
            warn!(level_id, "本地归档版本不匹配");
            return Err(PluginFailure::LocalData(
                "本地归档版本不匹配，请重新采集".into(),
            ));
        }
        let archive: CommentArchive = serde_json::from_value(value).map_err(|_| {
            warn!(level_id, "本地归档结构已变化");
            PluginFailure::LocalData("本地归档结构已变化，请重新采集".into())
        })?;
        if !archive.level.level_id.is_empty() && archive.level.level_id != level_id {
            warn!(level_id, "本地归档所属关卡与文件名不一致");
            return Err(PluginFailure::LocalData(
                "本地归档所属关卡与文件名不一致，请重新采集".into(),
            ));
        }
        Ok(Some(archive))
    }

    fn save(&self, archive: &CommentArchive) -> Result<(), PluginFailure> {
        let path = self.archive_path(&archive.level.level_id)?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(storage)?;
        }
        let bytes = serde_json::to_vec(archive).map_err(storage)?;
        // 半途失败不破坏上一份归档：先写临时文件再原子替换。
        let temp = path.with_extension("tmp");
        fs::write(&temp, bytes).map_err(storage)?;
        fs::rename(temp, path).map_err(storage)
    }

    /// 已采集的关卡清单，按最近采集时间倒序。
    ///
    /// 读不出来的归档**不静默跳过**：跳过了界面就会显示"还没采过"，
    /// 而用户其实有归档、只是需要重新采集。所以退一步用文件名与文件时间占位，
    /// 让条目仍然可见可点——点进去 [`Self::archive`] 会给出"版本不匹配 / 结构已变化"的确切原因。
    pub fn archives(&self) -> Result<Vec<ArchiveSummary>, PluginFailure> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(storage)? {
            let path = entry.map_err(storage)?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            if let Ok(archive) = serde_json::from_slice::<CommentArchive>(&bytes) {
                out.push(ArchiveSummary {
                    level_id: archive.level.level_id,
                    level_name: archive.level.level_name,
                    cover_url: archive.level.cover_url,
                    updated_at: archive.updated_at,
                    count: archive.comments.len() as u32,
                });
                continue;
            }
            // 占位只给"确实是 JSON、但结构/版本对不上"的文件留：
            // 真损坏的文件没有任何可展示的信息，列出来只会变成一张点不开的谜之卡片。
            if serde_json::from_slice::<serde_json::Value>(&bytes).is_err() {
                continue;
            }
            out.push(ArchiveSummary {
                // 内容不可信时文件名是唯一可靠的关卡 id。
                level_id: path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or_default()
                    .to_owned(),
                level_name: String::new(),
                cover_url: String::new(),
                updated_at: modified_at(&path),
                count: 0,
            });
        }
        out.sort_by_key(|item| std::cmp::Reverse(item.updated_at));
        Ok(out)
    }

    /// 采集一次并并入本地归档，返回合并后的归档。
    ///
    /// 官方只给滚动窗口，所以这里不做"逐次快照"，而是把评论按 `reply_id` 并进已有归档：
    /// 已经滑出窗口的评论仍留在本地，重复采集也不会产生副本。
    ///
    /// 每 [`CHECKPOINT_PAGES`] 页落一次盘，取消与失败时也会把已抓到的页并进去——
    /// 官方接口抖一下不该毁掉几十页的成果。
    #[instrument(
        skip(self, query, progress),
        fields(plugin = "comment_collector", level_id = %query.level_id)
    )]
    pub async fn collect<F: FnMut(CollectProgress) + Send>(
        &self,
        query: CommentQuery,
        mut progress: F,
    ) -> Result<CommentArchive, PluginFailure> {
        let _guard = self.collecting.try_lock().map_err(|_| PluginFailure::Busy)?;
        if !valid_level_id(&query.level_id) {
            return Err(PluginFailure::InvalidInput);
        }
        self.cancel.store(false, Ordering::SeqCst);
        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(storage)?
            .as_secs();
        let mut level = self.bbs.level(&query.level_id).await?;
        if level.level_id.is_empty() {
            level.level_id = query.level_id.clone();
        }
        let mut archive = self
            .archive(&query.level_id)?
            .unwrap_or_else(|| CommentArchive::empty(level.clone()));
        info!("开始采集评论");

        let mut cursor: Option<serde_json::Value> = None;
        let mut pages = 0u32;
        let mut fetched = 0u32;
        // 是否撞上页数上限而停（而不是"官方说没有更多了"）。
        let mut truncated = false;
        // 距上次落盘新抓到的评论；检查点与收尾都从这里取。
        let mut pending = Vec::new();
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                let _ = self.checkpoint(&mut archive, std::mem::take(&mut pending), started_at);
                info!("采集已取消");
                return Err(cancelled());
            }
            let mut attempts = 0u32;
            let page = match self
                .page_with_retry(&query.level_id, cursor.as_ref(), &mut attempts)
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    // 尽力保住已抓到的页；落盘本身失败也不该盖掉真正的失败原因。
                    let _ = self.checkpoint(&mut archive, std::mem::take(&mut pending), started_at);
                    warn!(
                        code = error.code(),
                        page = pages,
                        attempts,
                        "翻页失败，采集中止"
                    );
                    return Err(error);
                }
            };
            pages += 1;
            let items = page.items();
            fetched += items.len() as u32;
            pending.extend(items);
            progress(CollectProgress {
                page: pages,
                fetched,
            });
            debug!(page = pages, fetched, "抓取一页评论");
            // `has_more` 为真但游标空转时必须停下，否则会反复翻同一页。
            if !page.more() || page.next().is_empty() {
                break;
            }
            // 页数上限只是官方 `has_more` 异常时的兜底。撞上它属于异常，
            // 不能和"官方说抓完了"混成同一个出口，否则截断了也没人知道。
            if pages >= MAX_PAGES {
                truncated = true;
                break;
            }
            cursor = page.cursor().cloned();
            if pages.is_multiple_of(CHECKPOINT_PAGES) {
                self.checkpoint(&mut archive, std::mem::take(&mut pending), started_at)
                    .map_err(|error| {
                        warn!(reason = %error, "检查点落盘失败");
                        error
                    })?;
            }
            tokio::time::sleep(PAGE_DELAY).await;
        }
        archive.merge(level, started_at, pages, pending);
        self.save(&archive)?;
        if truncated {
            // 数据已经落盘（上面 save 过了），这里只负责说清楚"可能没抓完"——
            // 把撞上限当成一次正常完成会静默吞掉遗漏。
            warn!(pages, "翻到页数上限，可能有遗漏");
            return Err(PluginFailure::Other(format!(
                "翻到 {MAX_PAGES} 页上限仍未结束，可能有遗漏；已抓到的部分已存入归档"
            )));
        }
        info!(pages, fetched, total = archive.comments.len(), "采集完成");
        Ok(archive)
    }

}

#[derive(Debug, Clone)]
pub struct ExportPayload {
    pub name: String,
    pub content: Vec<u8>,
    pub format: ExportFormat,
    pub rows: u32,
}
