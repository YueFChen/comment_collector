//! 归档导出：JSON（完整）/ CSV（带 BOM，Excel 直接双击可用）/ Excel（.xlsx）。
//!
//! 三种格式共用同一套列定义与取值，避免各写一份后互相漂移。
//! 中文一路按 UTF-8 处理：CSV 额外加 BOM，xlsx 本身就是 UTF-8 的 XML。

use std::fmt::Display;

use rust_xlsxwriter::{Color, Format, FormatBorder, Workbook};
use wonderland_plugin_sdk::PluginFailure;

use crate::model::{CommentArchive, CommentItem};

/// 导出表的列定义：CSV 与 Excel 共用。JSON 保留完整字段，不受此表限制。
const COLUMNS: [&str; 14] = [
    "楼层",
    "昵称",
    "UID",
    "是否作者",
    "是否推荐",
    "点赞数",
    "回复数",
    "发表时间",
    "IP归属地",
    "内容",
    "类型",
    "所属主评论",
    "回复对象",
    "评论ID",
];

/// 与 [`COLUMNS`] 一一对应的列宽。
const COLUMN_WIDTHS: [f64; 14] = [
    8.0, 18.0, 14.0, 10.0, 10.0, 8.0, 8.0, 20.0, 12.0, 80.0, 10.0, 16.0, 16.0, 20.0,
];

/// Excel 单个单元格的字符上限是 32767，这里留一点余量。
const EXCEL_CELL_LIMIT: usize = 32_000;

fn failure(error: impl Display) -> PluginFailure {
    PluginFailure::Other(format!("导出失败：{error}"))
}

fn yes_no(value: bool) -> String {
    if value { "是" } else { "否" }.to_owned()
}

/// Unix 秒 → 本地时间文案；时间戳异常时留空，不伪造 1970 年。
fn time(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|moment| {
            moment
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}

/// 一条评论 → 一行单元格。
fn row_of(item: &CommentItem) -> [String; 14] {
    [
        item.floor_id.clone(),
        item.nickname.clone(),
        item.uid.clone(),
        yes_no(item.is_owner),
        // 楼中楼官方不给此字段，留空而不是写「否」——那会把未知说成不推荐。
        item.is_recommend.map(yes_no).unwrap_or_default(),
        item.like_count.to_string(),
        item.reply_count.to_string(),
        time(item.created_at),
        item.ip_region.clone(),
        item.content.clone(),
        if item.is_sub {
            "楼中楼"
        } else {
            "主评论"
        }
        .to_owned(),
        item.parent_id.clone(),
        item.reply_to.clone(),
        item.reply_id.clone(),
    ]
}

/// RFC 4180：含分隔符、引号或换行的字段加引号，内部引号翻倍。
fn quote(value: &str) -> String {
    // 评论来自网络。Excel 会把以 =、+、- 或 @ 开头的 CSV 字段当公式执行；
    // 先加单引号，再按 CSV 规则转义，避免打开导出文件时触发公式。
    let formula_candidate = value.trim_start().starts_with(['=', '+', '-', '@']);
    let safe = if formula_candidate {
        format!("'{value}")
    } else {
        value.to_owned()
    };
    if safe.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", safe.replace('"', "\"\""))
    } else {
        safe
    }
}

/// CSV 正文：表头 + 全部行，行尾用 CRLF（Windows 上的 Excel 认这个）。
pub(crate) fn csv(archive: &CommentArchive) -> Vec<u8> {
    let mut text = String::new();
    text.push_str(&COLUMNS.join(","));
    text.push_str("\r\n");
    for item in &archive.comments {
        let cells: Vec<String> = row_of(item).iter().map(|cell| quote(cell)).collect();
        text.push_str(&cells.join(","));
        text.push_str("\r\n");
    }
    // BOM：不加的话 Excel 会把中文按本地代码页读成乱码。
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

/// JSON 完整归档：保留全部字段与类型，便于后续程序化分析。
pub(crate) fn json(archive: &CommentArchive) -> Result<Vec<u8>, PluginFailure> {
    serde_json::to_vec_pretty(archive).map_err(failure)
}

/// Excel 单元格：XML 不接受控制字符，且单元格有长度上限。
fn cell(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\t' | '\n' | '\r'))
        .take(EXCEL_CELL_LIMIT)
        .collect()
}

/// Excel：表头加粗、冻结首行、预置列宽。
pub(crate) fn excel(archive: &CommentArchive) -> Result<Vec<u8>, PluginFailure> {
    let mut workbook = Workbook::new();
    let header = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0xF1F1F5))
        .set_border(FormatBorder::Thin);
    let sheet = workbook.add_worksheet();
    sheet.set_name("评论").map_err(failure)?;
    for (index, column) in COLUMNS.iter().enumerate() {
        sheet
            .write_string_with_format(0, index as u16, *column, &header)
            .map_err(failure)?;
    }
    for (index, item) in archive.comments.iter().enumerate() {
        for (column, value) in row_of(item).iter().enumerate() {
            sheet
                .write_string(index as u32 + 1, column as u16, cell(value))
                .map_err(failure)?;
        }
    }
    for (index, width) in COLUMN_WIDTHS.iter().enumerate() {
        sheet
            .set_column_width(index as u16, *width)
            .map_err(failure)?;
    }
    sheet.set_freeze_panes(1, 0).map_err(failure)?;
    workbook.save_to_buffer().map_err(failure)
}

/// 关卡名可能带文件系统不允许的字符，替换后再限长。
pub(crate) fn slug(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if r#"\/:*?"<>|"#.contains(character) || character.is_control() {
                '_'
            } else {
                character
            }
        })
        .take(40)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "关卡".to_owned()
    } else {
        cleaned.to_owned()
    }
}

/// 导出文件名：关卡名 + 关卡 id + 本地时间，多次导出不会互相覆盖。
pub(crate) fn file_name(archive: &CommentArchive, extension: &str) -> String {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    format!(
        "{}_{}_{stamp}.{extension}",
        slug(&archive.level.level_name),
        archive.level.level_id
    )
}
