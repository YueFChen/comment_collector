//! Offline regressions for cursor safety, cumulative archives and monitor admission.
use super::*;
use crate::monitor::due_mode;
use serde_json::{Value, json};
use std::collections::{HashSet, VecDeque};
use tokio::sync::{Notify, Semaphore};

const LEVEL: &str = "100000001";

#[derive(Debug, Clone)]
struct Request {
    url: String,
    body: Value,
    at: tokio::time::Instant,
}

#[derive(Default)]
struct MockHttp {
    replies: StdMutex<VecDeque<Result<Value, PluginFailure>>>,
    requests: StdMutex<Vec<Request>>,
    metadata: StdMutex<Vec<String>>,
    gate: Option<Arc<Semaphore>>,
    entered: Notify,
    panic_on_metadata: bool,
}
impl MockHttp {
    fn with_pages(pages: impl IntoIterator<Item = Value>) -> Arc<Self> {
        Arc::new(Self {
            replies: StdMutex::new(pages.into_iter().map(Ok).collect()),
            ..Default::default()
        })
    }
    fn enqueue(&self, page: Result<Value, PluginFailure>) {
        self.replies.lock().unwrap().push_back(page);
    }
    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
    fn assert_drained(&self) {
        assert!(
            self.replies.lock().unwrap().is_empty(),
            "not every expected page was requested"
        );
    }
}
#[async_trait::async_trait]
impl PublicHttpClient for MockHttp {
    async fn get_bytes(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        _query: &[(String, String)],
    ) -> Result<Vec<u8>, PluginFailure> {
        assert!(headers.iter().any(|(name, _)| *name == "user-agent"));
        let id = url
            .split("level_id=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap();
        self.metadata.lock().unwrap().push(id.into());
        self.entered.notify_one();
        assert!(!self.panic_on_metadata, "intentional test transport panic");
        if let Some(gate) = &self.gate {
            gate.acquire().await.unwrap().forget();
        }
        Ok(serde_json::to_vec(&json!({"retcode": 0, "data": {
            "level_info": {"level_id": id, "level_name": "Offline test"}
        }}))
        .unwrap())
    }
    async fn post_json(
        &self,
        url: &str,
        _headers: &[(&str, &str)],
        body: &str,
    ) -> Result<Vec<u8>, PluginFailure> {
        self.requests.lock().unwrap().push(Request {
            url: url.into(),
            body: serde_json::from_str(body).unwrap(),
            at: tokio::time::Instant::now(),
        });
        let value = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected page request")?;
        Ok(serde_json::to_vec(&value).unwrap())
    }
}

struct TestRoot(PathBuf);
impl TestRoot {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("wonderland-monitor-test-{}", monitor::run_id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn options() -> MonitorConfig {
    MonitorConfig {
        request_interval_ms: 250,
        ..Default::default()
    }
}
fn setup(http: Arc<MockHttp>) -> (Arc<CommentCollector>, TestRoot) {
    let root = TestRoot::new();
    let collector = Arc::new(CommentCollector::new(root.0.clone(), http).unwrap());
    collector.configure_monitor(options()).unwrap();
    (collector, root)
}
fn item(id: &str, created_at: i64) -> Value {
    json!({"reply_id": id, "floor_id": id, "content": format!("comment {id}"),
        "created_at": created_at, "reply_stat": {"like_count": "1", "reply_count": "0"}})
}
fn page(items: Vec<Value>, cursor: Value) -> Value {
    json!({"retcode": 0, "data": {"reply_list": items, "cursor": cursor}})
}
fn last(items: Vec<Value>) -> Value {
    page(items, json!({"has_more": false, "next": ""}))
}
fn more(items: Vec<Value>, next: &str) -> Value {
    page(items, json!({"has_more": true, "next": next}))
}
fn seed(collector: &CommentCollector, items: Vec<Value>, state: CollectionState) {
    let mut archive = CommentArchive::empty(LevelInfo {
        level_id: LEVEL.into(),
        ..Default::default()
    });
    archive.comments = bbs::decode::<bbs::ReplyListData>(last(items))
        .unwrap()
        .items();
    archive.collection_state = state;
    archive.fetch_count = 4;
    collector.save(&archive).unwrap();
}
fn latest_archive(collector: &CommentCollector) -> CommentArchive {
    collector.archive(LEVEL).unwrap().unwrap()
}
fn frontier(collector: &CommentCollector) -> HashSet<String> {
    serde_json::from_slice(
        &fs::read(
            collector
                .root
                .join("frontiers")
                .join(format!("{LEVEL}.json")),
        )
        .unwrap(),
    )
    .unwrap()
}
async fn collect(
    collector: &CommentCollector,
    mode: CollectionMode,
) -> Result<CommentArchive, PluginFailure> {
    collector
        .collect_mode(
            LEVEL,
            mode,
            &options(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
}

#[tokio::test]
async fn request_bodies_keep_size_order_and_every_opaque_cursor_field() {
    let cursor = json!({"has_more": true, "next": "opaque+/=", "offset": "9007199254740993", "token": {"a": [1,"x"]}});
    let http = MockHttp::with_pages([
        page(vec![item("1", 7)], cursor.clone()),
        last(vec![item("2", 7)]),
    ]);
    let (collector, _root) = setup(http.clone());
    collect(&collector, CollectionMode::Full).await.unwrap();
    let requests = http.requests();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert!(request.url.ends_with("/reply/list?lang=zh-cn"));
        assert_eq!(request.body["level_id"], LEVEL);
        assert_eq!(request.body["uid"], "");
        assert_eq!(request.body["region"], "cn_gf01");
        assert_eq!(request.body["cursor"]["size"], 20);
        assert_eq!(request.body["cursor"]["sort_type"], "SORT_TYPE_HOT");
    }
    assert_eq!(requests[0].body["cursor"]["next"], "");
    let mut expected = cursor;
    expected["size"] = json!(20);
    expected["sort_type"] = json!("SORT_TYPE_HOT");
    assert_eq!(requests[1].body["cursor"], expected);
    // The shared limiter covers successive pages, not only distinct jobs.
    assert!(requests[1].at.duration_since(requests[0].at) >= Duration::from_millis(240));
    http.assert_drained();
}

#[tokio::test]
async fn incremental_overlap_counts_consecutive_known_pages_not_timestamp_watermarks() {
    let http = MockHttp::with_pages([
        more(vec![item("new-tie", 100), item("known-1", 100)], "A"),
        more(vec![item("known-2", 99)], "B"),
        more(vec![item("late-arrival", 1)], "C"),
        more(vec![item("known-3", 98)], "D"),
        more(vec![item("known-4", 97)], "E"),
    ]);
    let (collector, _root) = setup(http.clone());
    seed(
        &collector,
        vec![
            item("known-1", 100),
            item("known-2", 99),
            item("known-3", 98),
            item("known-4", 97),
        ],
        CollectionState::Complete,
    );
    let archive = collect(&collector, CollectionMode::Incremental)
        .await
        .unwrap();
    assert_eq!(
        archive.last_pages, 5,
        "a newly seen ID resets the known-page overlap"
    );
    assert_eq!(archive.comments.len(), 6);
    assert!(
        archive
            .comments
            .iter()
            .any(|c| c.reply_id == "late-arrival")
    );
    assert!(archive.comments.iter().any(|c| c.reply_id == "new-tie"));
    assert_eq!(archive.collection_state, CollectionState::Complete);
    for request in http.requests() {
        assert_eq!(request.body["cursor"]["sort_type"], "SORT_TYPE_FLOOR_DESC");
        assert_eq!(request.body["cursor"]["size"], 20);
    }
    http.assert_drained();
}

#[tokio::test]
async fn partial_failure_keeps_observations_but_cannot_advance_the_frontier() {
    let mut new_comment = item("partial-new", 20);
    new_comment["is_recommend"] = json!(true);
    let http = MockHttp::with_pages([more(vec![new_comment], "A")]);
    http.enqueue(Err(PluginFailure::Business {
        code: -1,
        message: "offline fixture failure".into(),
    }));
    let (collector, _root) = setup(http.clone());
    seed(
        &collector,
        vec![item("committed", 10)],
        CollectionState::Complete,
    );
    fs::create_dir_all(collector.root.join("frontiers")).unwrap();
    monitor::atomic_json(
        &collector
            .root
            .join("frontiers")
            .join(format!("{LEVEL}.json")),
        &vec!["committed"],
    )
    .unwrap();
    assert!(matches!(
        collect(&collector, CollectionMode::Incremental).await,
        Err(PluginFailure::Business { .. })
    ));
    assert_eq!(
        frontier(&collector),
        HashSet::from(["committed".to_owned()])
    );
    let archive = collector.archive(LEVEL).unwrap().unwrap();
    assert_eq!(archive.collection_state, CollectionState::Partial);
    assert_eq!(archive.fetch_count, 4);
    assert_eq!(archive.comments.len(), 2);
    assert!(archive.comments.iter().any(|c| c.reply_id == "partial-new"));
    assert_eq!(
        archive.last_new_counts,
        Some(NewCommentCounts {
            recommended: 1,
            not_recommended: 0
        })
    );
    // Replaying a checkpoint ID does not falsely satisfy a one-page stop boundary.
    http.enqueue(Ok(more(vec![item("partial-new", 20)], "REPLAY")));
    http.enqueue(Ok(last(vec![item("unseen-after-checkpoint", 19)])));
    let mut config = options();
    config.overlap_pages = 1;
    let recovered = collector
        .collect_mode(
            LEVEL,
            CollectionMode::Incremental,
            &config,
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(recovered.last_pages, 2);
    assert!(frontier(&collector).contains("unseen-after-checkpoint"));
    http.assert_drained();
}

#[tokio::test]
async fn cancellation_keeps_partial_data_without_publishing_a_frontier() {
    let http = MockHttp::with_pages([more(vec![item("captured", 20)], "A")]);
    let (collector, _root) = setup(http.clone());
    seed(&collector, vec![item("old", 10)], CollectionState::Complete);
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let result = collector
        .collect_mode(LEVEL, CollectionMode::Full, &options(), cancel, move |_| {
            stop.store(true, Ordering::SeqCst);
        })
        .await;
    assert!(matches!(result, Err(PluginFailure::Other(message)) if message.contains("取消")));
    assert!(
        !collector
            .root
            .join("frontiers")
            .join(format!("{LEVEL}.json"))
            .exists()
    );
    let saved = collector.archive(LEVEL).unwrap().unwrap();
    assert_eq!(saved.collection_state, CollectionState::Partial);
    assert_eq!(saved.fetch_count, 4);
    assert_eq!(saved.comments.len(), 2);
    assert!(!collector.root.join("snapshots").exists());
    http.assert_drained();
}

#[tokio::test]
async fn full_scan_updates_latest_retains_missing_ids_without_touching_legacy_files() {
    let mut revised = item("edited", 10);
    revised["content"] = json!("revised text");
    revised["reply_stat"]["like_count"] = json!(99);
    let http = MockHttp::with_pages([last(vec![revised.clone()])]);
    let (collector, _root) = setup(http.clone());
    seed(
        &collector,
        vec![item("edited", 10), item("not-observed", 9)],
        CollectionState::Complete,
    );
    // Existing legacy files are ignored and retained; no new history is written.
    let legacy_dir = collector.root.join("snapshots").join(LEVEL);
    fs::create_dir_all(&legacy_dir).unwrap();
    let legacy_path = legacy_dir.join("legacy.json");
    let bytes_before = b"legacy content that is deliberately not parseable";
    fs::write(&legacy_path, bytes_before).unwrap();
    let archive = collect(&collector, CollectionMode::Full).await.unwrap();
    assert_eq!(
        archive.comments.len(),
        2,
        "absence in a rolling window is never deletion"
    );
    let edited = archive
        .comments
        .iter()
        .find(|i| i.reply_id == "edited")
        .unwrap();
    assert_eq!(edited.content, "revised text");
    assert_eq!(edited.like_count, 99);
    assert_eq!(archive.collection_state, CollectionState::Complete);
    assert_eq!(archive.last_new_counts, Some(NewCommentCounts::default()));
    http.enqueue(Ok(last(vec![revised])));
    let latest = collect(&collector, CollectionMode::Incremental)
        .await
        .unwrap();
    assert_eq!(latest.comments.len(), 2);
    assert_eq!(fs::read(legacy_path).unwrap(), bytes_before);
    assert_eq!(fs::read_dir(&legacy_dir).unwrap().count(), 1);
    http.assert_drained();
}

#[tokio::test]
async fn child_threads_expand_all_ascending_pages_and_deduplicate_inline_previews() {
    let mut main = item("parent", 10);
    main["reply_stat"]["reply_count"] = json!(3);
    main["sub_replies"] = json!([item("child-1", 11), item("child-2", 12)]);
    let sub_cursor = json!({"has_more": true, "next": "CHILD-NEXT", "opaque": {"nested": true}});
    let http = MockHttp::with_pages([
        last(vec![main]),
        page(
            vec![item("child-1", 11), item("child-2", 12)],
            sub_cursor.clone(),
        ),
        last(vec![item("child-3", 13)]),
    ]);
    let (collector, _root) = setup(http.clone());
    let archive = collect(&collector, CollectionMode::Full).await.unwrap();
    assert_eq!(archive.last_pages, 3);
    assert_eq!(archive.comments.len(), 4);
    for child in archive
        .comments
        .iter()
        .filter(|i| i.reply_id.starts_with("child"))
    {
        assert!(child.is_sub);
        assert_eq!(child.parent_id, "parent");
        assert!(child.floor_id.is_empty());
    }
    let requests = http.requests();
    for request in &requests[1..] {
        assert!(request.url.contains("/level/reply/sub_replies?"));
        assert_eq!(request.body["parent_reply_id"], "parent");
        assert_eq!(request.body["cursor"]["sort_type"], "SORT_TYPE_FLOOR_ASC");
        assert_eq!(request.body["cursor"]["size"], 20);
    }
    let mut expected = sub_cursor;
    expected["size"] = json!(20);
    expected["sort_type"] = json!("SORT_TYPE_FLOOR_ASC");
    assert_eq!(requests[2].body["cursor"], expected);
    http.assert_drained();
}

#[tokio::test]
async fn invalid_continuation_cursors_never_mark_a_full_scan_complete() {
    let cases = vec![
        vec![page(vec![item("a", 1)], json!({"has_more": true}))],
        vec![more(vec![item("a", 1)], "")],
        vec![more(vec![], "A")],
        vec![more(vec![item("a", 1)], "A"), more(vec![item("b", 2)], "A")],
        vec![page(
            vec![item("a", 1)],
            json!({"has_more": true, "next": 123}),
        )],
    ];
    for pages in cases {
        let http = MockHttp::with_pages(pages);
        let (collector, _root) = setup(http.clone());
        assert!(matches!(
            collect(&collector, CollectionMode::Full).await,
            Err(PluginFailure::InvalidResponse)
        ));
        assert_eq!(
            collector.archive(LEVEL).unwrap().unwrap().collection_state,
            CollectionState::Partial
        );
        assert_eq!(
            latest_archive(&collector).collection_state,
            CollectionState::Partial
        );
        assert!(!collector.root.join("frontiers").exists());
        http.assert_drained();
    }
}

#[tokio::test]
async fn overlap_stop_cannot_hide_an_invalid_cursor() {
    for cursor in [
        json!({"has_more": true}),
        json!({"has_more": true, "next": ""}),
    ] {
        let http = MockHttp::with_pages([page(vec![item("known", 1)], cursor)]);
        let (collector, _root) = setup(http.clone());
        seed(
            &collector,
            vec![item("known", 1)],
            CollectionState::Complete,
        );
        let mut config = options();
        config.overlap_pages = 1;
        let result = collector
            .collect_mode(
                LEVEL,
                CollectionMode::Incremental,
                &config,
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .await;
        assert!(matches!(result, Err(PluginFailure::InvalidResponse)));
        assert_eq!(
            latest_archive(&collector).collection_state,
            CollectionState::Partial
        );
        http.assert_drained();
    }
}

#[tokio::test]
async fn incremental_page_budget_is_partial_without_a_committed_frontier() {
    let http = MockHttp::with_pages([more(vec![item("a", 1)], "A"), more(vec![item("b", 2)], "B")]);
    let (collector, _root) = setup(http.clone());
    let mut config = options();
    config.incremental_max_pages = 2;
    let result = collector
        .collect_mode(
            LEVEL,
            CollectionMode::Incremental,
            &config,
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await;
    assert!(matches!(result, Err(PluginFailure::Other(message)) if message.contains("增量页数")));
    let archive = latest_archive(&collector);
    assert_eq!(archive.collection_state, CollectionState::Partial);
    assert_eq!(archive.comments.len(), 2);
    assert!(!collector.root.join("frontiers").exists());
    http.assert_drained();
}

#[test]
fn monitor_config_is_opt_in_validated_and_persisted() {
    let (collector, root) = setup(MockHttp::with_pages([]));
    let default = MonitorConfig::default();
    assert!(!default.enabled);
    assert!(default.level_ids.is_empty());
    assert_eq!(default.incremental_interval_secs, 1800);
    assert_eq!(default.full_interval_secs, 86_400);
    assert_eq!(default.max_parallel, 2);
    assert_eq!(default.request_interval_ms, 500);
    assert_eq!(default.overlap_pages, 2);
    assert_eq!(default.incremental_max_pages, 50);
    let mut config = options();
    config.incremental_interval_secs = 300; // Existing custom settings are not replaced by the new default.
    config.enabled = true;
    config.level_ids = vec![LEVEL.into(), "2".into()];
    collector.configure_monitor(config.clone()).unwrap();
    let restarted = CommentCollector::new(root.0.clone(), MockHttp::with_pages([])).unwrap();
    assert_eq!(
        serde_json::to_value(restarted.monitor_config().unwrap()).unwrap(),
        serde_json::to_value(config.clone()).unwrap()
    );
    let mut invalid = vec![];
    let mut c = config.clone();
    c.level_ids.push(LEVEL.into());
    invalid.push(c);
    let mut c = config.clone();
    c.level_ids = vec!["../1".into()];
    invalid.push(c);
    let mut c = config.clone();
    c.level_ids = (0..101).map(|i| i.to_string()).collect();
    invalid.push(c);
    let mut c = config.clone();
    c.incremental_interval_secs = 59;
    invalid.push(c);
    let mut c = config.clone();
    c.full_interval_secs = 299;
    invalid.push(c);
    let mut c = config.clone();
    c.full_interval_secs = 2_592_001;
    invalid.push(c);
    let mut c = config.clone();
    c.incremental_interval_secs = 600;
    c.full_interval_secs = 300;
    invalid.push(c);
    for value in [0, 5] {
        let mut c = config.clone();
        c.max_parallel = value;
        invalid.push(c);
    }
    for value in [249, 60_001] {
        let mut c = config.clone();
        c.request_interval_ms = value;
        invalid.push(c);
    }
    for value in [0, 11] {
        let mut c = config.clone();
        c.overlap_pages = value;
        invalid.push(c);
    }
    for value in [0, 501, 1] {
        let mut c = config.clone();
        c.incremental_max_pages = value;
        invalid.push(c);
    }
    for invalid in invalid {
        assert!(matches!(
            collector.configure_monitor(invalid),
            Err(PluginFailure::InvalidInput)
        ));
    }
    assert_eq!(
        serde_json::to_value(collector.monitor_config().unwrap()).unwrap(),
        serde_json::to_value(config).unwrap()
    );
    let mut unknown = serde_json::to_value(default).unwrap();
    unknown["surprise"] = json!(true);
    assert!(serde_json::from_value::<MonitorConfig>(unknown).is_err());
}

#[test]
fn scheduler_prioritizes_full_scan_and_respects_failure_cooldown() {
    let config = MonitorConfig {
        incremental_interval_secs: 300,
        ..Default::default()
    };
    let mut status = MonitorLevelStatus::default();
    assert_eq!(
        due_mode(&config, &status, 100_000),
        Some(CollectionMode::Full)
    );
    status.last_full_success_at = 99_000;
    assert_eq!(
        due_mode(&config, &status, 100_000),
        Some(CollectionMode::Incremental)
    );
    status.last_attempt_at = 99_900;
    assert_eq!(due_mode(&config, &status, 100_000), None);
    assert_eq!(
        due_mode(&config, &status, 100_200),
        Some(CollectionMode::Incremental)
    );
    status.consecutive_failures = 1;
    assert_eq!(
        due_mode(&config, &status, 100_500),
        Some(CollectionMode::Full),
        "recovery uses a full scan even when the last full is recent"
    );
    status.last_full_success_at = 1;
    assert_eq!(
        due_mode(&config, &status, 100_200),
        None,
        "a due full scan does not bypass errors"
    );
    assert_eq!(
        due_mode(&config, &status, 100_500),
        Some(CollectionMode::Full)
    );
    status.consecutive_failures = u32::MAX;
    assert_eq!(due_mode(&config, &status, 99_900 + 19_199), None);
    assert_eq!(
        due_mode(&config, &status, 99_900 + 19_200),
        Some(CollectionMode::Full)
    );
    status.running = true;
    assert_eq!(due_mode(&config, &status, u64::MAX), None);
    status.running = false;
    assert_eq!(
        due_mode(&config, &status, 1),
        None,
        "clock rollback cannot run work early"
    );
}

#[test]
fn full_deadline_is_independent_of_recent_incremental_attempts() {
    let config = MonitorConfig {
        incremental_interval_secs: 1800,
        full_interval_secs: 86_400,
        ..Default::default()
    };
    let full_success = 100_000;
    let deadline = full_success + config.full_interval_secs as u64;
    let mut status = MonitorLevelStatus {
        last_full_success_at: full_success,
        last_attempt_at: deadline - 60,
        ..Default::default()
    };
    assert_eq!(due_mode(&config, &status, deadline - 1), None);
    assert_eq!(
        due_mode(&config, &status, deadline),
        Some(CollectionMode::Full)
    );
    assert_eq!(
        due_mode(&config, &status, deadline + 1),
        Some(CollectionMode::Full)
    );
    status.running = true;
    assert_eq!(due_mode(&config, &status, deadline), None);
    status.running = false;
    status.consecutive_failures = 1;
    assert_eq!(due_mode(&config, &status, deadline), None);
    let retry_at = status.last_attempt_at + 3600;
    assert_eq!(
        due_mode(&config, &status, retry_at),
        Some(CollectionMode::Full)
    );
}

#[tokio::test]
async fn foreground_full_baseline_is_reused_after_adding_monitor_and_restart() {
    let http = MockHttp::with_pages([last(vec![item("baseline", 1)])]);
    let (collector, _root) = setup(http.clone());
    let started = monitor::now();
    collector
        .collect(
            CommentQuery {
                level_id: LEVEL.into(),
            },
            |_| {},
        )
        .await
        .unwrap();
    collector.add_monitor(LEVEL).unwrap();
    let status = collector.monitor_status().unwrap().levels.remove(0);
    assert!(status.last_full_success_at >= started);
    assert_eq!(status.last_mode, Some(CollectionMode::Full));
    assert_eq!(status.consecutive_failures, 0);
    assert!(!status.needs_full_recovery);
    assert_eq!(due_mode(&options(), &status, monitor::now()), None);
    assert_eq!(
        status.next_collect_at,
        Some(status.last_attempt_at + options().incremental_interval_secs as u64)
    );

    let restarted = CommentCollector::new(collector.root.clone(), http.clone()).unwrap();
    let restored = restarted.monitor_status().unwrap().levels.remove(0);
    assert_eq!(restored.last_full_success_at, status.last_full_success_at);
    assert_eq!(due_mode(&options(), &restored, monitor::now()), None);
    restarted
        .request_monitor_run(LEVEL, CollectionMode::Incremental)
        .unwrap();
    assert_eq!(
        restarted.monitor.lock().unwrap().requested.get(LEVEL),
        Some(&CollectionMode::Incremental)
    );
    assert_eq!(
        http.requests().len(),
        1,
        "adding or restarting does not repeat full collection"
    );
    http.assert_drained();
}

#[tokio::test]
async fn foreground_failed_scan_does_not_publish_a_full_baseline() {
    let http = MockHttp::with_pages([page(vec![], json!({"has_more": true, "next": "bad"}))]);
    let (collector, _root) = setup(http.clone());
    assert!(
        collector
            .collect(
                CommentQuery {
                    level_id: LEVEL.into()
                },
                |_| {}
            )
            .await
            .is_err()
    );
    collector.add_monitor(LEVEL).unwrap();
    let status = collector.monitor_status().unwrap().levels.remove(0);
    assert_eq!(status.last_full_success_at, 0);
    assert_eq!(
        due_mode(&options(), &status, monitor::now()),
        Some(CollectionMode::Full)
    );
    http.assert_drained();
}

#[tokio::test]
async fn foreground_full_clears_previous_monitor_failure_and_recovery_state() {
    let http = MockHttp::with_pages([last(vec![item("recovered", 1)])]);
    let (collector, _root) = setup(http.clone());
    collector.add_monitor(LEVEL).unwrap();
    collector.monitor_status().unwrap();
    collector.monitor.lock().unwrap().statuses.insert(
        LEVEL.into(),
        MonitorLevelStatus {
            level_id: LEVEL.into(),
            last_attempt_at: 1,
            last_full_success_at: 1,
            needs_full_recovery: true,
            consecutive_failures: 3,
            last_error: "old failure".into(),
            ..Default::default()
        },
    );
    collector
        .collect(
            CommentQuery {
                level_id: LEVEL.into(),
            },
            |_| {},
        )
        .await
        .unwrap();
    let status = collector.monitor_status().unwrap().levels.remove(0);
    assert_eq!(status.consecutive_failures, 0);
    assert!(!status.needs_full_recovery);
    assert!(status.last_error.is_empty());
    assert_eq!(due_mode(&options(), &status, monitor::now()), None);
    collector
        .request_monitor_run(LEVEL, CollectionMode::Incremental)
        .unwrap();
    assert_eq!(
        collector.monitor.lock().unwrap().requested.get(LEVEL),
        Some(&CollectionMode::Incremental)
    );
    http.assert_drained();
}

#[test]
fn restart_clears_running_flag_preserves_state_and_manual_requests_are_scoped() {
    let (collector, _root) = setup(MockHttp::with_pages([]));
    let mut config = options();
    config.enabled = true;
    config.level_ids = vec![LEVEL.into()];
    collector.configure_monitor(config.clone()).unwrap();
    let status = MonitorLevelStatus {
        level_id: LEVEL.into(),
        running: true,
        last_attempt_at: 50,
        last_success_at: 40,
        last_full_success_at: 30,
        consecutive_failures: 2,
        last_error: "previous error".into(),
        last_mode: Some(CollectionMode::Incremental),
        needs_full_recovery: false,
        next_collect_at: None,
    };
    monitor::atomic_json(&collector.root.join("monitor-state.json"), &vec![status]).unwrap();
    let status = collector.monitor_status().unwrap().levels.remove(0);
    assert!(!status.running);
    assert!(status.last_error.contains("中断"));
    assert_eq!(status.last_success_at, 40);
    assert_eq!(status.last_full_success_at, 30);
    assert_eq!(status.consecutive_failures, 2);
    assert!(matches!(
        collector.request_monitor_run("2", CollectionMode::Full),
        Err(PluginFailure::InvalidInput)
    ));
    collector
        .request_monitor_run(LEVEL, CollectionMode::Full)
        .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    collector
        .monitor
        .lock()
        .unwrap()
        .active
        .insert(LEVEL.into(), cancel.clone());
    assert!(matches!(
        collector.request_monitor_run(LEVEL, CollectionMode::Full),
        Err(PluginFailure::Busy)
    ));
    config.enabled = false;
    collector.configure_monitor(config).unwrap();
    assert!(cancel.load(Ordering::SeqCst));
    assert!(collector.monitor.lock().unwrap().requested.is_empty());
    assert!(matches!(
        collector.request_monitor_run(LEVEL, CollectionMode::Full),
        Err(PluginFailure::InvalidInput)
    ));
}

#[tokio::test]
async fn level_exclusion_and_global_job_limit_allow_distinct_levels_concurrently() {
    let gate = Arc::new(Semaphore::new(0));
    let http = Arc::new(MockHttp {
        gate: Some(gate.clone()),
        ..Default::default()
    });
    http.enqueue(Ok(last(vec![])));
    http.enqueue(Ok(last(vec![])));
    let (collector, _root) = setup(http.clone());
    let first_collector = collector.clone();
    let first = tokio::spawn(async move {
        first_collector
            .collect_mode(
                LEVEL,
                CollectionMode::Full,
                &options(),
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), http.entered.notified())
        .await
        .unwrap();
    assert!(matches!(
        collect(&collector, CollectionMode::Full).await,
        Err(PluginFailure::Busy)
    ));
    let second_collector = collector.clone();
    let second = tokio::spawn(async move {
        second_collector
            .collect_mode(
                "2",
                CollectionMode::Full,
                &options(),
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), http.entered.notified())
        .await
        .unwrap();
    assert_eq!(collector.active_jobs.load(Ordering::SeqCst), 2);
    assert_eq!(
        http.metadata.lock().unwrap().len(),
        2,
        "distinct jobs have both entered the network"
    );
    assert!(matches!(
        collector
            .collect_mode(
                "3",
                CollectionMode::Full,
                &options(),
                Arc::new(AtomicBool::new(false)),
                |_| {}
            )
            .await,
        Err(PluginFailure::Busy)
    ));
    gate.add_permits(2);
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    assert_eq!(collector.active_jobs.load(Ordering::SeqCst), 0);
    http.assert_drained();
}

#[tokio::test]
async fn cancelled_before_admission_does_not_consume_a_job_slot_or_issue_requests() {
    let http = MockHttp::with_pages([]);
    let (collector, _root) = setup(http.clone());
    assert!(
        collector
            .collect_mode(
                LEVEL,
                CollectionMode::Full,
                &options(),
                Arc::new(AtomicBool::new(true)),
                |_| {}
            )
            .await
            .is_err()
    );
    assert_eq!(collector.active_jobs.load(Ordering::SeqCst), 0);
    assert!(http.metadata.lock().unwrap().is_empty());
    assert!(http.requests().is_empty());
    assert!(collector.archive(LEVEL).unwrap().is_none());
}

#[test]
fn malformed_response_envelopes_and_pages_fail_loudly() {
    for value in [
        json!({}),
        json!({"retcode": "0", "data": {}}),
        json!({"retcode": 0, "data": null}),
        json!({"retcode": 0, "data": {"cursor": {"has_more": false}}}),
    ] {
        assert!(matches!(
            bbs::decode::<bbs::ReplyListData>(value),
            Err(PluginFailure::InvalidResponse)
        ));
    }
    for value in [
        page(vec![], json!(null)),
        page(vec![], json!({})),
        page(vec![], json!({"has_more": "maybe"})),
        page(vec![], json!({"has_more": 2})),
        last(vec![json!({"content": "missing ID"})]),
    ] {
        let decoded: bbs::ReplyListData = bbs::decode(value).unwrap();
        assert!(matches!(
            decoded.validate(),
            Err(PluginFailure::InvalidResponse)
        ));
    }
    for flag in [
        json!(true),
        json!(false),
        json!("true"),
        json!("false"),
        json!("1"),
        json!("0"),
        json!(1),
        json!(0),
    ] {
        let decoded: bbs::ReplyListData =
            bbs::decode(page(vec![], json!({"has_more": flag, "next": "A"}))).unwrap();
        decoded.validate().unwrap();
    }
}

#[test]
fn malformed_inline_child_cannot_be_silently_discarded() {
    let mut main = item("main", 1);
    main["sub_replies"] = json!([{"content": "child with missing ID"}]);
    let decoded: bbs::ReplyListData = bbs::decode(last(vec![main])).unwrap();
    assert!(matches!(
        decoded.validate(),
        Err(PluginFailure::InvalidResponse)
    ));
}

async fn wait_for(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("condition did not become true");
}

#[tokio::test]
async fn cancellation_while_waiting_for_shared_throttle_prevents_dispatch() {
    let http = MockHttp::with_pages([]);
    let (collector, _root) = setup(http.clone());
    collector.bbs.set_request_interval(1_000);
    collector
        .bbs
        .level(LEVEL, &AtomicBool::new(false))
        .await
        .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker = collector.clone();
    let token = cancel.clone();
    let task = tokio::spawn(async move {
        worker
            .collect_mode(LEVEL, CollectionMode::Full, &options(), token, |_| {})
            .await
    });
    // Initial checkpoint is written before metadata queues behind the earlier request.
    wait_for(|| collector.root.join(format!("{LEVEL}.json")).exists()).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    cancel.store(true, Ordering::SeqCst);
    assert!(task.await.unwrap().is_err());
    assert_eq!(
        http.metadata.lock().unwrap().len(),
        1,
        "canceled queued metadata must not dispatch"
    );
    assert!(http.requests().is_empty());
    assert_eq!(collector.active_jobs.load(Ordering::SeqCst), 0);
    assert_eq!(
        latest_archive(&collector).collection_state,
        CollectionState::Partial
    );
}

#[tokio::test]
async fn child_count_shortfall_is_partial_but_does_not_starve_later_main_pages() {
    let mut parent = item("parent", 1);
    parent["reply_stat"]["reply_count"] = json!(5);
    let http = MockHttp::with_pages([
        more(vec![parent], "NEXT-MAIN"),
        last(vec![item("only-child", 2)]),
        last(vec![item("later-main", 3)]),
    ]);
    let (collector, _root) = setup(http.clone());
    seed(
        &collector,
        vec![item("historical", 0)],
        CollectionState::Complete,
    );
    assert!(matches!(collect(&collector, CollectionMode::Full).await,
        Err(PluginFailure::Other(message)) if message.contains("楼中楼")));
    let archive = latest_archive(&collector);
    assert_eq!(archive.collection_state, CollectionState::Partial);
    assert!(archive.comments.iter().any(|c| c.reply_id == "later-main"));
    assert!(archive.comments.iter().any(|c| c.reply_id == "only-child"));
    assert!(!collector.root.join("frontiers").exists());
    http.assert_drained();
}

#[tokio::test]
async fn latest_new_recommendation_counts_persist_and_exclude_edits_children_and_old_rows() {
    let mut old = item("old", 1);
    old["is_recommend"] = json!(true);
    let mut edited = old.clone();
    edited["is_recommend"] = json!(false);
    let mut positive = item("positive", 2);
    positive["is_recommend"] = json!(true);
    positive["reply_stat"]["reply_count"] = json!(1);
    let mut positive_two = item("positive-two", 3);
    positive_two["is_recommend"] = json!(true);
    let mut negative = item("negative", 4);
    negative["is_recommend"] = json!(false);
    let unknown = item("unknown", 5);
    let mut child = item("child", 6);
    child["is_recommend"] = json!(false);
    let main_rows = vec![edited, positive, positive_two, negative, unknown];
    let http = MockHttp::with_pages([last(main_rows.clone()), last(vec![child.clone()])]);
    let (collector, _root) = setup(http.clone());
    seed(&collector, vec![old], CollectionState::Complete);
    collect(&collector, CollectionMode::Full).await.unwrap();
    let expected = Some(NewCommentCounts {
        recommended: 2,
        not_recommended: 1,
    });
    assert_eq!(latest_archive(&collector).last_new_counts, expected);
    assert_eq!(collector.archives().unwrap()[0].last_new_counts, expected);
    let restarted = CommentCollector::new(collector.root.clone(), http.clone()).unwrap();
    assert_eq!(latest_archive(&restarted).last_new_counts, expected);
    assert_eq!(restarted.archives().unwrap()[0].last_new_counts, expected);

    http.enqueue(Ok(last(main_rows)));
    http.enqueue(Ok(last(vec![child])));
    collect(&collector, CollectionMode::Full).await.unwrap();
    assert_eq!(
        latest_archive(&collector).last_new_counts,
        Some(NewCommentCounts::default())
    );
    assert!(!collector.root.join("snapshots").exists());
    http.assert_drained();
}

#[tokio::test]
async fn complete_single_main_page_accepts_verified_hot_to_floor_desc_fallback() {
    for total in [json!(6), json!("6")] {
        let mut response = page(
            (1..=6).map(|id| item(&id.to_string(), id)).collect(),
            json!({"has_more": false, "next": "2", "sort_type": "SORT_TYPE_FLOOR_DESC"}),
        );
        response["data"]["total"] = total;
        let http = MockHttp::with_pages([response]);
        let (collector, _root) = setup(http.clone());
        let archive = collect(&collector, CollectionMode::Full).await.unwrap();
        assert_eq!(archive.comments.len(), 6);
        assert_eq!(archive.collection_state, CollectionState::Complete);
        assert_eq!(
            latest_archive(&collector).collection_state,
            CollectionState::Complete
        );
        assert_eq!(frontier(&collector).len(), 6);
        assert_eq!(http.requests().len(), 1);
        assert_eq!(
            http.requests()[0].body["cursor"]["sort_type"],
            "SORT_TYPE_HOT"
        );
        http.assert_drained();
    }
}

#[tokio::test]
async fn full_scan_replays_hot_to_floor_desc_cursor_and_keeps_desc_when_omitted() {
    for first_page_transition in [false, true] {
        let mut pages = Vec::new();
        if !first_page_transition {
            pages.push(page(
                vec![item("hot", 1)],
                json!({"has_more": true, "next": "20", "sort_type": "SORT_TYPE_HOT"}),
            ));
        }
        // The same token can occur in the HOT-offset and DESC-floor namespaces.
        let transition = json!({"has_more": true, "next": "20",
            "sort_type": "SORT_TYPE_FLOOR_DESC", "opaque": {"token": "unchanged"}});
        pages.push(page(vec![item("transition", 2)], transition.clone()));
        pages.push(more(vec![item("desc", 3)], "19"));
        pages.push(last(vec![item("terminal", 4)]));
        let http = MockHttp::with_pages(pages);
        let (collector, _root) = setup(http.clone());
        let archive = collect(&collector, CollectionMode::Full).await.unwrap();
        assert_eq!(archive.collection_state, CollectionState::Complete);
        assert_eq!(
            archive.comments.len(),
            if first_page_transition { 3 } else { 4 }
        );
        let requests = http.requests();
        let transition_index = usize::from(!first_page_transition);
        assert_eq!(requests[0].body["cursor"]["sort_type"], "SORT_TYPE_HOT");
        let mut expected = transition;
        expected["size"] = json!(20);
        assert_eq!(requests[transition_index + 1].body["cursor"], expected);
        assert_eq!(
            requests[transition_index + 2].body["cursor"]["sort_type"],
            "SORT_TYPE_FLOOR_DESC"
        );
        assert_eq!(frontier(&collector).len(), archive.comments.len());
        http.assert_drained();
    }
}

#[tokio::test]
async fn unexpected_reply_sort_changes_cannot_be_committed_as_a_complete_scan() {
    for (child, mode, request_sort, response_sort) in [
        (
            true,
            CollectionMode::Full,
            "SORT_TYPE_FLOOR_ASC",
            "SORT_TYPE_FLOOR_DESC",
        ),
        (
            false,
            CollectionMode::Full,
            "SORT_TYPE_HOT",
            "SORT_TYPE_NEW",
        ),
        (
            false,
            CollectionMode::Full,
            "SORT_TYPE_FLOOR_DESC",
            "SORT_TYPE_HOT",
        ),
        (
            false,
            CollectionMode::Incremental,
            "SORT_TYPE_FLOOR_DESC",
            "SORT_TYPE_HOT",
        ),
    ] {
        let mut parent = item("parent", 1);
        parent["reply_stat"]["reply_count"] = json!(1);
        let wrong_sort = page(
            vec![item("reply", 2)],
            json!({"has_more": false, "next": "", "sort_type": response_sort}),
        );
        let pages = if child {
            vec![last(vec![parent]), wrong_sort]
        } else if mode == CollectionMode::Full && request_sort == "SORT_TYPE_FLOOR_DESC" {
            vec![
                page(
                    vec![item("transition", 1)],
                    json!({"has_more": true, "next": "20", "sort_type": request_sort}),
                ),
                wrong_sort,
            ]
        } else {
            vec![wrong_sort]
        };
        let http = MockHttp::with_pages(pages);
        let (collector, _root) = setup(http.clone());
        assert!(matches!(
            collect(&collector, mode).await,
            Err(PluginFailure::InvalidResponse)
        ));
        assert_eq!(
            latest_archive(&collector).collection_state,
            CollectionState::Partial
        );
        assert!(!collector.root.join("frontiers").exists());
        http.assert_drained();
    }
}

#[tokio::test]
async fn repeated_page_ids_are_detected_even_if_cursor_changes() {
    for child in [false, true] {
        let repeated = vec![
            more(vec![item("same", 1)], "A"),
            more(vec![item("same", 1)], "B"),
        ];
        let pages = if child {
            let mut parent = item("parent", 0);
            parent["reply_stat"]["reply_count"] = json!(2);
            [vec![last(vec![parent])], repeated].concat()
        } else {
            repeated
        };
        let http = MockHttp::with_pages(pages);
        let (collector, _root) = setup(http.clone());
        assert!(matches!(
            collect(&collector, CollectionMode::Full).await,
            Err(PluginFailure::InvalidResponse)
        ));
        assert_eq!(
            latest_archive(&collector).collection_state,
            CollectionState::Partial
        );
        assert!(!collector.root.join("frontiers").exists());
        http.assert_drained();
    }
}

#[tokio::test]
async fn retryable_page_failures_retry_with_same_cursor_and_bounded_backoff() {
    let http = MockHttp::with_pages([]);
    http.enqueue(Err(PluginFailure::Http(429)));
    http.enqueue(Err(PluginFailure::Http(503)));
    http.enqueue(Ok(last(vec![item("recovered", 1)])));
    let (collector, _root) = setup(http.clone());
    let archive = collect(&collector, CollectionMode::Full).await.unwrap();
    assert_eq!(archive.comments.len(), 1);
    let requests = http.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].body, requests[1].body);
    assert_eq!(requests[1].body, requests[2].body);
    assert!(requests[1].at.duration_since(requests[0].at) >= Duration::from_millis(290));
    assert!(requests[2].at.duration_since(requests[1].at) >= Duration::from_millis(890));
    http.assert_drained();
}

#[tokio::test]
async fn exhausted_retries_preserve_partial_state_and_release_job_slot() {
    let http = MockHttp::with_pages([]);
    for _ in 0..3 {
        http.enqueue(Err(PluginFailure::Timeout));
    }
    let (collector, _root) = setup(http.clone());
    assert!(matches!(
        collect(&collector, CollectionMode::Full).await,
        Err(PluginFailure::Timeout)
    ));
    assert_eq!(
        http.requests().len(),
        3,
        "two retries plus the original request"
    );
    assert_eq!(collector.active_jobs.load(Ordering::SeqCst), 0);
    assert_eq!(
        latest_archive(&collector).collection_state,
        CollectionState::Partial
    );
    assert!(!collector.root.join("frontiers").exists());
    http.assert_drained();
}

#[tokio::test]
async fn explicitly_requested_monitor_run_survives_foreground_contention() {
    let http = MockHttp::with_pages([last(vec![])]);
    let (collector, _root) = setup(http.clone());
    let mut config = options();
    config.enabled = true;
    config.level_ids = vec![LEVEL.into()];
    collector.configure_monitor(config.clone()).unwrap();
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    collector
        .level_locks
        .lock()
        .unwrap()
        .insert(LEVEL.into(), lock.clone());
    let guard = lock.lock().await;
    collector
        .request_monitor_run(LEVEL, CollectionMode::Full)
        .unwrap();
    collector.start_monitoring();
    wait_for(|| {
        let monitor = collector.monitor.lock().unwrap();
        collector.root.join("monitor-state.json").exists()
            && monitor.active.is_empty()
            && monitor.requested.get(LEVEL) == Some(&CollectionMode::Full)
    })
    .await;
    let status = collector.monitor_status().unwrap().levels.remove(0);
    assert_eq!(status.consecutive_failures, 0);
    assert_eq!(
        status.last_attempt_at, 0,
        "contention is not an upstream attempt"
    );
    assert!(http.metadata.lock().unwrap().is_empty());
    drop(guard);
    wait_for(|| collector.monitor_status().unwrap().levels[0].last_full_success_at > 0).await;
    assert!(collector.monitor.lock().unwrap().requested.is_empty());
    assert_eq!(collector.active_jobs.load(Ordering::SeqCst), 0);
    config.enabled = false;
    collector.configure_monitor(config).unwrap();
    http.assert_drained();
}

#[tokio::test]
async fn panicked_monitor_worker_releases_scheduler_and_global_slots() {
    let http = Arc::new(MockHttp {
        panic_on_metadata: true,
        ..Default::default()
    });
    let (collector, _root) = setup(http);
    let mut config = options();
    config.enabled = true;
    config.level_ids = vec![LEVEL.into()];
    collector.configure_monitor(config.clone()).unwrap();
    collector.start_monitoring();
    wait_for(|| collector.monitor_status().unwrap().levels[0].consecutive_failures == 1).await;
    let status = collector.monitor_status().unwrap().levels.remove(0);
    assert!(!status.running);
    assert!(status.last_error.contains("异常结束"));
    assert_eq!(status.last_success_at, 0);
    assert!(collector.monitor.lock().unwrap().active.is_empty());
    assert_eq!(collector.active_jobs.load(Ordering::SeqCst), 0);
    assert_eq!(
        collector.archive(LEVEL).unwrap().unwrap().collection_state,
        CollectionState::Partial
    );
    config.enabled = false;
    collector.configure_monitor(config).unwrap();
}

#[tokio::test]
async fn partial_archive_without_frontier_never_supplies_an_overlap_boundary() {
    let http = MockHttp::with_pages([
        more(vec![item("checkpoint-only", 2)], "A"),
        last(vec![item("older-missed", 1)]),
    ]);
    let (collector, _root) = setup(http.clone());
    seed(
        &collector,
        vec![item("checkpoint-only", 2)],
        CollectionState::Partial,
    );
    let mut config = options();
    config.overlap_pages = 1;
    let archive = collector
        .collect_mode(
            LEVEL,
            CollectionMode::Incremental,
            &config,
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(archive.last_pages, 2);
    assert_eq!(archive.comments.len(), 2);
    assert!(frontier(&collector).contains("older-missed"));
    http.assert_drained();
}

#[tokio::test]
async fn corrupt_archive_raw_bytes_survive_recollection() {
    let http = MockHttp::with_pages([last(vec![item("new", 10)])]);
    let (collector, _root) = setup(http);
    let original = b"{broken archive bytes\n";
    fs::write(collector.root.join(format!("{LEVEL}.json")), original).unwrap();
    let archive = collector
        .collect_mode(
            LEVEL,
            CollectionMode::Full,
            &options(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
        .unwrap();
    assert_eq!(archive.comments.len(), 1);
    let files: Vec<_> = fs::read_dir(collector.root.join("recovery"))
        .unwrap()
        .collect();
    assert_eq!(files.len(), 1);
    assert_eq!(
        fs::read(files[0].as_ref().unwrap().path()).unwrap(),
        original
    );
}

#[test]
fn interrupted_full_scan_requires_full_recovery_even_with_recent_success() {
    let (collector, _root) = setup(MockHttp::with_pages([]));
    let mut config = options();
    config.enabled = true;
    config.level_ids = vec![LEVEL.into()];
    collector.configure_monitor(config.clone()).unwrap();
    let prior = MonitorLevelStatus {
        level_id: LEVEL.into(),
        running: true,
        last_attempt_at: 100,
        last_full_success_at: 99,
        last_mode: Some(CollectionMode::Full),
        ..Default::default()
    };
    monitor::atomic_json(&collector.root.join("monitor-state.json"), &vec![prior]).unwrap();
    let restored = collector.monitor_status().unwrap().levels.remove(0);
    assert!(!restored.running);
    assert_eq!(restored.consecutive_failures, 1);
    assert_eq!(
        due_mode(&config, &restored, 4000),
        Some(CollectionMode::Full)
    );
}

#[test]
fn monitor_target_actions_preserve_settings_pause_and_archive() {
    let (collector, _root) = setup(MockHttp::with_pages([]));
    let mut config = options();
    config.incremental_interval_secs = 600;
    collector.configure_monitor(config).unwrap();
    let first = collector.add_monitor(LEVEL).unwrap();
    assert!(first.enabled);
    assert_eq!(first.level_ids, vec![LEVEL]);
    assert_eq!(first.incremental_interval_secs, 600);
    seed(
        &collector,
        vec![item("existing", 1)],
        CollectionState::Complete,
    );
    let mut paused = first;
    paused.enabled = false;
    collector.configure_monitor(paused).unwrap();
    assert!(!collector.add_monitor(LEVEL).unwrap().enabled);
    let second = collector.add_monitor("000000002").unwrap();
    assert!(!second.enabled);
    assert_eq!(second.level_ids, vec![LEVEL, "000000002"]);
    assert_eq!(second.incremental_interval_secs, 600);
    collector.remove_monitor(LEVEL).unwrap();
    assert!(collector.archive(LEVEL).unwrap().is_some());
    let empty = collector.remove_monitor("000000002").unwrap();
    assert!(!empty.enabled);
    assert!(empty.level_ids.is_empty());
    assert!(collector.add_monitor("../invalid").is_err());
}

#[test]
fn concurrent_monitor_adds_never_replace_another_target() {
    let (collector, _root) = setup(MockHttp::with_pages([]));
    let mut tasks = vec![];
    for id in 1..=8 {
        let collector = collector.clone();
        tasks.push(std::thread::spawn(move || {
            collector.add_monitor(&id.to_string()).unwrap()
        }));
    }
    for task in tasks {
        task.join().unwrap();
    }
    let config = collector.monitor_config().unwrap();
    assert_eq!(config.level_ids.len(), 8);
    for id in 1..=8 {
        assert!(config.level_ids.contains(&id.to_string()));
    }
}

#[tokio::test]
async fn paused_full_scan_requires_persisted_full_recovery() {
    let gate = Arc::new(Semaphore::new(0));
    let http = Arc::new(MockHttp {
        gate: Some(gate.clone()),
        ..Default::default()
    });
    let (collector, root) = setup(http.clone());
    let mut config = options();
    config.enabled = true;
    config.level_ids = vec![LEVEL.into()];
    collector.configure_monitor(config.clone()).unwrap();
    let timestamp = monitor::now();
    let prior = MonitorLevelStatus {
        level_id: LEVEL.into(),
        last_attempt_at: timestamp,
        last_success_at: timestamp,
        last_full_success_at: timestamp,
        last_mode: Some(CollectionMode::Full),
        ..Default::default()
    };
    monitor::atomic_json(&collector.root.join("monitor-state.json"), &vec![prior]).unwrap();
    collector
        .request_monitor_run(LEVEL, CollectionMode::Full)
        .unwrap();
    collector.start_monitoring();
    http.entered.notified().await;
    config.enabled = false;
    collector.configure_monitor(config.clone()).unwrap();
    gate.add_permits(1);
    wait_for(|| !collector.monitor_status().unwrap().levels[0].running).await;
    let status = collector.monitor_status().unwrap().levels.remove(0);
    assert!(status.needs_full_recovery);
    assert_eq!(status.last_full_success_at, timestamp);
    assert_eq!(status.consecutive_failures, 0);
    assert_eq!(
        collector.archive(LEVEL).unwrap().unwrap().collection_state,
        CollectionState::Partial
    );
    assert_eq!(
        due_mode(
            &config,
            &status,
            timestamp + config.incremental_interval_secs as u64 + 1
        ),
        Some(CollectionMode::Full)
    );

    // Restart preserves the recovery requirement; even an explicit incremental
    // request must complete the unfinished full traversal first.
    drop(collector);
    let resumed_http = MockHttp::with_pages([last(vec![item("recovered", 1)])]);
    let resumed = Arc::new(CommentCollector::new(root.0.clone(), resumed_http.clone()).unwrap());
    assert!(resumed.monitor_status().unwrap().levels[0].needs_full_recovery);
    config.enabled = true;
    resumed.configure_monitor(config.clone()).unwrap();
    resumed
        .request_monitor_run(LEVEL, CollectionMode::Incremental)
        .unwrap();
    assert_eq!(
        resumed.monitor.lock().unwrap().requested.get(LEVEL),
        Some(&CollectionMode::Full)
    );
    resumed.start_monitoring();
    wait_for(|| !resumed.monitor_status().unwrap().levels[0].needs_full_recovery).await;
    let status = resumed.monitor_status().unwrap().levels.remove(0);
    assert!(!status.running);
    assert_eq!(status.last_mode, Some(CollectionMode::Full));
    assert_eq!(
        resumed_http.requests()[0].body["cursor"]["sort_type"],
        "SORT_TYPE_HOT"
    );
    config.enabled = false;
    resumed.configure_monitor(config).unwrap();
    resumed_http.assert_drained();
}

#[test]
fn old_archive_versions_load_without_stats_or_rewriting_files() {
    let (collector, _root) = setup(MockHttp::with_pages([]));
    seed(
        &collector,
        vec![item("existing", 1)],
        CollectionState::Complete,
    );
    let path = collector.archive_path(LEVEL).unwrap();
    let mut legacy: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("last_new_counts");
    for version in [1, 2] {
        legacy["schema_version"] = json!(version);
        let bytes = serde_json::to_vec(&legacy).unwrap();
        fs::write(&path, &bytes).unwrap();
        let archive = latest_archive(&collector);
        assert_eq!(archive.schema_version, ARCHIVE_VERSION);
        assert_eq!(archive.comments.len(), 1);
        assert_eq!(
            archive.collection_state,
            if version == 1 {
                CollectionState::Unknown
            } else {
                CollectionState::Complete
            }
        );
        assert_eq!(archive.last_new_counts, None);
        assert_eq!(collector.archives().unwrap()[0].last_new_counts, None);
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn displayed_next_collection_matches_admission_cooldown_and_current_settings() {
    let (collector, _root) = setup(MockHttp::with_pages([]));
    let mut config = options();
    config.enabled = true;
    config.level_ids = vec![LEVEL.into()];
    collector.configure_monitor(config.clone()).unwrap();
    collector.monitor_status().unwrap();
    let timestamp = monitor::now();
    let attempt = timestamp - 60;
    for (failures, wait) in [(0, 1800), (1, 3600), (u32::MAX, 86_400)] {
        let state = MonitorLevelStatus {
            level_id: LEVEL.into(),
            last_attempt_at: attempt,
            last_full_success_at: timestamp - 45,
            consecutive_failures: failures,
            ..Default::default()
        };
        collector
            .monitor
            .lock()
            .unwrap()
            .statuses
            .insert(LEVEL.into(), state.clone());
        let next = collector.monitor_status().unwrap().levels[0]
            .next_collect_at
            .unwrap();
        assert_eq!(next, attempt + wait);
        assert_eq!(due_mode(&config, &state, next - 1), None);
        assert!(due_mode(&config, &state, next).is_some());
    }
    let state = MonitorLevelStatus {
        level_id: LEVEL.into(),
        last_attempt_at: attempt,
        last_full_success_at: timestamp - 45,
        ..Default::default()
    };
    collector
        .monitor
        .lock()
        .unwrap()
        .statuses
        .insert(LEVEL.into(), state);
    config.incremental_interval_secs = 600;
    collector.configure_monitor(config.clone()).unwrap();
    assert_eq!(
        collector.monitor_status().unwrap().levels[0].next_collect_at,
        Some(attempt + 600)
    );
    config.enabled = false;
    collector.configure_monitor(config).unwrap();
    assert_eq!(
        collector.monitor_status().unwrap().levels[0].next_collect_at,
        None
    );
}

#[test]
fn next_collection_reports_first_run_manual_queue_and_active_run_truthfully() {
    let (collector, _root) = setup(MockHttp::with_pages([]));
    let mut config = options();
    config.enabled = true;
    config.level_ids = vec![LEVEL.into()];
    collector.configure_monitor(config.clone()).unwrap();
    let timestamp = monitor::now();
    let first = collector.monitor_status().unwrap().levels[0]
        .next_collect_at
        .unwrap();
    assert!(first >= timestamp && first <= monitor::now());
    collector.monitor.lock().unwrap().statuses.insert(
        LEVEL.into(),
        MonitorLevelStatus {
            level_id: LEVEL.into(),
            last_attempt_at: timestamp,
            last_full_success_at: timestamp,
            ..Default::default()
        },
    );
    collector
        .request_monitor_run(LEVEL, CollectionMode::Incremental)
        .unwrap();
    let queued = collector.monitor_status().unwrap().levels[0]
        .next_collect_at
        .unwrap();
    assert!(
        queued >= timestamp && queued <= monitor::now(),
        "manual requests bypass the timer"
    );
    collector
        .monitor
        .lock()
        .unwrap()
        .statuses
        .get_mut(LEVEL)
        .unwrap()
        .running = true;
    assert_eq!(
        collector.monitor_status().unwrap().levels[0].next_collect_at,
        None
    );
    config.enabled = false;
    collector.configure_monitor(config).unwrap();
    assert_eq!(
        collector.monitor_status().unwrap().levels[0].next_collect_at,
        None
    );
}

#[test]
fn displayed_full_deadline_tracks_settings_and_survives_restart() {
    let http = MockHttp::with_pages([]);
    let (collector, _root) = setup(http.clone());
    let mut config = options();
    config.enabled = true;
    config.level_ids = vec![LEVEL.into()];
    collector.configure_monitor(config.clone()).unwrap();
    collector.monitor_status().unwrap();
    let timestamp = monitor::now();
    let full_success = timestamp - config.full_interval_secs as u64 + 60;
    collector
        .record_full_success(LEVEL, timestamp - 10, "fixture")
        .unwrap();
    let mut state = collector.monitor.lock().unwrap().statuses[LEVEL].clone();
    state.last_full_success_at = full_success;
    state.last_attempt_at = timestamp - 10;
    collector
        .monitor
        .lock()
        .unwrap()
        .statuses
        .insert(LEVEL.into(), state.clone());
    fs::write(
        collector.root.join("monitor-state.json"),
        serde_json::to_vec(&vec![state.clone()]).unwrap(),
    )
    .unwrap();
    assert_eq!(
        collector.monitor_status().unwrap().levels[0].next_collect_at,
        Some(timestamp + 60)
    );
    let restarted = CommentCollector::new(collector.root.clone(), http).unwrap();
    assert_eq!(
        restarted.monitor_status().unwrap().levels[0].next_collect_at,
        Some(timestamp + 60)
    );
    config.full_interval_secs += 300;
    restarted.configure_monitor(config.clone()).unwrap();
    assert_eq!(
        restarted.monitor_status().unwrap().levels[0].next_collect_at,
        Some(timestamp + 360)
    );
    config.full_interval_secs -= 600;
    restarted.configure_monitor(config.clone()).unwrap();
    assert_eq!(
        due_mode(&config, &state, timestamp),
        Some(CollectionMode::Full)
    );
    let displayed = restarted.monitor_status().unwrap().levels[0]
        .next_collect_at
        .unwrap();
    assert!(displayed >= timestamp && displayed <= monitor::now());
}
