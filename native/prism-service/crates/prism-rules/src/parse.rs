//! 解析和校验（详细设计 10.5）：先查格式，再查内容，所有问题一次列全。

use std::collections::HashSet;
use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::model::{
    BulletRules, ExportColumn, ExportTemplate, FORMAT_VERSION, ImageSpec, LocalizedText, Market,
    Resource, ResourceKind, RulePack, SOURCE_ROOTS, TextLimit, WordEntry, WordList,
};

/// 一条校验问题。`path` 是 JSON 路径，如 `images.main.maxCount`，空表示整个文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub path: String,
    pub message: String,
}

impl Problem {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{}：{}", self.path, self.message)
        }
    }
}

/// 解析并校验一个文件。成功返回内容，失败返回全部问题。
///
/// 不检查对其他文件的引用（规则包引用的词表），那一步在注册表里做。
pub fn validate(bytes: &[u8]) -> Result<Resource, Vec<Problem>> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        vec![Problem::new(
            "",
            format!(
                "不是有效的 JSON：第 {} 行第 {} 列附近有错误",
                error.line(),
                error.column()
            ),
        )]
    })?;
    let kind = read_kind(&value)?;
    check_format_version(&value)?;
    match kind {
        ResourceKind::RulePack => {
            let pack: RulePack = strict(value)?;
            let problems = check_rule_pack(&pack);
            if problems.is_empty() {
                Ok(Resource::RulePack(Box::new(pack)))
            } else {
                Err(problems)
            }
        }
        ResourceKind::WordList => {
            let list: WordList = strict(value)?;
            let problems = check_word_list(&list);
            if problems.is_empty() {
                Ok(Resource::WordList(list))
            } else {
                Err(problems)
            }
        }
        ResourceKind::ExportTemplate => {
            let template: ExportTemplate = strict(value)?;
            let problems = check_export_template(&template);
            if problems.is_empty() {
                Ok(Resource::ExportTemplate(template))
            } else {
                Err(problems)
            }
        }
    }
}

const KIND_OPTIONS: &str = "rule_pack / word_list / export_template";

fn read_kind(value: &Value) -> Result<ResourceKind, Vec<Problem>> {
    let Some(object) = value.as_object() else {
        return Err(vec![Problem::new("", "文件内容应该是一个 JSON 对象")]);
    };
    match object.get("kind").and_then(Value::as_str) {
        Some("rule_pack") => Ok(ResourceKind::RulePack),
        Some("word_list") => Ok(ResourceKind::WordList),
        Some("export_template") => Ok(ResourceKind::ExportTemplate),
        Some(other) => Err(vec![Problem::new(
            "kind",
            format!("不支持的文件类型 {other}，可选：{KIND_OPTIONS}"),
        )]),
        None => Err(vec![Problem::new(
            "kind",
            format!("缺少必填字段 kind（文件类型：{KIND_OPTIONS}）"),
        )]),
    }
}

/// 格式版本比程序新时，后面的字段多半也不认识，直接提示升级，不再逐条报错。
fn check_format_version(value: &Value) -> Result<(), Vec<Problem>> {
    match value.get("formatVersion").and_then(Value::as_u64) {
        Some(version) if version > u64::from(FORMAT_VERSION) => Err(vec![Problem::new(
            "formatVersion",
            format!("文件格式版本 {version} 比应用支持的 {FORMAT_VERSION} 新，请升级应用"),
        )]),
        _ => Ok(()),
    }
}

fn strict<T: DeserializeOwned>(value: Value) -> Result<T, Vec<Problem>> {
    serde_path_to_error::deserialize(value).map_err(|error| {
        let path = error.path().to_string();
        // 路径为 "." 表示出错的是整个对象（比如缺字段）
        let path = if path == "." { String::new() } else { path };
        vec![Problem::new(path, translate(&error.inner().to_string()))]
    })
}

