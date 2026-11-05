//! 输入校验和规整，规则见详细设计 9.2。
//!
//! 每个函数校验一个字段，通过就返回规整后的值（去首尾空格、统一大小写），
//! 不通过返回给用户看的中文说明，写明哪个字段、为什么。

use std::collections::HashSet;

use crate::Result;
use crate::error::invalid;
use crate::types::Attribute;

pub(crate) const NAME_MAX: usize = 200;
pub(crate) const CODE_MAX: usize = 64;
pub(crate) const SHORT_TEXT_MAX: usize = 100;
pub(crate) const DESCRIPTION_MAX: usize = 5000;
pub(crate) const NOTES_MAX: usize = 1000;
pub(crate) const TITLE_MAX: usize = 200;
pub(crate) const TEXT_ASSET_MAX: usize = 20_000;
pub(crate) const SELLING_POINTS_MAX: usize = 20;
pub(crate) const SELLING_POINT_MAX: usize = 200;
pub(crate) const ATTRIBUTES_MAX: usize = 100;
pub(crate) const ATTRIBUTE_NAME_MAX: usize = 64;
pub(crate) const ATTRIBUTE_VALUE_MAX: usize = 500;
pub(crate) const SKUS_MAX: usize = 200;
pub(crate) const SPEC_ITEMS_MAX: usize = 5;
pub(crate) const SPEC_NAME_MAX: usize = 32;
pub(crate) const SPEC_VALUE_MAX: usize = 64;
pub(crate) const TAGS_MAX: usize = 20;
pub(crate) const TAG_MAX: usize = 32;
const PLATFORM_MAX: usize = 32;

/// 单张图片的大小上限，和上传原图的限制一致（需求 FR-01）。
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// 分页：不传或传 0 时一页 50 条，最多 200 条。
pub(crate) const PAGE_DEFAULT: i64 = 50;
pub(crate) const PAGE_MAX: i64 = 200;

pub(crate) const ASSET_SOURCES: &[&str] = &["upload", "generated", "captured", "imported"];
pub(crate) const ASSET_KINDS: &[&str] = &["image", "text"];
pub(crate) const COPY_FIELDS: &[&str] = &[
    "title",
    "bullets",
    "description",
    "keywords",
    "attributes",
    "selling_points",
];

fn char_count(text: &str) -> usize {
    text.chars().count()
}

/// 必填文本：去首尾空格后不能为空，不超过 `max` 字。
pub(crate) fn required(field: &str, value: &str, max: usize) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid(format!("{field}不能为空")));
    }
    if char_count(value) > max {
        return Err(invalid(format!("{field}不能超过 {max} 字")));
    }
    Ok(value.to_owned())
}

/// 选填文本：空白当作没填。
pub(crate) fn optional(field: &str, value: Option<&str>, max: usize) -> Result<Option<String>> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => required(field, value, max).map(Some),
    }
}

/// 商品编码、SKU 编码：选填，不能含空白字符。
pub(crate) fn code(field: &str, value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = optional(field, value, CODE_MAX)? else {
        return Ok(None);
    };
    if value.chars().any(char::is_whitespace) {
        return Err(invalid(format!("{field}「{value}」不能包含空格")));
    }
    Ok(Some(value))
}

/// 平台标识：小写字母、数字、下划线。
pub(crate) fn platform(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    let valid = (1..=PLATFORM_MAX).contains(&normalized.len())
        && normalized
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if !valid {
        return Err(invalid(format!(
            "平台标识「{}」格式不对：只能用小写字母、数字和下划线，最多 {PLATFORM_MAX} 位",
            value.trim()
        )));
    }
    Ok(normalized)
}

/// 两位国家 / 地区代码，统一成大写。
pub(crate) fn region(field: &str, value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_uppercase();
    if normalized.len() != 2 || !normalized.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(invalid(format!(
            "{field}「{}」应该是两位国家或地区代码，如 US、DE",
            value.trim()
        )));
    }
    Ok(normalized)
}

pub(crate) fn optional_region(field: &str, value: Option<&str>) -> Result<Option<String>> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => region(field, value).map(Some),
    }
}

/// 目标市场：每项是地区代码，去重，保留第一次出现的顺序。
pub(crate) fn markets(values: &[String]) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut markets = Vec::new();
    for value in values.iter().filter(|value| !value.trim().is_empty()) {
        let market = region("目标市场", value)?;
        if seen.insert(market.clone()) {
            markets.push(market);
        }
    }
    Ok(markets)
}

/// 语言：`zh-CN`、`en-US`、`en` 这种格式，统一成"语言小写-地区大写"；下划线当作连字符。
pub(crate) fn language(value: &str) -> Result<String> {
    let trimmed = value.trim();
    let normalized = trimmed.replace('_', "-");
    let mut parts = normalized.split('-');
    let lang = parts.next().unwrap_or_default().to_ascii_lowercase();
    let region = parts.next().map(str::to_ascii_uppercase);
    let valid = (2..=3).contains(&lang.len())
        && lang.bytes().all(|b| b.is_ascii_lowercase())
        && region
            .as_deref()
            .is_none_or(|r| r.len() == 2 && r.bytes().all(|b| b.is_ascii_uppercase()))
        && parts.next().is_none();
    if !valid {
        return Err(invalid(format!(
            "语言「{trimmed}」格式不对，应该像 zh-CN、en-US、en 这样"
        )));
    }
    Ok(match region {
        Some(region) => format!("{lang}-{region}"),
        None => lang,
    })
}

