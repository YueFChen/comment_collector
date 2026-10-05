//! 评论采集器插件。
//!
//! 走米游社匿名社区接口抓单个奇域关卡的全量评论，本地长期累积，并可导出
//! CSV / JSON / Excel。**全程匿名**，不需要账号；出站请求通过 Core 的受控公共网络服务。

mod bbs;
mod export;
pub mod model;
mod monitor;
#[cfg(test)]
mod monitor_tests;
mod scan;
#[cfg(test)]
mod tests;

pub use model::*;
pub use monitor::{CollectionMode, MonitorConfig, MonitorLevelStatus, MonitorStatus};

use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bbs::Bbs;
pub use bbs::PublicHttpClient;
use tokio::sync::Mutex;
use tracing::{instrument, warn};
use wonderland_plugin_sdk::PluginFailure;

/// 检查点间隔：每抓这么多页就把已抓到的评论落一次盘。
///
/// 链式游标跨会话不可用（下一页游标只存在于上一页的响应里），所以检查点**省不下翻页**——
/// 它保的是**已经拿到的数据**：中途失败、取消或应用被关掉时不丢已有内容；下次仍从首页安全重扫。
const CHECKPOINT_PAGES: u32 = 10;
/// 单页失败的重试次数（不含首次请求）。
const PAGE_RETRIES: u32 = 2;
/// 重试退避基数：第 n 次重试等 `RETRY_BACKOFF * 3^n`，即 300ms、900ms。
const RETRY_BACKOFF: Duration = Duration::from_millis(300);

/// Core 的 `core.files.export` 接口单文件上限。先在插件侧检查，给出可读错误。
const CORE_EXPORT_MAX_BYTES: usize = 20 * 1024 * 1024;
/// 单个插件最多保留的本地收藏关卡数。
const MAX_FAVORITES: usize = 2_000;
const FAVORITES_SCHEMA_VERSION: u32 = 1;

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
    /// Only foreground requests share a cancellation flag. Background jobs have their own tokens.
    collecting: Mutex<()>,
    level_locks: StdMutex<HashMap<String, Arc<Mutex<()>>>>,
    monitor_config_lock: StdMutex<()>,
    monitor: StdMutex<monitor::MonitorRuntime>,
    monitor_started: AtomicBool,
    active_jobs: AtomicU32,
    /// 取消标记：由宿主置位，采集循环在翻页间隙检查。
    cancel: Arc<AtomicBool>,
    /// 当前浏览关卡的解析结果，避免每次翻页重新读取整份 JSON。
    view_cache: StdMutex<Option<CachedArchive>>,
    /// 串行化收藏列表的读改写，避免并发切换时丢失条目。
    favorites_lock: StdMutex<()>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct FavoritesFile {
    schema_version: u32,
    items: Vec<FavoriteLevel>,
}

struct CachedArchive {
    level_id: String,
    modified: Option<SystemTime>,
    length: u64,
    archive: Arc<CommentArchive>,
}

struct GroupIndices {
    main: Option<usize>,
    subs: Vec<usize>,
}

fn comment_matches(item: &CommentItem, keyword: &str) -> bool {
    item.nickname.to_lowercase().contains(keyword)
        || item.uid.contains(keyword)
        || item.ip_region.to_lowercase().contains(keyword)
        || item.content.to_lowercase().contains(keyword)
}

fn floor_rank(value: &str) -> i64 {
    let digits: String = value
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().unwrap_or(i64::MAX)
}

impl CommentCollector {
    pub fn new(root: PathBuf, http: Arc<dyn PublicHttpClient>) -> Result<Self, PluginFailure> {
        fs::create_dir_all(&root).map_err(storage)?;
        Ok(Self {
            root,
            bbs: Bbs::new(http),
            collecting: Mutex::new(()),
            level_locks: StdMutex::new(HashMap::new()),
            monitor_config_lock: StdMutex::new(()),
            monitor: StdMutex::new(monitor::MonitorRuntime::default()),
            monitor_started: AtomicBool::new(false),
            active_jobs: AtomicU32::new(0),
            cancel: Arc::new(AtomicBool::new(false)),
            view_cache: StdMutex::new(None),
            favorites_lock: StdMutex::new(()),
        })
    }