/// 把 serde 的英文报错翻成中文，认不出的保留原文。
fn translate(message: &str) -> String {
    if let Some(rest) = message.strip_prefix("unknown field `") {
        let field = rest.split('`').next().unwrap_or_default();
        return format!("不认识的字段 {field}，请检查拼写");
    }
    if let Some(rest) = message.strip_prefix("missing field `") {
        let field = rest.split('`').next().unwrap_or_default();
        return format!("缺少必填字段 {field}");
    }
    if let Some(rest) = message.strip_prefix("unknown variant `") {
        let value = rest.split('`').next().unwrap_or_default();
        let options: Vec<&str> = rest
            .split("expected ")
            .nth(1)
            .unwrap_or_default()
            .split('`')
            .skip(1)
            .step_by(2)
            .collect();
        return format!("不支持的值 {value}，可选：{}", options.join(" / "));
    }
    if message.starts_with("invalid type") || message.starts_with("invalid value") {
        let expected = message.split("expected ").nth(1).unwrap_or_default();
        return format!("类型不对，应该是{}", describe(expected));
    }
    message.to_owned()
}

fn describe(expected: &str) -> &'static str {
    match expected {
        "u32" | "u64" => "非负整数",
        "f64" => "数字",
        "a string" => "文本",
        "a sequence" => "数组",
        "a map" => "对象",
        "a boolean" => "true 或 false",
        _ if expected.starts_with("struct") => "对象",
        _ => "其他类型",
    }
}

// ---------- 内容检查 ----------

#[derive(Default)]
struct Checker {
    problems: Vec<Problem>,
}

impl Checker {
    fn add(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.problems.push(Problem::new(path, message));
    }

    fn require(&mut self, ok: bool, path: impl Into<String>, message: impl Into<String>) {
        if !ok {
            self.add(path, message);
        }
    }
}

struct Header<'a> {
    expected_kind: ResourceKind,
    kind: ResourceKind,
    format_version: u32,
    id: &'a str,
    revision: u32,
    name: &'a LocalizedText,
    updated_at: &'a str,
    sources: &'a [String],
}

fn check_header(c: &mut Checker, header: &Header<'_>) {
    c.require(
        header.kind == header.expected_kind,
        "kind",
        format!("文件类型应该是 {:?}", header.expected_kind),
    );
    c.require(
        header.format_version == FORMAT_VERSION,
        "formatVersion",
        format!("目前只支持 {FORMAT_VERSION}"),
    );
    c.require(
        is_identifier(header.id),
        "id",
        "只能用小写字母、数字和下划线，1～32 位",
    );
    c.require(header.revision >= 1, "revision", "修订号从 1 开始");
    check_localized(c, "name", header.name);
    c.require(
        is_date(header.updated_at),
        "updatedAt",
        format!("「{}」不是有效日期，格式是 YYYY-MM-DD", header.updated_at),
    );
    for (index, source) in header.sources.iter().enumerate() {
        c.require(
            source.starts_with("https://") || source.starts_with("http://"),
            format!("sources[{index}]"),
            "应该是 http(s) 链接",
        );
    }
}

fn check_localized(c: &mut Checker, path: &str, text: &LocalizedText) {
    for language in ["zh-CN", "en"] {
        let ok = text
            .get(language)
            .is_some_and(|value| !value.trim().is_empty());
        c.require(ok, path, format!("缺少 {language} 的文字"));
    }
}

