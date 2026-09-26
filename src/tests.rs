use super::*;

fn fixture(name: &str) -> serde_json::Value {
    let source = match name {
        "detail" => include_str!("../tests/fixtures/detail.json"),
        "page1" => include_str!("../tests/fixtures/page1.json"),
        "page2" => include_str!("../tests/fixtures/page2.json"),
        _ => include_str!("../tests/fixtures/business.json"),
    };
    serde_json::from_str(source).unwrap()
}

fn level_of(value: &serde_json::Value) -> LevelInfo {
    bbs::decode::<bbs::LevelDetailData>(value.clone())
        .unwrap()
        .into_level()
}

fn setup() -> (CommentCollector, PathBuf) {
    // 计数器不能省：这些用例是并行跑的，只靠时钟纳秒会让相邻两次 `setup()` 拿到同一个路径，
    // 一个用例收尾的 `remove_dir_all` 会把另一个正在写的目录删掉（表现为 "系统找不到指定的路径"）。
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let temp = std::env::temp_dir().join(format!(
        "wonderland-comment-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let plugin = CommentCollector::new(temp.join("archives"), Arc::new(NoNetwork)).unwrap();
    (plugin, temp)
}

struct NoNetwork;

#[async_trait::async_trait]
impl PublicHttpClient for NoNetwork {
    async fn get_bytes(&self, _url: &str, _headers: &[(&str, &str)], _query: &[(String, String)]) -> Result<Vec<u8>, PluginFailure> {
        Err(PluginFailure::Connection)
    }
    async fn post_json(&self, _url: &str, _headers: &[(&str, &str)], _body: &str) -> Result<Vec<u8>, PluginFailure> {
        Err(PluginFailure::Connection)
    }
}

/// 一份含主评论、楼中楼、需转义内容与脏数据的归档。
fn sample_archive() -> CommentArchive {
    let level = level_of(&fixture("detail"));
    let mut archive = CommentArchive::empty(level.clone());
    let mut items = bbs::decode::<bbs::ReplyListData>(fixture("page1"))
        .unwrap()
        .items();
    items.extend(
        bbs::decode::<bbs::ReplyListData>(fixture("page2"))
            .unwrap()
            .items(),
    );
    archive.merge(level, 1_750_000_000, 2, items);
    archive
}

#[test]
fn protocol_surfaces_business_and_normalizes_scalars() {
    let level = level_of(&fixture("detail"));
    assert_eq!(level.level_id, "100000001");
    assert_eq!(level.level_name, "测试关卡");
    assert_eq!(level.hot_score, "872");
    assert_eq!(level.good_rate, "96.7%");
    assert_eq!(level.play_range, "1-8");
    assert_eq!(level.play_type, "情景观赏");
    assert_eq!(level.cover_url, "https://example.invalid/cover.png");
    // 空 URL 的图被滤掉。
    assert_eq!(
        level.images,
        vec!["https://example.invalid/1.png".to_owned()]
    );

    // 官方业务错误带上官方文案，不被当成结构变化。
    assert!(matches!(
        bbs::decode::<bbs::LevelDetailData>(fixture("business")),
        Err(PluginFailure::Business { code: -2000431, .. })
    ));
    assert!(matches!(
        bbs::decode::<bbs::LevelDetailData>(serde_json::json!({ "retcode": 0, "data": null })),
        Err(PluginFailure::InvalidResponse)
    ));
}

#[test]
fn scalar_accepts_both_strings_and_numbers() {
    // 同一响应里两种类型混用，任一种都要能收下。
    let quoted: bbs::Scalar = serde_json::from_str("\"872\"").unwrap();
    assert_eq!(quoted.text(), "872");
    assert_eq!(quoted.u32(), 872);
    let bare: bbs::Scalar = serde_json::from_str("872").unwrap();
    assert_eq!(bare.text(), "872");
    let rate: bbs::Scalar = serde_json::from_str("\"96.7%\"").unwrap();
    assert_eq!(rate.text(), "96.7%");
    // 解析不出数字时按 0 处理，不 panic；缺字段（null）同理。
    assert_eq!(rate.u32(), 0);
    let empty: bbs::Scalar = serde_json::from_str("null").unwrap();
    assert!(empty.text().is_empty());
    assert_eq!(empty.i64(), 0);
}

#[test]
fn expands_sub_replies_and_marks_unknown_recommend() {
    let page: bbs::ReplyListData = bbs::decode(fixture("page1")).unwrap();
    assert!(page.more());
    assert_eq!(page.next(), "NEXT-1");
    let items = page.items();
    assert_eq!(items.len(), 3, "两条主评论 + 一条楼中楼");

    let main = &items[0];
    assert_eq!(main.reply_id, "1001");
    assert_eq!(main.floor_id, "1");
    assert_eq!(main.like_count, 12, "字符串型计数照样解析");
    assert_eq!(main.reply_count, 1);
    assert_eq!(main.is_recommend, Some(true));
    assert_eq!(main.ip_region, "浙江");
    assert!(!main.is_sub);
    assert_eq!(main.parent_id, "");

    let sub = &items[1];
    assert!(sub.is_sub);
    assert_eq!(sub.parent_id, "1001");
    // 楼中楼另有子楼层序号，与主楼层不同源，归档里留空。
    assert_eq!(sub.floor_id, "");
    // 官方对楼中楼也下发 is_recommend（实测恒为 false），照实展示会给每条回复盖「不推荐」，故留空。
    assert_eq!(sub.is_recommend, None);
    assert_eq!(
        sub.reply_to, "",
        "回复楼中楼时的 r_user 才非空，直接回复主评论时为空"
    );
    assert_eq!(sub.created_at, 1_750_000_100, "字符串型时间戳照样解析");

    let owner = &items[2];
    assert!(owner.is_owner);
    assert_eq!(owner.floor_id, "2");
    assert_eq!(owner.is_recommend, Some(false));

    let last: bbs::ReplyListData = bbs::decode(fixture("page2")).unwrap();
    assert!(!last.more());
    assert_eq!(last.next(), "");
    // 缺 id 的脏数据被跳过，不会以空 id 混进归档（空 id 会在去重时互相覆盖）。
    assert_eq!(last.items().len(), 1);
}

#[test]
fn archive_merge_dedups_and_accumulates() {
    let level = level_of(&fixture("detail"));
    let mut archive = CommentArchive::empty(level.clone());
    let first = bbs::decode::<bbs::ReplyListData>(fixture("page1"))
        .unwrap()
        .items();
    archive.merge(level.clone(), 1_000, 1, first);
    assert_eq!(archive.fetch_count, 1);
    assert_eq!(archive.last_pages, 1);
    assert_eq!(archive.comments.len(), 3);
    assert_eq!(archive.main_count(), 2);

    // 第二次只回一条主评论，且点赞数被官方修订：应就地更新而不是追加。
    let mut second = bbs::decode::<bbs::ReplyListData>(fixture("page1"))
        .unwrap()
        .items();
    second.truncate(1);
    second[0].like_count = 99;
    archive.merge(level, 2_000, 1, second);
    assert_eq!(archive.fetch_count, 2);
    assert_eq!(archive.updated_at, 2_000);
    assert_eq!(archive.comments.len(), 3, "去重后不会产生副本");
    assert_eq!(archive.comments[0].like_count, 99, "较新的数据覆盖旧的");
    // 上一轮抓到的另一条主评论与楼中楼都还在。
    assert_eq!(archive.comments[1].reply_id, "1002");
    assert_eq!(archive.comments[2].reply_id, "1003");
}

#[test]
fn exports_carry_chinese_and_quote_correctly() {
    let archive = sample_archive();
    let csv = export::csv(&archive);
    assert_eq!(csv[..3], [0xEF, 0xBB, 0xBF], "CSV 必须带 UTF-8 BOM");
    let text = String::from_utf8(csv[3..].to_vec()).unwrap();
    assert!(text.starts_with("楼层,昵称,UID"));
    assert!(
        text.contains("\"带,逗号与\"\"引号\"\"\n以及换行的内容\""),
        "含分隔符的字段按 RFC 4180 加引号"
    );
    // 主评论：推荐列有值；楼中楼：推荐列留空。
    assert!(text.contains("旅行者甲,8001,否,是,12,1,"));
    assert!(text.contains("旅行者乙,8002,否,,0,0,"));

    // JSON 是完整归档，可原样读回。
    let json = export::json(&archive).unwrap();
    let parsed: CommentArchive = serde_json::from_slice(&json).unwrap();
    assert_eq!(parsed.comments.len(), archive.comments.len());
    assert_eq!(parsed.comments[0].nickname, "旅行者甲");
    assert_eq!(parsed.level.level_name, "测试关卡");

    // xlsx 是 zip 容器，内容非空即可（中文由 xlsx 自身的 UTF-8 XML 保证）。
    let excel = export::excel(&archive).unwrap();
    assert_eq!(excel[..2], *b"PK");
    assert!(excel.len() > 1_000);

    assert!(export::file_name(&archive, "csv").starts_with("测试关卡_100000001_"));
    assert_eq!(export::slug("a/b:c*d?e"), "a_b_c_d_e");
    assert_eq!(export::slug("   "), "关卡");
}

#[test]
fn csv_escapes_formula_like_public_text() {
    let mut archive = sample_archive();
    archive.comments[0].nickname = "  =HYPERLINK(\"https://example.invalid\")".to_owned();
    archive.comments[0].content = "\t+SUM(1,1)".to_owned();
    archive.comments[0].ip_region = "@SUM(1,1)".to_owned();

    let text = String::from_utf8(export::csv(&archive)[3..].to_vec()).unwrap();
    assert!(text.contains("'  =HYPERLINK("));
    assert!(text.contains("\"'\t+SUM(1,1)\""));
    assert!(text.contains("'@SUM(1,1)"));
}

#[test]
fn storage_round_trip_and_export_paths() {
    let (plugin, temp) = setup();
    let archive = sample_archive();
    assert!(plugin.archive("100000001").unwrap().is_none());
    plugin.save(&archive).unwrap();

    let loaded = plugin.archive("100000001").unwrap().unwrap();
    assert_eq!(loaded.level.level_name, "测试关卡");
    assert_eq!(loaded.comments.len(), archive.comments.len());

    let list = plugin.archives().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].count, archive.comments.len() as u32);
    assert_eq!(list[0].level_id, "100000001");
    // 列表卡片要用封面做背景，所以清单里必须带上它。
    assert_eq!(list[0].cover_url, "https://example.invalid/cover.png");

    // 插件只生成内容与安全文件名，实际落盘交由 Core 的 files.export 服务。
    for format in [ExportFormat::Json, ExportFormat::Csv, ExportFormat::Excel] {
        let payload = plugin.export_payload("100000001", format).unwrap();
        assert!(!payload.content.is_empty());
        assert_eq!(payload.rows as usize, archive.comments.len());
    }
    // 没采过的关卡不能凭空导出。
    assert!(matches!(
        plugin.export_payload("999999", ExportFormat::Csv),
        Err(PluginFailure::LocalData(_))
    ));

    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn rejects_archive_that_is_not_readable() {
    let (plugin, temp) = setup();
    // 版本不匹配：必须在解析整体之前就被拦下，否则只会剩一句模糊的"无法解析"。
    let mut archive = sample_archive();
    archive.schema_version = ARCHIVE_VERSION + 1;
    plugin.save(&archive).unwrap();
    assert!(matches!(
        plugin.archive("100000001"),
        Err(PluginFailure::LocalData(message)) if message.contains("版本")
    ));

    // 结构变化（这里用缺字段模拟）与根本不是 JSON 的文件，都要给出各自的诊断而不是崩溃。
    let path = plugin.archive_path("100000001").unwrap();
    let mut value: serde_json::Value = serde_json::to_value(sample_archive()).unwrap();
    value["level"].as_object_mut().unwrap().remove("hot_score");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(matches!(
        plugin.archive("100000001"),
        Err(PluginFailure::LocalData(message)) if message.contains("结构")
    ));

    let broken = plugin.archive_path("100000002").unwrap();
    fs::write(&broken, b"not json").unwrap();
    assert!(matches!(
        plugin.archive("100000002"),
        Err(PluginFailure::LocalData(message)) if message.contains("JSON")
    ));

    // 清单：结构对不上的归档仍要列出来，否则界面显示"还没采过"、用户以为数据丢了；
    // 内容不可信时退回文件名。连 JSON 都不是的文件没有任何可展示信息，跳过。
    let list = plugin.archives().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].level_id, "100000001", "退回文件名");
    assert!(list[0].level_name.is_empty());
    assert_eq!(list[0].count, 0);

    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn checkpoint_merges_comments_without_counting_a_fetch() {
    let level = level_of(&fixture("detail"));
    let mut archive = CommentArchive::empty(level.clone());
    let items = bbs::decode::<bbs::ReplyListData>(fixture("page1"))
        .unwrap()
        .items();

    // 检查点只并数据：一次采集中途落盘多次，界面上仍然只算一次。
    archive.merge_comments(items.clone());
    assert_eq!(archive.comments.len(), 3);
    assert_eq!(archive.fetch_count, 0);
    assert_eq!(archive.updated_at, 0);

    // 整轮跑完才记账；检查点并过的条目不会变成副本。
    archive.merge(level, 1_750_000_000, 5, items);
    assert_eq!(archive.fetch_count, 1);
    assert_eq!(archive.last_pages, 5);
    assert_eq!(archive.updated_at, 1_750_000_000);
    assert_eq!(archive.comments.len(), 3);
}

