//! 规则包和词表的数据结构（详细设计 10.2～10.4）。
//!
//! 所有结构都拒绝不认识的字段：字段名拼错时报错，而不是让那条规则悄悄失效。
//! JSON Schema 由这些类型生成（`rules/schema/`），改字段后跑一次
//! `PRISM_UPDATE_SCHEMA=1 cargo test -p prism-rules` 更新。

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 当前支持的文件格式版本。
pub const FORMAT_VERSION: u32 = 1;

/// 文件类型。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    RulePack,
    WordList,
    ExportTemplate,
}

impl ResourceKind {
    /// 内置和用户目录里存放这类文件的子目录名。
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::RulePack => "packs",
            Self::WordList => "words",
            Self::ExportTemplate => "exports",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::RulePack => "规则包",
            Self::WordList => "词表",
            Self::ExportTemplate => "导出模板",
        }
    }
}

/// 多语言文本，键是语言（`zh-CN`、`en`），至少要有中文和英文。
pub type LocalizedText = BTreeMap<String, String>;

/// 是否经过真实后台核对。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// 根据公开资料整理，未经真实后台核对
    Draft,
    /// 已在真实后台核对
    Verified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Market {
    CrossBorder,
    Domestic,
}

/// 长度单位（详细设计 10.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    /// 字符数
    Char,
    /// UTF-8 字节数
    Byte,
    /// 中日韩字符算 2，其他算 1
    CjkDouble,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WhiteBackground {
    #[default]
    None,
    /// 首图必须白底
    First,
    /// 全部白底
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImageFileFormat {
    Jpeg,
    Png,
    Webp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// 必须改
    Error,
    /// 提示风险
    Warning,
}

/// 词的匹配方式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    /// 子串匹配，用于中文
    #[default]
    Substring,
    /// 整词匹配、不区分大小写，用于英文等有空格分词的语言
    Word,
}

// ---------- 规则包 ----------

/// 一个平台的规则包。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RulePack {
    /// 指向 JSON Schema，只给编辑器用
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub kind: ResourceKind,
    pub format_version: u32,
    /// 平台标识，和店铺的 `platform` 一致
    pub id: String,
    /// 修订号，每次修改加 1
    pub revision: u32,
    pub name: LocalizedText,
    pub status: Status,
    /// 最后修改日期 `YYYY-MM-DD`
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,

    pub market: Market,
    /// 站点，两位地区代码；国内平台为空
    #[serde(default)]
    pub sites: Vec<String>,
    pub default_language: String,
    pub images: ImageRules,
    #[serde(default)]
    pub text: TextRules,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_attributes: Vec<RequiredAttribute>,
    #[serde(default)]
    pub banned_words: BannedWords,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageRules {
    /// 主图 / 轮播图
    pub main: ImageSpec,
    /// 详情图
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<ImageSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageSpec {
    #[serde(default = "one")]
    pub min_count: u32,
    pub max_count: u32,
    /// 较长边的最小像素
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_side: Option<u32>,
    /// 较长边的最大像素
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_side: Option<u32>,
    /// 允许的宽高比，如 `1:1`、`3:4`；空表示不限
    #[serde(default)]
    pub aspect_ratios: Vec<String>,
    pub formats: Vec<ImageFileFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    #[serde(default)]
    pub white_background: WhiteBackground,
    /// 商品占画面比例下限（0～1）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_product_ratio: Option<f64>,
}

fn one() -> u32 {
    1
}

/// 各文案字段的限制，不写表示没有限制。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextRules {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<TextLimit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bullets: Option<BulletRules>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<TextLimit>,
    /// 搜索关键词（如 Amazon 后台搜索词）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords: Option<TextLimit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextLimit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u32>,
    pub max_length: u32,
    pub unit: LengthUnit,
    /// 不允许出现的字符，逐个字符算
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forbidden_chars: Option<String>,
    /// 同一个词最多出现几次
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_word_repeat: Option<u32>,
}

/// 五点描述这类分条文案。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BulletRules {
    pub count: u32,
    pub max_length: u32,
    pub unit: LengthUnit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequiredAttribute {
    pub key: String,
    pub name: LocalizedText,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BannedWords {
    /// 引用的词表 ID
    #[serde(default)]
    pub lists: Vec<String>,
    /// 下面 `words` 的匹配方式
    #[serde(default, rename = "match")]
    pub match_mode: MatchMode,
    /// 平台独有的词
    #[serde(default)]
    pub words: Vec<WordEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WordEntry {
    pub word: String,
    pub severity: Severity,
    /// 替换建议
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

// ---------- 词表 ----------

/// 多个平台共用的词表，如广告法极限词。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WordList {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub kind: ResourceKind,
    pub format_version: u32,
    pub id: String,
    pub revision: u32,
    pub name: LocalizedText,
    pub status: Status,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,

    /// 词的语言，如 `zh`、`en`
    pub language: String,
    #[serde(rename = "match")]
    pub match_mode: MatchMode,
    pub words: Vec<WordEntry>,
}

// ---------- 导出模板 ----------

/// 一张刊登表格的样子（详细设计 10.7）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportTemplate {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub kind: ResourceKind,
    pub format_version: u32,
    pub id: String,
    pub revision: u32,
    pub name: LocalizedText,
    pub status: Status,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,

    /// 适用的平台，设置页分组用
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// 工作表名称
    pub sheet_name: String,
    /// 表头上面的固定行
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preamble: Vec<Vec<String>>,
    /// 每个商品一行还是每个规格一行，由业务模块据此组织数据
    pub row_per: RowPer,
    pub columns: Vec<ExportColumn>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RowPer {
    Product,
    Sku,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportColumn {
    pub header: String,
    /// `const:固定值`，或 `product`、`sku`、`copy`、`image`、`shop` 开头的路径，如 `image.main[0]`
    pub source: String,
    #[serde(default)]
    pub format: CellFormat,
    #[serde(default)]
    pub required: bool,
    /// 取不到值时用的默认值
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// 数据是列表时的连接符，默认 `; `
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// 单元格格式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CellFormat {
    /// 按文本写，保留前导 0
    #[default]
    Text,
    Integer,
    Decimal,
    /// 数据是"分"，写成两位小数
    Money,
    Boolean,
}

/// `source` 允许的根。
pub const SOURCE_ROOTS: &[&str] = &["product", "sku", "copy", "image", "shop"];

/// 解析出来的一个文件。
#[derive(Debug, Clone, PartialEq)]
pub enum Resource {
    // 规则包比词表大得多，装箱避免每个 Resource 都占规则包那么大的空间
    RulePack(Box<RulePack>),
    WordList(WordList),
    ExportTemplate(ExportTemplate),
}

impl Resource {
    pub fn kind(&self) -> ResourceKind {
        match self {
            Self::RulePack(_) => ResourceKind::RulePack,
            Self::WordList(_) => ResourceKind::WordList,
            Self::ExportTemplate(_) => ResourceKind::ExportTemplate,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::RulePack(pack) => &pack.id,
            Self::WordList(list) => &list.id,
            Self::ExportTemplate(template) => &template.id,
        }
    }

    pub fn revision(&self) -> u32 {
        match self {
            Self::RulePack(pack) => pack.revision,
            Self::WordList(list) => list.revision,
            Self::ExportTemplate(template) => template.revision,
        }
    }
}