pub(crate) fn check_rule_pack(pack: &RulePack) -> Vec<Problem> {
    let mut c = Checker::default();
    check_header(
        &mut c,
        &Header {
            expected_kind: ResourceKind::RulePack,
            kind: pack.kind,
            format_version: pack.format_version,
            id: &pack.id,
            revision: pack.revision,
            name: &pack.name,
            updated_at: &pack.updated_at,
            sources: &pack.sources,
        },
    );

    match pack.market {
        Market::CrossBorder => c.require(!pack.sites.is_empty(), "sites", "跨境平台至少填一个站点"),
        Market::Domestic => c.require(pack.sites.is_empty(), "sites", "国内平台不填站点"),
    }
    let mut seen = HashSet::new();
    for (index, site) in pack.sites.iter().enumerate() {
        let path = format!("sites[{index}]");
        c.require(
            is_region(site),
            &path,
            format!("「{site}」应该是两位大写地区代码，如 US"),
        );
        c.require(seen.insert(site), &path, format!("「{site}」重复了"));
    }
    c.require(
        is_language(&pack.default_language),
        "defaultLanguage",
        format!(
            "「{}」格式不对，应该像 zh-CN、en-US、en 这样",
            pack.default_language
        ),
    );

    check_image_spec(&mut c, "images.main", &pack.images.main, 1);
    if let Some(detail) = &pack.images.detail {
        check_image_spec(&mut c, "images.detail", detail, 0);
    }

    let text = &pack.text;
    for (name, limit) in [
        ("title", &text.title),
        ("description", &text.description),
        ("keywords", &text.keywords),
    ] {
        if let Some(limit) = limit {
            check_text_limit(&mut c, &format!("text.{name}"), limit);
        }
    }
    if let Some(bullets) = &text.bullets {
        check_bullets(&mut c, bullets);
    }

    let mut keys = HashSet::new();
    for (index, attribute) in pack.required_attributes.iter().enumerate() {
        let path = format!("requiredAttributes[{index}]");
        c.require(
            is_identifier(&attribute.key),
            format!("{path}.key"),
            "只能用小写字母、数字和下划线，1～32 位",
        );
        c.require(
            keys.insert(attribute.key.as_str()),
            format!("{path}.key"),
            format!("「{}」重复了", attribute.key),
        );
        check_localized(&mut c, &format!("{path}.name"), &attribute.name);
    }

    let mut lists = HashSet::new();
    for (index, list) in pack.banned_words.lists.iter().enumerate() {
        let path = format!("bannedWords.lists[{index}]");
        c.require(is_identifier(list), &path, "词表 ID 格式不对");
        c.require(lists.insert(list), &path, format!("「{list}」重复了"));
    }
    check_words(&mut c, "bannedWords.words", &pack.banned_words.words);
    c.problems
}

fn check_image_spec(c: &mut Checker, path: &str, spec: &ImageSpec, min_count_floor: u32) {
    c.require(
        spec.min_count >= min_count_floor,
        format!("{path}.minCount"),
        format!("不能小于 {min_count_floor}"),
    );
    c.require(spec.max_count >= 1, format!("{path}.maxCount"), "至少为 1");
    c.require(
        spec.min_count <= spec.max_count,
        format!("{path}.maxCount"),
        format!("不能小于 minCount（{}）", spec.min_count),
    );
    for (name, side) in [("minSide", spec.min_side), ("maxSide", spec.max_side)] {
        c.require(side != Some(0), format!("{path}.{name}"), "应该大于 0");
    }
    if let (Some(min), Some(max)) = (spec.min_side, spec.max_side) {
        c.require(
            min <= max,
            format!("{path}.maxSide"),
            format!("不能小于 minSide（{min}）"),
        );
    }
    let mut ratios = HashSet::new();
    for (index, ratio) in spec.aspect_ratios.iter().enumerate() {
        let item = format!("{path}.aspectRatios[{index}]");
        c.require(
            parse_ratio(ratio).is_some(),
            &item,
            format!("「{ratio}」应该写成 宽:高，如 1:1、3:4"),
        );
        c.require(ratios.insert(ratio), &item, format!("「{ratio}」重复了"));
    }
    c.require(
        !spec.formats.is_empty(),
        format!("{path}.formats"),
        "至少允许一种图片格式",
    );
    let unique: HashSet<_> = spec.formats.iter().collect();
    c.require(
        unique.len() == spec.formats.len(),
        format!("{path}.formats"),
        "有重复的格式",
    );
    c.require(
        spec.max_bytes != Some(0),
        format!("{path}.maxBytes"),
        "应该大于 0",
    );
    if let Some(ratio) = spec.min_product_ratio {
        c.require(
            ratio > 0.0 && ratio <= 1.0,
            format!("{path}.minProductRatio"),
            "应该在 0～1 之间，如 0.85",
        );
    }
}

fn check_text_limit(c: &mut Checker, path: &str, limit: &TextLimit) {
    c.require(
        limit.max_length >= 1,
        format!("{path}.maxLength"),
        "应该大于 0",
    );
    if let Some(min) = limit.min_length {
        c.require(
            min <= limit.max_length,
            format!("{path}.minLength"),
            format!("不能大于 maxLength（{}）", limit.max_length),
        );
    }
    if let Some(chars) = &limit.forbidden_chars {
        c.require(
            !chars.is_empty(),
            format!("{path}.forbiddenChars"),
            "不能是空字符串，不需要就删掉这个字段",
        );
    }
    c.require(
        limit.max_word_repeat != Some(0),
        format!("{path}.maxWordRepeat"),
        "至少为 1",
    );
}