#[test]
fn only_network_failures_are_worth_retrying() {
    for error in [
        PluginFailure::Timeout,
        PluginFailure::Connection,
        PluginFailure::Http(429),
        PluginFailure::Http(503),
    ] {
        assert!(retryable(&error), "{error:?} 属于网络类失败，应该重试");
    }
    // 业务错误与结构变化重试多少次都是同一个答案。
    for error in [
        PluginFailure::InvalidInput,
        PluginFailure::InvalidResponse,
        PluginFailure::Http(404),
        PluginFailure::LocalData(String::new()),
        PluginFailure::Business {
            code: -2000431,
            message: String::new(),
        },
    ] {
        assert!(!retryable(&error), "{error:?} 不该重试");
    }
}

#[test]
fn rejects_unsafe_level_id() {
    let (plugin, temp) = setup();
    assert!(valid_level_id("100000001"));
    assert!(!valid_level_id(""));
    assert!(!valid_level_id("../100000001"));
    assert!(!valid_level_id("100000001a"));
    assert!(matches!(
        plugin.archive("../x"),
        Err(PluginFailure::InvalidInput)
    ));
    assert!(matches!(
        plugin.export_payload("../x", ExportFormat::Csv),
        Err(PluginFailure::InvalidInput)
    ));
    fs::remove_dir_all(temp).unwrap();
}
