//! 米游社匿名社区接口（`bbs-api.miyoushe.com`）的接入层。
//!
//! 这里的结构只描述**官方响应**，与 [`crate::model`] 的对外 DTO 分开：
//! 官方改字段时只需改这一层，本地归档与前端绑定不受影响。
//! 接口全程匿名，不需要账号凭据。

use std::sync::Arc;

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
/// 排序方式**必须用热度**：实测按楼层降序只返回部分评论（243 条里少 35 条）。
const SORT_TYPE: &str = "SORT_TYPE_HOT";
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
    #[serde(default)]
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
}

impl Bbs {
    pub fn new(net: Arc<dyn PublicHttpClient>) -> Self {
        Self { net }
    }

    /// 关卡详情。
    pub(crate) async fn level(&self, level_id: &str) -> Result<LevelInfo, PluginFailure> {
        let url =
            format!("{BASE}/level/detail?level_id={level_id}&uid=&region={REGION}&lang=zh-cn");
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
    ) -> Result<ReplyListData, PluginFailure> {
        let url = format!("{BASE}/reply/list?lang=zh-cn");
        let cursor = cursor
            .cloned()
            .unwrap_or_else(|| json!({ "next": "", "size": PAGE_SIZE, "sort_type": SORT_TYPE }));
        let body = json!({
            "uid": "",
            "region": REGION,
            "level_id": level_id,
            "cursor": cursor,
        });
        let bytes = self
            .net
            .post_json(&url, &headers(), &body.to_string())
            .await?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| PluginFailure::InvalidResponse)?;
        decode(value)
    }
}
