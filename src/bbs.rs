//! 米游社匿名社区接口（`bbs-api.miyoushe.com`）的接入层。
//!
//! 这里的结构只描述**官方响应**，与 [`crate::model`] 的对外 DTO 分开：
//! 官方改字段时只需改这一层，本地归档与前端绑定不受影响。
//! 接口全程匿名，不需要账号凭据。

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

use async_trait::async_trait;
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use wonderland_plugin_sdk::PluginFailure;

use crate::model::{CommentItem, LevelInfo};

/// 匿名社区接口根路径。
const BASE: &str = "https://bbs-api.miyoushe.com/community/ugc_community/web/api";
/// 官方固定分区；关卡 id 在全区唯一。
const REGION: &str = "cn_gf01";
/// 每页条数。
///
/// 20 是官方**硬上限**，不是我们的选择：实测 `size` 给 5 / 10 会被尊重，给 50 / 100 也只回 20。
/// 所以调大没有收益，提速只能靠压缩 `lib.rs` 里的翻页间隔。
const PAGE_SIZE: u32 = 20;
/// Full scans start with the original HOT query and follow server phases;
/// incremental scans start with descending floors. See docs/API-RESEARCH.md.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplySort {
    Hot,
    FloorDesc,
    FloorAsc,
}
impl ReplySort {
    fn wire(self) -> &'static str {
        match self {
            Self::Hot => "SORT_TYPE_HOT",
            Self::FloorDesc => "SORT_TYPE_FLOOR_DESC",
            Self::FloorAsc => "SORT_TYPE_FLOOR_ASC",
        }
    }
}
/// 单次采集的页数上限，防止官方 `has_more` 异常时无限翻页。
pub(crate) const MAX_PAGES: u32 = 500;

/// 官方接口对 UA 敏感，沿用旧版实测可用的移动端 UA，不用客户端默认的桌面 UA。
const UA: &str = "Mozilla/5.0 (Linux; Android 6.0; Nexus 5 Build/MRA58N) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Mobile Safari/537.36";

fn headers() -> [(&'static str, &'static str); 5] {
    [
        ("user-agent", UA),
        ("origin", "https://act.miyoushe.com"),
        ("referer", "https://act.miyoushe.com/"),
        ("x-rpc-client_type", "5"),
        ("x-rpc-language", "zh-cn"),
    ]
}

/// 官方同一响应里数字与字符串混用：`like_count` / `hot_score` 是字符串，
/// `floor_id` / `created_at` 是数字，同一字段在不同关卡上也可能变。
///
/// 因此这里只按「标量」解析，不对单个字段的 JSON 类型下断言。
#[derive(Debug, Clone, Default)]
pub(crate) struct Scalar(String);

impl Scalar {
    pub(crate) fn text(&self) -> &str {
        self.0.trim()
    }

    pub(crate) fn u32(&self) -> u32 {
        self.number()
            .map(|value| value.max(0.0) as u32)
            .unwrap_or(0)
    }

    pub(crate) fn i64(&self) -> i64 {
        self.number().map(|value| value as i64).unwrap_or(0)
    }

    fn number(&self) -> Option<f64> {
        self.text()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
    }
}

impl<'de> Deserialize<'de> for Scalar {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(match Value::deserialize(deserializer)? {
            Value::String(text) => text,
            Value::Number(number) => number.to_string(),
            Value::Bool(flag) => flag.to_string(),
            _ => String::new(),
        }))
    }
}

/// 解官方外壳：`retcode` 非 0 一律是业务错误，把官方文案带进错误里。
pub(crate) fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, PluginFailure> {
    let code = value
        .get("retcode")
        .and_then(Value::as_i64)
        .ok_or(PluginFailure::InvalidResponse)?;
    if code != 0 {
        let message = value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        return Err(PluginFailure::Business { code, message });
    }
    let data = value
        .get("data")
        .filter(|data| !data.is_null())
        .ok_or(PluginFailure::InvalidResponse)?;
    serde_json::from_value(data.clone()).map_err(|_| PluginFailure::InvalidResponse)
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => text == "true" || text == "1",
        Some(Value::Number(number)) => number.as_i64().unwrap_or(0) != 0,
        _ => false,
    }
}