fn check_bullets(c: &mut Checker, bullets: &BulletRules) {
    c.require(bullets.count >= 1, "text.bullets.count", "至少为 1");
    c.require(
        bullets.max_length >= 1,
        "text.bullets.maxLength",
        "应该大于 0",
    );
}

fn check_words(c: &mut Checker, path: &str, words: &[WordEntry]) {
    let mut seen = HashSet::new();
    for (index, entry) in words.iter().enumerate() {
        let item = format!("{path}[{index}].word");
        let word = entry.word.trim();
        if word.is_empty() {
            c.add(item, "词不能为空");
            continue;
        }
        c.require(
            word == entry.word,
            &item,
            format!("「{}」首尾有空格", entry.word),
        );
        c.require(
            seen.insert(word.to_lowercase()),
            &item,
            format!("「{word}」重复了"),
        );
    }
}

pub(crate) fn check_word_list(list: &WordList) -> Vec<Problem> {
    let mut c = Checker::default();
    check_header(
        &mut c,
        &Header {
            expected_kind: ResourceKind::WordList,
            kind: list.kind,
            format_version: list.format_version,
            id: &list.id,
            revision: list.revision,
            name: &list.name,
            updated_at: &list.updated_at,
            sources: &list.sources,
        },
    );
    c.require(
        is_language(&list.language),
        "language",
        format!("「{}」格式不对，应该像 zh、en、zh-CN 这样", list.language),
    );
    c.require(!list.words.is_empty(), "words", "词表不能是空的");
    check_words(&mut c, "words", &list.words);
    c.problems
}

/// Excel 工作表名称的限制：1～31 个字符，不能有 `[]:*?/\`。
const SHEET_NAME_MAX: usize = 31;
const SHEET_NAME_FORBIDDEN: &[char] = &['[', ']', ':', '*', '?', '/', '\\'];

pub(crate) fn check_export_template(template: &ExportTemplate) -> Vec<Problem> {
    let mut c = Checker::default();
    check_header(
        &mut c,
        &Header {
            expected_kind: ResourceKind::ExportTemplate,
            kind: template.kind,
            format_version: template.format_version,
            id: &template.id,
            revision: template.revision,
            name: &template.name,
            updated_at: &template.updated_at,
            sources: &template.sources,
        },
    );
    if let Some(platform) = &template.platform {
        c.require(is_identifier(platform), "platform", "平台标识格式不对");
    }
    let sheet = template.sheet_name.trim();
    c.require(
        !sheet.is_empty()
            && sheet.chars().count() <= SHEET_NAME_MAX
            && !sheet.contains(SHEET_NAME_FORBIDDEN),
        "sheetName",
        format!("工作表名称要 1～{SHEET_NAME_MAX} 个字，不能含 [ ] : * ? / \\"),
    );
    c.require(!template.columns.is_empty(), "columns", "至少要有一列");
    for (index, row) in template.preamble.iter().enumerate() {
        c.require(
            row.len() <= template.columns.len(),
            format!("preamble[{index}]"),
            format!("不能比列数（{}）多", template.columns.len()),
        );
    }
    let mut headers = HashSet::new();
    for (index, column) in template.columns.iter().enumerate() {
        check_column(&mut c, &format!("columns[{index}]"), column, &mut headers);
    }
    c.problems
}

fn check_column<'a>(
    c: &mut Checker,
    path: &str,
    column: &'a ExportColumn,
    headers: &mut HashSet<&'a str>,
) {
    let header = column.header.trim();
    if header.is_empty() {
        c.add(format!("{path}.header"), "表头不能为空");
    } else {
        c.require(
            headers.insert(header),
            format!("{path}.header"),
            format!("「{header}」重复了"),
        );
    }
    if let Err(message) = check_source(&column.source) {
        c.add(format!("{path}.source"), message);
    }
    if let Some(separator) = &column.separator {
        c.require(
            !separator.is_empty(),
            format!("{path}.separator"),
            "不能是空字符串，不需要就删掉这个字段",
        );
    }
}

