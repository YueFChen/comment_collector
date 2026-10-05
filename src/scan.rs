//! Conservative scans: replay opaque cursors, stop incremental work only after
//! overlapping known pages, and never infer deletion from the upstream window.
use crate::{
    CHECKPOINT_PAGES, CollectProgress, CollectionMode, CollectionState, CommentArchive,
    CommentCollector, CommentItem, LevelInfo, MonitorConfig, NewCommentCounts, PAGE_RETRIES,
    RETRY_BACKOFF,
    bbs::{self, MAX_PAGES, ReplySort},
    cancelled,
    monitor::{now, run_id},
    retryable, storage, valid_level_id,
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use wonderland_plugin_sdk::PluginFailure;

// Independent main/thread bounds avoid exhausting a 500-main-page allowance
// merely because inline previews need child expansion. Still bound total work.
const MAX_TOTAL_PAGES: u32 = 20_000;

impl CommentCollector {
    async fn scan_page(
        &self,
        level_id: &str,
        cursor: Option<&Value>,
        sort: ReplySort,
        parent: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<bbs::ReplyListData, PluginFailure> {
        let mut retries = 0;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(cancelled());
            }
            let result = if let Some(parent) = parent {
                self.bbs.sub_page(level_id, parent, cursor, cancel).await
            } else {
                self.bbs.page(level_id, cursor, sort, cancel).await
            };
            match result {
                Ok(page) => {
                    page.validate()?;
                    return Ok(page);
                }
                Err(error) if retryable(&error) && retries < PAGE_RETRIES => {
                    // Every retry also passes through the shared Bbs request limiter.
                    let backoff = RETRY_BACKOFF * 3u32.pow(retries);
                    if matches!(error, PluginFailure::Http(429)) {
                        self.bbs.cooldown(backoff).await;
                    }
                    tokio::time::sleep(backoff).await;
                    retries += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub(crate) async fn collect_mode<F: FnMut(CollectProgress) + Send>(
        &self,
        level_id: &str,
        mode: CollectionMode,
        options: &MonitorConfig,
        cancel: Arc<AtomicBool>,
        mut progress: F,
    ) -> Result<CommentArchive, PluginFailure> {
        if !valid_level_id(level_id) {
            return Err(PluginFailure::InvalidInput);
        }
        options.validate()?;
        let lock = self
            .level_locks
            .lock()
            .map_err(storage)?
            .entry(level_id.into())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = lock.try_lock_owned().map_err(|_| PluginFailure::Busy)?;
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        self.active_jobs
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < options.max_parallel).then_some(n + 1)
            })
            .map_err(|_| PluginFailure::Busy)?;
        let _slot = JobSlot(&self.active_jobs);
        let started_at = now();
        let id = run_id();
        let mut archive = match self.archive(level_id) {
            Ok(Some(archive)) => archive,
            Ok(None) => CommentArchive::empty(LevelInfo {
                level_id: level_id.into(),
                ..Default::default()
            }),
            Err(PluginFailure::LocalData(_)) => {
                // Keep damaged or future-version input for recovery rather than silently destroying it.
                let recovery = self.root.join("recovery");
                std::fs::create_dir_all(&recovery).map_err(storage)?;
                std::fs::copy(
                    self.archive_path(level_id)?,
                    recovery.join(format!("{level_id}-{id}.json")),
                )
                .map_err(storage)?;
                CommentArchive::empty(LevelInfo {
                    level_id: level_id.into(),
                    ..Default::default()
                })
            }
            Err(error) => return Err(error),
        };
        let original: HashSet<String> = archive
            .comments
            .iter()
            .map(|c| c.reply_id.clone())
            .collect();
        // A checkpoint contains observations, not a committed scan frontier. Reusing
        // those IDs as a stop boundary after a partial run would strand unseen pages.
        let frontier_path = self.root.join("frontiers").join(format!("{level_id}.json"));
        let known: HashSet<String> = match std::fs::read(&frontier_path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(storage)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if archive.collection_state == CollectionState::Complete {
                    archive
                        .comments
                        .iter()
                        .filter(|c| !c.is_sub)
                        .map(|c| c.reply_id.clone())
                        .collect()
                } else {
                    HashSet::new()
                }
            }
            Err(error) => return Err(storage(error)),
        };
        self.checkpoint(&mut archive, vec![], started_at, 0, "本次采集尚未完成")?;
        let mut pages = 0u32;
        let mut observed = HashMap::<String, CommentItem>::new();
        let mut pending = vec![];
        let result: Result<(), PluginFailure> = async {
            let mut retries = 0;
            let mut level = loop {
                if cancel.load(Ordering::SeqCst) {
                    return Err(cancelled());
                }
                match self.bbs.level(level_id, &cancel).await {
                    Ok(level) => break level,
                    Err(error) if retryable(&error) && retries < PAGE_RETRIES => {
                        let backoff = RETRY_BACKOFF * 3u32.pow(retries);
                        if matches!(error, PluginFailure::Http(429)) {
                            self.bbs.cooldown(backoff).await;
                        }
                        tokio::time::sleep(backoff).await;
                        retries += 1;
                    }
                    Err(error) => return Err(error),
                }
            };
            if level.level_id.is_empty() {
                level.level_id = level_id.into();
            }
            if level.level_id != level_id {
                return Err(PluginFailure::InvalidResponse);
            }
            archive.level = level;
            let sort = match mode {
                CollectionMode::Full => ReplySort::Hot,
                CollectionMode::Incremental => ReplySort::FloorDesc,
            };
            let mut cursor = None;
            let mut cursors = HashSet::new();
            let mut known_pages = 0;
            let mut main_pages = 0;
            let mut main_ids = HashSet::new();
            let mut short_threads = 0u32;
            loop {
                if main_pages >= MAX_PAGES || pages >= MAX_TOTAL_PAGES {
                    return Err(PluginFailure::Other(
                        "达到主评论 500 页或总请求 20000 页上限，已保留数据；未完成比对".into(),
                    ));
                }
                let page = self
                    .scan_page(level_id, cursor.as_ref(), sort, None, &cancel)
                    .await?;
                pages += 1;
                main_pages += 1;
                let items = page.items();
                let mains: Vec<_> = items.iter().filter(|c| !c.is_sub).cloned().collect();
                if mains.is_empty() && page.more() {
                    return Err(PluginFailure::InvalidResponse);
                }
                if !mains.is_empty() && mains.iter().all(|item| main_ids.contains(&item.reply_id)) {
                    return Err(PluginFailure::InvalidResponse);
                }
                main_ids.extend(mains.iter().map(|item| item.reply_id.clone()));
                if !mains.is_empty() && mains.iter().all(|c| known.contains(&c.reply_id)) {
                    known_pages += 1;
                } else {
                    known_pages = 0;
                }
                for item in &items {
                    observed.insert(item.reply_id.clone(), item.clone());
                }
                pending.extend(items);
                progress(CollectProgress {
                    page: pages,
                    fetched: observed.len().min(u32::MAX as usize) as u32,
                });

                // Inline sub_replies are only a preview (typically two). Expand every
                // visited nonempty thread; counts can lag and cannot prove completeness.
                for main in mains.iter().filter(|c| c.reply_count > 0) {
                    let mut sub_cursor = None;
                    let mut sub_cursors = HashSet::new();
                    let mut sub_ids = HashSet::new();
                    let mut sub_pages = 0;
                    loop {
                        if sub_pages >= MAX_PAGES || pages >= MAX_TOTAL_PAGES {
                            return Err(PluginFailure::Other(
                                "达到单楼 500 页或总请求 20000 页上限；未完成比对".into(),
                            ));
                        }
                        let sub = self
                            .scan_page(
                                level_id,
                                sub_cursor.as_ref(),
                                ReplySort::FloorAsc,
                                Some(&main.reply_id),
                                &cancel,
                            )
                            .await?;
                        pages += 1;
                        sub_pages += 1;
                        let items = sub.items_as_sub(&main.reply_id);
                        if items.is_empty() && sub.more() {
                            return Err(PluginFailure::InvalidResponse);
                        }
                        if !items.is_empty() && items.iter().all(|item| sub_ids.contains(&item.reply_id)) {
                            return Err(PluginFailure::InvalidResponse);
                        }
                        for item in &items {
                            sub_ids.insert(item.reply_id.clone());
                            observed.insert(item.reply_id.clone(), item.clone());
                        }
                        pending.extend(items);
                        progress(CollectProgress {
                            page: pages,
                            fetched: observed.len().min(u32::MAX as usize) as u32,
                        });
                        if pages.is_multiple_of(CHECKPOINT_PAGES) {
                            self.checkpoint(
                                &mut archive,
                                std::mem::take(&mut pending),
                                started_at,
                                pages,
                                "本次采集尚未完成",
                            )?;
                        }
                        if !sub.more() {
                            if sub_ids.len() < main.reply_count as usize {
                                // Count lag is local to this thread; continue scanning
                                // later parents so a stale count cannot starve them.
                                short_threads += 1;
                            }
                            break;
                        }
                        advance_cursor(&sub, &mut sub_cursor, &mut sub_cursors)?;
                    }
                }
                if pages.is_multiple_of(CHECKPOINT_PAGES) {
                    self.checkpoint(
                        &mut archive,
                        std::mem::take(&mut pending),
                        started_at,
                        pages,
                        "本次采集尚未完成",
                    )?;
                }
                if !page.more() {
                    break;
                }
                advance_cursor(&page, &mut cursor, &mut cursors)?;
                if mode == CollectionMode::Incremental && known_pages >= options.overlap_pages {
                    break;
                }
                if mode == CollectionMode::Incremental && main_pages >= options.incremental_max_pages {
                    return Err(PluginFailure::Other(
                        "增量页数达到上限且尚未遇到足够重叠页；请运行全量比对，不能认定已追上最新评论"
                            .into(),
                    ));
                }
            }
            if cancel.load(Ordering::SeqCst) {
                return Err(cancelled());
            }
            if short_threads > 0 {
                return Err(PluginFailure::Other(format!(
                    "{short_threads} 个楼中楼实际返回数小于主楼声明数量，可能存在分页遗漏或实时变化；已继续扫描其余楼层，但未完成比对"
                )));
            }
            Ok(())
        }.await;
        archive.merge_comments(pending);
        archive.updated_at = started_at;
        archive.last_pages = pages;
        archive.collection_state = if result.is_ok() {
            CollectionState::Complete
        } else {
            CollectionState::Partial
        };
        archive.collection_message = match &result {
            Ok(()) if mode == CollectionMode::Incremental => {
                "增量扫描已完成；旧楼修改与新增楼中楼由周期全量比对补查".into()
            }
            Ok(()) => "已遍历本次接口可见窗口；未出现的历史评论不认定为删除".into(),
            Err(error) => format!("{error}；已抓取部分已保留")
                .chars()
                .take(2048)
                .collect(),
        };
        if result.is_ok() {
            archive.fetch_count = archive.fetch_count.saturating_add(1);
        }
        let mut counts = NewCommentCounts::default();
        for comment in observed.values() {
            if comment.is_sub || original.contains(&comment.reply_id) {
                continue;
            }
            match comment.is_recommend {
                Some(true) => counts.recommended = counts.recommended.saturating_add(1),
                Some(false) => counts.not_recommended = counts.not_recommended.saturating_add(1),
                None => {}
            }
        }
        archive.last_new_counts = Some(counts);
        self.save(&archive)?;
        if result.is_ok() {
            let mut committed = known;
            committed.extend(
                observed
                    .values()
                    .filter(|c| !c.is_sub)
                    .map(|c| c.reply_id.clone()),
            );
            std::fs::create_dir_all(frontier_path.parent().expect("frontier directory"))
                .map_err(storage)?;
            crate::monitor::atomic_json(&frontier_path, &committed)?;
        }
        result.map(|()| archive)
    }
}
fn advance_cursor(
    page: &bbs::ReplyListData,
    cursor: &mut Option<Value>,
    seen: &mut HashSet<String>,
) -> Result<(), PluginFailure> {
    // Cursor loops and absent continuation tokens must never produce "complete".
    if page.next().is_empty() || !seen.insert(page.next().to_owned()) {
        return Err(PluginFailure::InvalidResponse);
    }
    *cursor = page.cursor().cloned();
    Ok(())
}

struct JobSlot<'a>(&'a std::sync::atomic::AtomicU32);
impl Drop for JobSlot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