#[derive(Deserialize)]
pub(crate) struct LevelDetailData {
    level_info: LevelWire,
}

impl LevelDetailData {
    pub(crate) fn into_level(self) -> LevelInfo {
        self.level_info.into()
    }
}

#[derive(Deserialize)]
struct LevelWire {
    #[serde(default)]
    level_id: Scalar,
    #[serde(default)]
    level_name: Scalar,
    #[serde(default)]
    desc: Scalar,
    #[serde(default)]
    hot_score: Scalar,
    #[serde(default)]
    good_rate: Scalar,
    #[serde(default)]
    play_type: Scalar,
    #[serde(default)]
    show_limit_play_num_str: Scalar,
    #[serde(default)]
    cover_img: Option<ImageWire>,
    #[serde(default)]
    images: Vec<ImageWire>,
}

#[derive(Deserialize)]
struct ImageWire {
    #[serde(default)]
    url: Scalar,
}

impl From<LevelWire> for LevelInfo {
    fn from(wire: LevelWire) -> Self {
        Self {
            level_id: wire.level_id.text().to_owned(),
            level_name: wire.level_name.text().to_owned(),
            cover_url: wire
                .cover_img
                .map(|image| image.url.text().to_owned())
                .unwrap_or_default(),
            hot_score: wire.hot_score.text().to_owned(),
            good_rate: wire.good_rate.text().to_owned(),
            play_type: wire.play_type.text().to_owned(),
            play_range: wire.show_limit_play_num_str.text().to_owned(),
            desc: wire.desc.text().to_owned(),
            images: wire
                .images
                .into_iter()
                .map(|image| image.url.text().to_owned())
                .filter(|url| !url.is_empty())
                .collect(),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct ReplyListData {
    reply_list: Vec<ReplyWire>,
    /// 官方游标：结构不透明，按原样带回下一次请求。
    #[serde(default)]
    cursor: Option<Value>,
}

impl ReplyListData {
    /// 本页展开后的评论（主评论在前，其楼中楼紧随其后）。
    pub(crate) fn items(&self) -> Vec<CommentItem> {
        let mut out = Vec::new();
        for reply in &self.reply_list {
            expand(&mut out, reply, "", false);
        }
        out
    }

    pub(crate) fn items_as_sub(&self, parent_id: &str) -> Vec<CommentItem> {
        let mut out = Vec::new();
        for reply in &self.reply_list {
            expand(&mut out, reply, parent_id, true);
        }
        out
    }

    pub(crate) fn validate(&self) -> Result<(), PluginFailure> {
        let flag = self.cursor.as_ref().and_then(|c| c.get("has_more"));
        let valid_flag = matches!(flag, Some(Value::Bool(_)))
            || matches!(flag, Some(Value::String(s)) if ["true", "false", "0", "1"].contains(&s.as_str()))
            || matches!(flag, Some(Value::Number(n)) if n.as_u64().is_some_and(|n| n <= 1));
        fn valid_reply(reply: &ReplyWire) -> bool {
            !reply.reply_id.text().is_empty() && reply.sub_replies.iter().all(valid_reply)
        }
        if !valid_flag || !self.reply_list.iter().all(valid_reply) {
            return Err(PluginFailure::InvalidResponse);
        }
        Ok(())
    }

    /// 是否还有下一页，以及下一页游标里的 `next`。
    pub(crate) fn more(&self) -> bool {
        truthy(
            self.cursor
                .as_ref()
                .and_then(|cursor| cursor.get("has_more")),
        )
    }

    pub(crate) fn next(&self) -> &str {
        self.cursor
            .as_ref()
            .and_then(|cursor| cursor.get("next"))
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    pub(crate) fn cursor(&self) -> Option<&Value> {
        self.cursor.as_ref()
    }
}

#[derive(Deserialize)]
struct ReplyWire {
    #[serde(default)]
    reply_id: Scalar,
    #[serde(default)]
    floor_id: Scalar,
    #[serde(default)]
    content: Scalar,
    #[serde(default)]
    created_at: Scalar,
    #[serde(default)]
    client_ip: Scalar,
    /// 官方只给主评论此字段，楼中楼缺省。
    #[serde(default)]
    is_recommend: Option<bool>,
    #[serde(default)]
    is_owner: bool,
    #[serde(default)]
    user_info: Option<UserWire>,
    #[serde(default)]
    reply_stat: Option<ReplyStatWire>,
    /// 楼中楼所回复的对象。
    #[serde(default)]
    r_user: Option<UserWire>,
    #[serde(default)]
    sub_replies: Vec<ReplyWire>,
}

#[derive(Deserialize)]
struct UserWire {
    #[serde(default)]
    uid: Scalar,
    #[serde(default)]
    nickname: Scalar,
    #[serde(default)]
    avatar: Scalar,
}

#[derive(Deserialize)]
struct ReplyStatWire {
    #[serde(default)]
    like_count: Scalar,
    #[serde(default)]
    reply_count: Scalar,
}

fn expand(out: &mut Vec<CommentItem>, reply: &ReplyWire, parent_id: &str, is_sub: bool) {
    let user = reply.user_info.as_ref();
    let stat = reply.reply_stat.as_ref();
    let reply_id = reply.reply_id.text().to_owned();
    // 官方必然给出 id；缺失说明结构变了。跳过而不是塞一行空 id——
    // 空 id 会在归档去重时互相覆盖，静默吃掉数据。
    if reply_id.is_empty() {
        return;
    }
    out.push(CommentItem {
        floor_id: if is_sub {
            String::new()
        } else {
            reply.floor_id.text().to_owned()
        },
        uid: user
            .map(|user| user.uid.text().to_owned())
            .unwrap_or_default(),
        nickname: user
            .map(|user| user.nickname.text().to_owned())
            .unwrap_or_default(),
        avatar_url: user
            .map(|user| user.avatar.text().to_owned())
            .unwrap_or_default(),
        content: reply.content.text().to_owned(),
        is_recommend: if is_sub { None } else { reply.is_recommend },
        is_owner: reply.is_owner,
        like_count: stat.map(|stat| stat.like_count.u32()).unwrap_or(0),
        reply_count: stat.map(|stat| stat.reply_count.u32()).unwrap_or(0),
        created_at: reply.created_at.i64(),
        ip_region: reply.client_ip.text().to_owned(),
        is_sub,
        parent_id: parent_id.to_owned(),
        reply_to: if is_sub {
            reply
                .r_user
                .as_ref()
                .map(|user| user.nickname.text().to_owned())
                .unwrap_or_default()
        } else {
            String::new()
        },
        reply_id: reply_id.clone(),
    });
    for sub in &reply.sub_replies {
        expand(out, sub, &reply_id, true);
    }
}

/// 匿名社区客户端。
#[async_trait]
pub trait PublicHttpClient: Send + Sync {
    async fn get_bytes(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        query: &[(String, String)],
    ) -> Result<Vec<u8>, PluginFailure>;
    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Result<Vec<u8>, PluginFailure>;
}

pub struct Bbs {
    net: Arc<dyn PublicHttpClient>,
    // Shared by every level, metadata request and retry, not a per-worker sleep.
    next_request: Mutex<Instant>,
    request_interval_ms: AtomicU64,
}

impl Bbs {
    pub fn new(net: Arc<dyn PublicHttpClient>) -> Self {
        Self {
            net,
            next_request: Mutex::new(Instant::now()),
            request_interval_ms: AtomicU64::new(500),
        }
    }

    pub(crate) fn set_request_interval(&self, millis: u64) {
        self.request_interval_ms
            .store(millis.clamp(250, 60_000), Ordering::Relaxed);
    }

    async fn throttle(&self, cancel: &AtomicBool) -> Result<(), PluginFailure> {
        let mut next = self.next_request.lock().await;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(crate::cancelled());
            }
            let now = Instant::now();
            if now >= *next {
                break;
            }
            tokio::time::sleep_until((*next).min(now + Duration::from_millis(100))).await;
        }
        *next = Instant::now()
            + Duration::from_millis(self.request_interval_ms.load(Ordering::Relaxed));
        Ok(())
    }

    pub(crate) async fn cooldown(&self, duration: Duration) {
        let mut next = self.next_request.lock().await;
        *next = (*next).max(Instant::now() + duration);
    }

    /// 关卡详情。
    pub(crate) async fn level(
        &self,
        level_id: &str,
        cancel: &AtomicBool,
    ) -> Result<LevelInfo, PluginFailure> {
        let url =
            format!("{BASE}/level/detail?level_id={level_id}&uid=&region={REGION}&lang=zh-cn");
        self.throttle(cancel).await?;
        let bytes = self.net.get_bytes(&url, &headers(), &[]).await?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| PluginFailure::InvalidResponse)?;
        let data: LevelDetailData = decode(value)?;
        Ok(data.into_level())
    }

    /// 取一页评论。`cursor` 为 `None` 时从第一页开始。
    pub(crate) async fn page(
        &self,
        level_id: &str,
        cursor: Option<&Value>,
        sort: ReplySort,
        cancel: &AtomicBool,
    ) -> Result<ReplyListData, PluginFailure> {
        self.request_page(level_id, cursor, sort, None, cancel)
            .await
    }

    pub(crate) async fn sub_page(
        &self,
        level_id: &str,
        parent_id: &str,
        cursor: Option<&Value>,
        cancel: &AtomicBool,
    ) -> Result<ReplyListData, PluginFailure> {
        self.request_page(
            level_id,
            cursor,
            ReplySort::FloorAsc,
            Some(parent_id),
            cancel,
        )
        .await
    }

    async fn request_page(
        &self,
        level_id: &str,
        cursor: Option<&Value>,
        sort: ReplySort,
        parent_id: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<ReplyListData, PluginFailure> {
        let path = if parent_id.is_some() {
            "level/reply/sub_replies"
        } else {
            "reply/list"
        };
        let url = format!("{BASE}/{path}?lang=zh-cn");
        let mut cursor = cursor.cloned().unwrap_or_else(|| json!({ "next": "" }));
        let object = cursor
            .as_object_mut()
            .ok_or(PluginFailure::InvalidResponse)?;
        // Preserve opaque server fields, while retaining size and order omitted by responses.
        object.insert("size".into(), json!(PAGE_SIZE));
        let request_sort = object
            .entry("sort_type")
            .or_insert_with(|| json!(sort.wire()))
            .as_str()
            .ok_or(PluginFailure::InvalidResponse)?
            .to_owned();
        // Full main scans start HOT and may enter the server's descending-floor
        // phase. Incremental main scans and ASC child scans retain their order.
        if request_sort != sort.wire()
            && !(parent_id.is_none()
                && sort == ReplySort::Hot
                && request_sort == ReplySort::FloorDesc.wire())
        {
            return Err(PluginFailure::InvalidResponse);
        }
        let mut body = json!({
            "uid": "",
            "region": REGION,
            "level_id": level_id,
            "cursor": cursor,
        });
        if let Some(parent_id) = parent_id {
            body["parent_reply_id"] = json!(parent_id);
        }
        self.throttle(cancel).await?;
        let bytes = self
            .net
            .post_json(&url, &headers(), &body.to_string())
            .await?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| PluginFailure::InvalidResponse)?;
        let mut page: ReplyListData = decode(value)?;
        let response_cursor = page
            .cursor
            .as_mut()
            .and_then(Value::as_object_mut)
            .ok_or(PluginFailure::InvalidResponse)?;
        let response_sort = response_cursor
            .entry("sort_type")
            .or_insert_with(|| json!(request_sort))
            .as_str()
            .ok_or(PluginFailure::InvalidResponse)?;
        // This is a normal main-list phase transition, not a malformed response.
        // Replay its cursor as v0.1.1 and the official frontend do; never replace
        // a floor cursor with HOT or allow the reverse/unknown transitions.
        if response_sort != request_sort
            && !(parent_id.is_none()
                && sort == ReplySort::Hot
                && request_sort == ReplySort::Hot.wire()
                && response_sort == ReplySort::FloorDesc.wire())
        {
            return Err(PluginFailure::InvalidResponse);
        }
        Ok(page)
    }
}