/// 数据来源：`const:固定值`，或 `根.路径`，路径每段不为空，可以带 `[序号]`。
fn check_source(source: &str) -> Result<(), String> {
    if source.starts_with("const:") {
        return Ok(());
    }
    let mut segments = source.split('.');
    let root = segments.next().unwrap_or_default();
    if !SOURCE_ROOTS.contains(&root) {
        return Err(format!(
            "不认识的数据来源「{source}」：应该以 {} 开头，或者写成 const:固定值",
            SOURCE_ROOTS.join(" / ")
        ));
    }
    let rest: Vec<&str> = segments.collect();
    if rest.is_empty() {
        return Err(format!("「{source}」只有根，缺少具体字段，如 {root}.name"));
    }
    for segment in rest {
        let name = match segment.split_once('[') {
            Some((name, index)) => {
                let digits = index.strip_suffix(']').unwrap_or_default();
                if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(format!("「{source}」里的序号要写成 [0]、[1] 这样"));
                }
                name
            }
            None => segment,
        };
        if name.trim().is_empty() || name.trim() != name {
            return Err(format!("「{source}」里有空的或首尾带空格的字段名"));
        }
    }
    Ok(())
}

// ---------- 格式判断 ----------

/// 小写字母、数字、下划线，1～32 位。
pub(crate) fn is_identifier(value: &str) -> bool {
    (1..=32).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn is_region(value: &str) -> bool {
    value.len() == 2 && value.bytes().all(|b| b.is_ascii_uppercase())
}

/// `zh`、`fil`、`zh-CN` 这种格式。
fn is_language(value: &str) -> bool {
    let mut parts = value.split('-');
    let language = parts.next().unwrap_or_default();
    let region = parts.next();
    (2..=3).contains(&language.len())
        && language.bytes().all(|b| b.is_ascii_lowercase())
        && region.is_none_or(is_region)
        && parts.next().is_none()
}

fn parse_ratio(value: &str) -> Option<(u32, u32)> {
    let (width, height) = value.split_once(':')?;
    let width: u32 = width.parse().ok()?;
    let height: u32 = height.parse().ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

/// `YYYY-MM-DD`，检查月份天数和闰年。
fn is_date(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return false;
    };
    if year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        year.parse::<u32>(),
        month.parse::<u32>(),
        day.parse::<u32>(),
    ) else {
        return false;
    };
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_messages_are_translated() {
        assert_eq!(
            translate("unknown field `maxCont`, expected one of `minCount`, `maxCount`"),
            "不认识的字段 maxCont，请检查拼写"
        );
        assert_eq!(translate("missing field `market`"), "缺少必填字段 market");
        assert_eq!(
            translate("unknown variant `square`, expected one of `none`, `first`, `all`"),
            "不支持的值 square，可选：none / first / all"
        );
        assert_eq!(
            translate("invalid value: integer `-1`, expected u32"),
            "类型不对，应该是非负整数"
        );
        assert_eq!(
            translate("invalid type: string \"9\", expected u32"),
            "类型不对，应该是非负整数"
        );
        assert_eq!(translate("something else"), "something else");
    }

    #[test]
    fn export_sources_are_checked() {
        for ok in [
            "const:New",
            "const:",
            "product.name",
            "image.main[0]",
            "product.attributes.材质",
            "copy.bullets",
        ] {
            assert!(check_source(ok).is_ok(), "{ok}");
        }
        assert!(
            check_source("item.name")
                .unwrap_err()
                .starts_with("不认识的数据来源")
        );
        assert!(
            check_source("product")
                .unwrap_err()
                .contains("缺少具体字段")
        );
        assert!(check_source("image.main[x]").unwrap_err().contains("序号"));
        assert!(check_source("product..name").is_err());
    }

    #[test]
    fn formats_are_recognized() {
        assert!(is_identifier("tiktok_shop"));
        assert!(!is_identifier("TikTok"));
        assert!(!is_identifier(""));
        assert!(is_language("zh-CN") && is_language("en") && is_language("fil"));
        assert!(!is_language("zh-cn") && !is_language("english"));
        assert_eq!(parse_ratio("3:4"), Some((3, 4)));
        assert_eq!(parse_ratio("0:1"), None);
        assert_eq!(parse_ratio("1x1"), None);
        assert!(is_date("2024-02-29"));
        assert!(!is_date("2026-02-29"));
        assert!(!is_date("2026-13-01"));
        assert!(!is_date("2026-1-01"));
    }
}