    /// Produce an export payload for the controlled Core file service.
    pub fn export_payload(
        &self,
        level_id: &str,
        format: ExportFormat,
    ) -> Result<ExportPayload, PluginFailure> {
        let archive = self
            .archive(level_id)?
            .ok_or_else(|| PluginFailure::LocalData("尚未采集过该关卡".into()))?;
        let (extension, content) = match format {
            ExportFormat::Json => ("json", export::json(&archive)?),
            ExportFormat::Csv => ("csv", export::csv(&archive)),
            ExportFormat::Excel => ("xlsx", export::excel(&archive)?),
        };
        if content.len() > CORE_EXPORT_MAX_BYTES {
            return Err(PluginFailure::Other(
                "导出文件超过 Core 的 20 MiB 单文件上限；本地归档未受影响".into(),
            ));
        }
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

    /// 中途落盘：把已抓到的评论并进归档，并推进「最近一次采集时间」。
    ///
    /// **不动 `fetch_count`**：一次采集不管中途落盘几次，界面上都只算一次。
    fn checkpoint(
        &self,
        archive: &mut CommentArchive,
        incoming: Vec<CommentItem>,
        started_at: u64,
        pages: u32,
        message: &str,
    ) -> Result<(), PluginFailure> {
        archive.merge_comments(incoming);
        archive.updated_at = started_at;
        archive.last_pages = pages;
        archive.collection_state = CollectionState::Partial;
        archive.collection_message = message.to_owned();
        self.save(archive)
    }

    fn archive_path(&self, level_id: &str) -> Result<PathBuf, PluginFailure> {
        if !valid_level_id(level_id) {
            return Err(PluginFailure::InvalidInput);
        }
        Ok(self.root.join(format!("{level_id}.json")))
    }

    fn parse_archive(&self, level_id: &str, bytes: &[u8]) -> Result<CommentArchive, PluginFailure> {
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| {
            warn!(level_id, "本地归档不是有效的 JSON");
            PluginFailure::LocalData("本地归档不是有效的 JSON，请重新采集".into())
        })?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64);
        if !matches!(version, Some(1) | Some(2) | Some(3)) {
            warn!(level_id, ?version, "本地归档版本不匹配");
            return Err(PluginFailure::LocalData(
                "本地归档版本不匹配，请重新采集".into(),
            ));
        }
        let mut archive: CommentArchive = serde_json::from_value(value).map_err(|_| {
            warn!(level_id, "本地归档结构已变化");
            PluginFailure::LocalData("本地归档结构已变化，请重新采集".into())
        })?;
        if archive.level.level_id != level_id {
            warn!(level_id, "本地归档所属关卡与文件名不一致");
            return Err(PluginFailure::LocalData(
                "本地归档所属关卡与文件名不一致，请重新采集".into(),
            ));
        }
        if version == Some(1) {
            archive.schema_version = ARCHIVE_VERSION;
            archive.collection_state = CollectionState::Unknown;
            archive.collection_message.clear();
        }
        archive.schema_version = ARCHIVE_VERSION;
        Ok(archive)
    }

    /// 读取本地归档（不触发采集）；v1/v2 归档在内存中迁移到 v3。
    pub fn archive(&self, level_id: &str) -> Result<Option<CommentArchive>, PluginFailure> {
        let path = self.archive_path(level_id)?;
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(path).map_err(storage)?;
        self.parse_archive(level_id, &bytes).map(Some)
    }

    fn cached_archive(&self, level_id: &str) -> Result<Option<Arc<CommentArchive>>, PluginFailure> {
        let path = self.archive_path(level_id)?;
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(storage(error)),
        };
        let modified = metadata.modified().ok();
        if let Ok(cache) = self.view_cache.lock()
            && let Some(cached) = cache.as_ref()
            && cached.level_id == level_id
            && cached.length == metadata.len()
            && cached.modified == modified
        {
            return Ok(Some(cached.archive.clone()));
        }
        let archive = Arc::new(
            self.archive(level_id)?
                .ok_or(PluginFailure::InvalidResponse)?,
        );
        if let Ok(mut cache) = self.view_cache.lock() {
            *cache = Some(CachedArchive {
                level_id: level_id.to_owned(),
                modified,
                length: metadata.len(),
                archive: archive.clone(),
            });
        }
        Ok(Some(archive))
    }

    /// 给本插件 UI 返回一页完整评论，按主评论分组；筛选和排序在后端完成。
    pub fn archive_view(
        &self,
        query: ArchiveViewQuery,
    ) -> Result<Option<ArchiveViewPage>, PluginFailure> {
        if !(1..=100).contains(&query.limit) || query.keyword.chars().count() > 512 {
            return Err(PluginFailure::InvalidInput);
        }
        let Some(archive) = self.cached_archive(&query.level_id)? else {
            return Ok(None);
        };
        let mut positions = HashMap::<String, usize>::new();
        let mut groups = Vec::<GroupIndices>::new();
        for (index, item) in archive.comments.iter().enumerate() {
            let key = if item.is_sub {
                &item.parent_id
            } else {
                &item.reply_id
            };
            let position = *positions.entry(key.clone()).or_insert_with(|| {
                groups.push(GroupIndices {
                    main: None,
                    subs: Vec::new(),
                });
                groups.len() - 1
            });
            if item.is_sub {
                groups[position].subs.push(index);
            } else {
                groups[position].main = Some(index);
            }
        }
        let keyword = query.keyword.trim().to_lowercase();
        let mut visible: Vec<&GroupIndices> = groups
            .iter()
            .filter(|group| {
                let Some(main_index) = group.main else {
                    return false;
                };
                let main = &archive.comments[main_index];
                let selected = match query.filter {
                    ArchiveFilter::All => true,
                    ArchiveFilter::Recommend => main.is_recommend == Some(true),
                    ArchiveFilter::NotRecommend => main.is_recommend == Some(false),
                    ArchiveFilter::Owner => main.is_owner,
                };
                selected
                    && (keyword.is_empty()
                        || comment_matches(main, &keyword)
                        || group
                            .subs
                            .iter()
                            .any(|index| comment_matches(&archive.comments[*index], &keyword)))
            })
            .collect();
        visible.sort_by(|left, right| {
            let left = &archive.comments[left.main.expect("filtered groups have a main comment")];
            let right = &archive.comments[right.main.expect("filtered groups have a main comment")];
            match query.sort {
                ArchiveSort::Default => std::cmp::Ordering::Equal,
                ArchiveSort::Newest => right.created_at.cmp(&left.created_at),
                ArchiveSort::Oldest => left.created_at.cmp(&right.created_at),
                ArchiveSort::Likes => right.like_count.cmp(&left.like_count),
                ArchiveSort::Floor => floor_rank(&left.floor_id).cmp(&floor_rank(&right.floor_id)),
            }
        });
        let total_groups = visible.len().min(u32::MAX as usize) as u32;
        let start = (query.offset as usize).min(visible.len());
        let end = start
            .saturating_add(query.limit as usize)
            .min(visible.len());
        let page_groups = visible[start..end]
            .iter()
            .map(|group| CommentGroup {
                main: archive.comments[group.main.expect("filtered groups have a main comment")]
                    .clone(),
                subs: group
                    .subs
                    .iter()
                    .map(|index| archive.comments[*index].clone())
                    .collect(),
            })
            .collect();
        Ok(Some(ArchiveViewPage {
            overview: ArchiveOverview::from(archive.as_ref()),
            total_groups,
            offset: query.offset,
            groups: page_groups,
        }))
    }

    /// Read a bounded page for the declared inter-plugin archive service.
    /// UID, avatar URL, and IP region are omitted from this cross-plugin DTO.
    pub fn archive_page(
        &self,
        level_id: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Option<CommentArchivePage>, PluginFailure> {
        if !(1..=50).contains(&limit) {
            return Err(PluginFailure::InvalidInput);
        }
        let Some(archive) = self.cached_archive(level_id)? else {
            return Ok(None);
        };
        let total_count = archive.comments.len().min(u32::MAX as usize) as u32;
        let start = (offset as usize).min(archive.comments.len());
        let end = start
            .saturating_add(limit as usize)
            .min(archive.comments.len());
        let comments = archive.comments[start..end]
            .iter()
            .map(|item| CommentArchiveServiceItem {
                floor_id: item.floor_id.clone(),
                nickname: item.nickname.clone(),
                content: item.content.clone(),
                is_recommend: item.is_recommend,
                like_count: item.like_count,
                reply_count: item.reply_count,
                created_at: item.created_at,
                is_sub: item.is_sub,
                reply_to: item.reply_to.clone(),
            })
            .collect();
        Ok(Some(CommentArchivePage {
            level_id: archive.level.level_id.clone(),
            level_name: archive.level.level_name.clone(),
            updated_at: archive.updated_at,
            total_count,
            offset,
            comments,
        }))
    }

    fn save(&self, archive: &CommentArchive) -> Result<(), PluginFailure> {
        let path = self.archive_path(&archive.level.level_id)?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(storage)?;
        }
        // 读取缓存由文件长度和修改时间识别；先失效，避免原子替换前后落在同一时间粒度时读到旧页。
        if let Ok(mut cache) = self.view_cache.lock() {
            *cache = None;
        }
        // 半途失败不破坏上一份归档：先写临时文件再原子替换。
        monitor::atomic_json(&path, archive)?;
        if let Ok(mut cache) = self.view_cache.lock() {
            *cache = None;
        }
        Ok(())
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
            let level_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default();
            if !valid_level_id(level_id) {
                continue;
            }
            let parsed = fs::read(&path)
                .map_err(storage)
                .and_then(|bytes| self.parse_archive(level_id, &bytes));
            if let Ok(archive) = parsed {
                out.push(ArchiveSummary {
                    level_id: archive.level.level_id,
                    level_name: archive.level.level_name,
                    cover_url: archive.level.cover_url,
                    updated_at: archive.updated_at,
                    count: archive.comments.len() as u32,
                    collection_state: archive.collection_state,
                    needs_recollect: false,
                    last_new_counts: archive.last_new_counts,
                });
                continue;
            }
            // 即使 JSON 损坏也保留入口，否则用户无法从界面重新采集。
            out.push(ArchiveSummary {
                level_id: level_id.to_owned(),
                level_name: String::new(),
                cover_url: String::new(),
                updated_at: modified_at(&path),
                count: 0,
                collection_state: CollectionState::Unknown,
                needs_recollect: true,
                last_new_counts: None,
            });
        }
        out.sort_by_key(|item| std::cmp::Reverse(item.updated_at));
        Ok(out)
    }

    /// Return only the fields needed to select a local archive from another plugin.
    pub fn service_archives(&self) -> Result<Vec<CommentArchiveServiceSummary>, PluginFailure> {
        let mut summaries = self
            .archives()?
            .into_iter()
            .filter(|summary| !summary.needs_recollect)
            .map(|summary| CommentArchiveServiceSummary {
                level_id: summary.level_id,
                level_name: summary.level_name,
                updated_at: summary.updated_at,
                count: summary.count,
            })
            .collect::<Vec<_>>();
        summaries.truncate(10_000);
        Ok(summaries)
    }

    fn favorites_path(&self) -> PathBuf {
        self.root.join("favorites.json")
    }

    fn read_favorites(&self) -> Result<Vec<FavoriteLevel>, PluginFailure> {
        let path = self.favorites_path();
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(storage(error)),
        };
        let file: FavoritesFile = serde_json::from_slice(&bytes)
            .map_err(|_| PluginFailure::LocalData("本地收藏列表无法读取".into()))?;
        if file.schema_version != FAVORITES_SCHEMA_VERSION
            || file.items.len() > MAX_FAVORITES
            || file
                .items
                .iter()
                .any(|item| !valid_level_id(&item.level_id))
        {
            return Err(PluginFailure::LocalData("本地收藏列表格式不受支持".into()));
        }
        Ok(file.items)
    }

    fn save_favorites(&self, items: Vec<FavoriteLevel>) -> Result<(), PluginFailure> {
        let path = self.favorites_path();
        let bytes = serde_json::to_vec(&FavoritesFile {
            schema_version: FAVORITES_SCHEMA_VERSION,
            items,
        })
        .map_err(storage)?;
        let temp = path.with_extension("tmp");
        fs::write(&temp, bytes).map_err(storage)?;
        fs::rename(temp, path).map_err(storage)
    }

    /// 列出当前安装的插件在本机保存的奇域收藏。
    pub fn favorites(&self) -> Result<Vec<FavoriteLevel>, PluginFailure> {
        self.read_favorites()
    }

    /// 切换一个关卡的本地收藏状态，返回切换后的状态。
    pub fn toggle_favorite(&self, level_id: &str) -> Result<bool, PluginFailure> {
        if !valid_level_id(level_id) {
            return Err(PluginFailure::InvalidInput);
        }
        let _guard = self
            .favorites_lock
            .lock()
            .map_err(|_| PluginFailure::Other("本地收藏暂不可用".into()))?;
        let mut items = self.read_favorites()?;
        if let Some(index) = items.iter().position(|item| item.level_id == level_id) {
            items.remove(index);
            self.save_favorites(items)?;
            return Ok(false);
        }
        if items.len() >= MAX_FAVORITES {
            return Err(PluginFailure::Other("本地收藏已达到数量上限".into()));
        }
        let archive = self
            .archive(level_id)?
            .ok_or_else(|| PluginFailure::LocalData("请先打开或采集该关卡".into()))?;
        let added_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(storage)?
            .as_secs();
        items.insert(
            0,
            FavoriteLevel {
                level_id: archive.level.level_id,
                level_name: archive.level.level_name,
                cover_url: archive.level.cover_url,
                added_at,
            },
        );
        self.save_favorites(items)?;
        Ok(true)
    }

    /// Foreground full comparison; background monitoring uses independent cancellation.
    #[instrument(skip(self, query, progress), fields(plugin = "comment_collector", level_id = %query.level_id))]
    pub async fn collect<F: FnMut(CollectProgress) + Send>(
        &self,
        query: CommentQuery,
        progress: F,
    ) -> Result<CommentArchive, PluginFailure> {
        let _guard = self
            .collecting
            .try_lock()
            .map_err(|_| PluginFailure::Busy)?;
        self.cancel.store(false, Ordering::SeqCst);
        let options = self.monitor_config()?;
        self.bbs
            .set_request_interval(options.request_interval_ms as u64);
        self.collect_mode(
            &query.level_id,
            CollectionMode::Full,
            &options,
            self.cancel.clone(),
            progress,
        )
        .await
    }
}

#[derive(Debug, Clone)]
pub struct ExportPayload {
    pub name: String,
    pub content: Vec<u8>,
    pub format: ExportFormat,
    pub rows: u32,
}