/// 币种：三位字母，统一成大写。
pub(crate) fn currency(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_uppercase();
    if normalized.len() != 3 || !normalized.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(invalid(format!(
            "币种「{}」应该是三位字母代码，如 CNY、USD",
            value.trim()
        )));
    }
    Ok(normalized)
}

/// 金额（分）、库存：不能为负。
pub(crate) fn non_negative(field: &str, value: Option<i64>) -> Result<Option<i64>> {
    match value {
        Some(value) if value < 0 => Err(invalid(format!("{field}不能是负数"))),
        other => Ok(other),
    }
}

/// 一组文本（如卖点）：去掉空行，限制条数和每条字数。
pub(crate) fn lines(
    field: &str,
    values: &[String],
    max_items: usize,
    max_len: usize,
) -> Result<Vec<String>> {
    let lines: Vec<String> = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    if lines.len() > max_items {
        return Err(invalid(format!("{field}最多 {max_items} 条")));
    }
    if let Some(index) = lines.iter().position(|line| char_count(line) > max_len) {
        return Err(invalid(format!(
            "{field}第 {} 条超过 {max_len} 字",
            index + 1
        )));
    }
    Ok(lines)
}

/// 键值对的长度限制。
pub(crate) struct PairLimits {
    pub max_items: usize,
    pub name_max: usize,
    pub value_max: usize,
}

/// 一组键值对（商品参数、SKU 规格）：名和值都要填，名不能重复；名和值都空的行直接去掉。
pub(crate) fn pairs(
    field: &str,
    values: &[Attribute],
    limits: &PairLimits,
) -> Result<Vec<(String, String)>> {
    let mut seen = HashSet::new();
    let mut pairs = Vec::new();
    for item in values {
        let name = item.name.trim();
        let value = item.value.trim();
        if name.is_empty() && value.is_empty() {
            continue;
        }
        if name.is_empty() {
            return Err(invalid(format!("{field}「{value}」缺少名称")));
        }
        if value.is_empty() {
            return Err(invalid(format!("{field}「{name}」缺少值")));
        }
        if char_count(name) > limits.name_max {
            return Err(invalid(format!(
                "{field}名称「{name}」超过 {} 字",
                limits.name_max
            )));
        }
        if char_count(value) > limits.value_max {
            return Err(invalid(format!(
                "{field}「{name}」的值超过 {} 字",
                limits.value_max
            )));
        }
        if !seen.insert(name.to_owned()) {
            return Err(invalid(format!("{field}「{name}」重复了")));
        }
        pairs.push((name.to_owned(), value.to_owned()));
    }
    if pairs.len() > limits.max_items {
        return Err(invalid(format!("{field}最多 {} 项", limits.max_items)));
    }
    Ok(pairs)
}

/// 标签：中间多个空白合成一个，不区分大小写去重（保留第一次的写法），每个素材最多 20 个。
pub(crate) fn tags(values: &[String]) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut tags = Vec::new();
    for value in values {
        let tag = value.split_whitespace().collect::<Vec<_>>().join(" ");
        if tag.is_empty() {
            continue;
        }
        if char_count(&tag) > TAG_MAX {
            return Err(invalid(format!("标签「{tag}」超过 {TAG_MAX} 字")));
        }
        if seen.insert(tag.to_lowercase()) {
            tags.push(tag);
        }
    }
    if tags.len() > TAGS_MAX {
        return Err(invalid(format!("每个素材最多 {TAGS_MAX} 个标签")));
    }
    Ok(tags)
}

/// 只能是列表里的某个值。
pub(crate) fn one_of(field: &str, value: &str, allowed: &[&str]) -> Result<String> {
    let value = value.trim();
    if allowed.contains(&value) {
        Ok(value.to_owned())
    } else {
        Err(invalid(format!(
            "{field}「{value}」不支持，可选：{}",
            allowed.join(" / ")
        )))
    }
}

/// 分页参数：返回 `(limit, offset)`。
pub(crate) fn page(limit: i64, offset: i64) -> (i64, i64) {
    let limit = if limit <= 0 {
        PAGE_DEFAULT
    } else {
        limit.min(PAGE_MAX)
    };
    (limit, offset.max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(result: Result<impl std::fmt::Debug>) -> String {
        result.unwrap_err().to_string()
    }

    fn attr(name: &str, value: &str) -> Attribute {
        Attribute {
            name: name.into(),
            value: value.into(),
        }
    }

    const SPEC: PairLimits = PairLimits {
        max_items: SPEC_ITEMS_MAX,
        name_max: SPEC_NAME_MAX,
        value_max: SPEC_VALUE_MAX,
    };

    #[test]
    fn text_is_trimmed_and_limited() {
        assert_eq!(required("商品名称", "  T 恤 ", 10).unwrap(), "T 恤");
        assert_eq!(message(required("商品名称", "   ", 10)), "商品名称不能为空");
        // 按字符算，不按字节
        assert!(required("商品名称", &"长".repeat(10), 10).is_ok());
        assert_eq!(
            message(required("商品名称", &"长".repeat(11), 10)),
            "商品名称不能超过 10 字"
        );
        assert_eq!(optional("品牌", Some("  "), 10).unwrap(), None);
        assert_eq!(optional("品牌", None, 10).unwrap(), None);
    }

    #[test]
    fn codes_cannot_contain_spaces() {
        assert_eq!(
            code("商品编码", Some(" TS-001 ")).unwrap().as_deref(),
            Some("TS-001")
        );
        assert_eq!(code("商品编码", Some("")).unwrap(), None);
        assert_eq!(
            message(code("商品编码", Some("TS 001"))),
            "商品编码「TS 001」不能包含空格"
        );
    }

    #[test]
    fn codes_are_normalized() {
        assert_eq!(platform(" TikTok_Shop ").unwrap(), "tiktok_shop");
        assert!(platform("tiktok-shop").is_err());
        assert!(platform("").is_err());
        assert_eq!(region("站点", "us").unwrap(), "US");
        assert!(region("站点", "USA").is_err());
        assert_eq!(optional_region("站点", Some(" ")).unwrap(), None);
        assert_eq!(
            markets(&["us".into(), "DE".into(), "US".into(), " ".into()]).unwrap(),
            ["US", "DE"]
        );
        assert_eq!(currency("usd").unwrap(), "USD");
        assert!(currency("RMB1").is_err());
    }

    #[test]
    fn languages_are_normalized() {
        assert_eq!(language("zh-cn").unwrap(), "zh-CN");
        assert_eq!(language("EN_us").unwrap(), "en-US");
        assert_eq!(language("en").unwrap(), "en");
        assert_eq!(language("fil").unwrap(), "fil");
        for bad in ["", "english", "en-USA", "zh-CN-x", "1n"] {
            assert!(language(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn amounts_cannot_be_negative() {
        assert_eq!(non_negative("售价", Some(0)).unwrap(), Some(0));
        assert_eq!(non_negative("售价", None).unwrap(), None);
        assert_eq!(message(non_negative("售价", Some(-1))), "售价不能是负数");
    }

    #[test]
    fn lines_drop_blanks_and_point_at_the_long_one() {
        let values = vec![" 纯棉 ".into(), "".into(), "长".repeat(6)];
        assert_eq!(
            message(lines("卖点", &values, 5, 5)),
            "卖点第 2 条超过 5 字"
        );
        assert_eq!(lines("卖点", &values[..2], 5, 5).unwrap(), ["纯棉"]);
        assert_eq!(
            message(lines("卖点", &vec!["a".into(); 3], 2, 5)),
            "卖点最多 2 条"
        );
    }

    #[test]
    fn pairs_need_names_values_and_no_duplicates() {
        let ok = pairs(
            "规格",
            &[attr(" 颜色 ", "红"), attr("", ""), attr("尺码", "M")],
            &SPEC,
        )
        .unwrap();
        assert_eq!(
            ok,
            [
                ("颜色".to_owned(), "红".to_owned()),
                ("尺码".to_owned(), "M".to_owned())
            ]
        );
        assert_eq!(
            message(pairs("规格", &[attr("颜色", " ")], &SPEC)),
            "规格「颜色」缺少值"
        );
        assert_eq!(
            message(pairs("规格", &[attr("", "红")], &SPEC)),
            "规格「红」缺少名称"
        );
        assert_eq!(
            message(pairs(
                "规格",
                &[attr("颜色", "红"), attr("颜色", "蓝")],
                &SPEC
            )),
            "规格「颜色」重复了"
        );
        let many: Vec<Attribute> = (0..6).map(|i| attr(&format!("k{i}"), "v")).collect();
        assert_eq!(message(pairs("规格", &many, &SPEC)), "规格最多 5 项");
    }

    #[test]
    fn tags_are_collapsed_and_deduplicated() {
        let values = vec![
            " 白底  主图 ".into(),
            "Summer".into(),
            "summer".into(),
            " ".into(),
        ];
        assert_eq!(tags(&values).unwrap(), ["白底 主图", "Summer"]);
        let many: Vec<String> = (0..21).map(|i| format!("t{i}")).collect();
        assert_eq!(message(tags(&many)), "每个素材最多 20 个标签");
        assert!(tags(&["长".repeat(33)]).is_err());
    }

    #[test]
    fn enums_and_pages() {
        assert_eq!(one_of("来源", " upload ", ASSET_SOURCES).unwrap(), "upload");
        assert!(one_of("来源", "camera", ASSET_SOURCES).is_err());
        assert_eq!(page(0, -5), (PAGE_DEFAULT, 0));
        assert_eq!(page(1000, 10), (PAGE_MAX, 10));
    }
}
