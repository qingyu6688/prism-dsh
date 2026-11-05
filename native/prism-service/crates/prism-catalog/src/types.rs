//! 商品库对外的数据类型。字段名序列化成 camelCase，直接给前端用。

use prism_store::{
    AssetRecord, CopyRecord, ProductRecord, ProductSummaryRecord, ShopRecord, SkuRecord,
};
use serde::{Deserialize, Serialize};

use crate::pairs;

/// 一项参数或规格，如 颜色 = 红。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribute {
    pub name: String,
    pub value: String,
}

impl Attribute {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// 分页结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    /// 同样条件下的总条数
    pub total: i64,
}

// ---------- 店铺 ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShopInput {
    pub name: String,
    /// 平台标识，如 `amazon`、`temu`、`taobao`
    pub platform: String,
    /// 站点，如 `US`；国内平台不填
    pub site: Option<String>,
    pub language: String,
    pub currency: String,
    pub notes: Option<String>,
}

impl Default for ShopInput {
    fn default() -> Self {
        Self {
            name: String::new(),
            platform: String::new(),
            site: None,
            language: "zh-CN".into(),
            currency: "CNY".into(),
            notes: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shop {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub site: Option<String>,
    pub language: String,
    pub currency: String,
    pub notes: Option<String>,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<ShopRecord> for Shop {
    fn from(record: ShopRecord) -> Self {
        Self {
            id: record.id,
            name: record.name,
            platform: record.platform,
            site: record.site,
            language: record.language,
            currency: record.currency,
            notes: record.notes,
            sort_order: record.sort_order,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

// ---------- 商品 ----------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProductInput {
    /// 货号，填了就必须唯一
    pub code: Option<String>,
    pub name: String,
    pub category: Option<String>,
    pub brand: Option<String>,
    pub description: Option<String>,
    pub selling_points: Vec<String>,
    pub attributes: Vec<Attribute>,
    /// 成本（分）
    pub cost_cents: Option<i64>,
    pub currency: String,
    /// 目标市场，两位地区代码
    pub target_markets: Vec<String>,
}

impl Default for ProductInput {
    fn default() -> Self {
        Self {
            code: None,
            name: String::new(),
            category: None,
            brand: None,
            description: None,
            selling_points: Vec::new(),
            attributes: Vec::new(),
            cost_cents: None,
            currency: "CNY".into(),
            target_markets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Product {
    pub id: String,
    pub code: Option<String>,
    pub name: String,
    pub category: Option<String>,
    pub brand: Option<String>,
    pub description: Option<String>,
    pub selling_points: Vec<String>,
    pub attributes: Vec<Attribute>,
    pub cost_cents: Option<i64>,
    pub currency: String,
    pub target_markets: Vec<String>,
    pub archived: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<ProductRecord> for Product {
    fn from(record: ProductRecord) -> Self {
        let owner = record.id.as_str();
        Self {
            selling_points: json_list(owner, "selling_points", record.selling_points.as_deref()),
            attributes: json_pairs(owner, "attributes", record.attributes.as_deref()),
            target_markets: json_list(owner, "target_markets", record.target_markets.as_deref()),
            archived: record.status == "archived",
            id: record.id,
            code: record.code,
            name: record.name,
            category: record.category,
            brand: record.brand,
            description: record.description,
            cost_cents: record.cost_cents,
            currency: record.currency,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

impl From<&Product> for ProductInput {
    /// 已有商品转成输入，导入时在它上面改，没改到的字段保持原样。
    fn from(product: &Product) -> Self {
        Self {
            code: product.code.clone(),
            name: product.name.clone(),
            category: product.category.clone(),
            brand: product.brand.clone(),
            description: product.description.clone(),
            selling_points: product.selling_points.clone(),
            attributes: product.attributes.clone(),
            cost_cents: product.cost_cents,
            currency: product.currency.clone(),
            target_markets: product.target_markets.clone(),
        }
    }
}

/// 商品列表的筛选条件。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProductFilter {
    /// 在名称、货号、类目、品牌里搜索
    pub keyword: Option<String>,
    /// `true` 只看已归档的，`false` 只看在用的
    pub archived: bool,
    /// 每页条数，0 表示默认 50，最多 200
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SkuInput {
    pub code: Option<String>,
    /// 规格组合，如 颜色 = 红、尺码 = M
    pub spec: Vec<Attribute>,
    pub price_cents: Option<i64>,
    pub cost_cents: Option<i64>,
    pub stock: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sku {
    pub id: String,
    pub code: Option<String>,
    pub spec: Vec<Attribute>,
    pub price_cents: Option<i64>,
    pub cost_cents: Option<i64>,
    pub stock: Option<i64>,
    pub sort_order: i64,
}

impl From<SkuRecord> for Sku {
    fn from(record: SkuRecord) -> Self {
        Self {
            spec: json_pairs(&record.id, "spec", Some(&record.spec)),
            id: record.id,
            code: record.code,
            price_cents: record.price_cents,
            cost_cents: record.cost_cents,
            stock: record.stock,
            sort_order: record.sort_order,
        }
    }
}

/// 某个语言的文案条数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyCount {
    pub language: String,
    pub total: i64,
    pub approved: i64,
}

/// 某个模块的批次数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchCount {
    pub module: String,
    pub count: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductSummary {
    pub sku_count: i64,
    pub image_assets: i64,
    pub text_assets: i64,
    pub starred_assets: i64,
    pub copies: Vec<CopyCount>,
    pub batches: Vec<BatchCount>,
    pub last_batch_at: Option<i64>,
}

impl From<ProductSummaryRecord> for ProductSummary {
    fn from(record: ProductSummaryRecord) -> Self {
        Self {
            sku_count: record.sku_count,
            image_assets: record.image_assets,
            text_assets: record.text_assets,
            starred_assets: record.starred_assets,
            copies: record
                .copies_by_language
                .into_iter()
                .map(|(language, total, approved)| CopyCount {
                    language,
                    total,
                    approved,
                })
                .collect(),
            batches: record
                .batches_by_module
                .into_iter()
                .map(|(module, count)| BatchCount { module, count })
                .collect(),
            last_batch_at: record.last_batch_at,
        }
    }
}

/// 商品详情：商品、规格、汇总。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductDetail {
    pub product: Product,
    pub skus: Vec<Sku>,
    pub summary: ProductSummary,
}

// ---------- 素材 ----------

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: String,
    /// `image` / `text`
    pub kind: String,
    pub product_id: Option<String>,
    /// `upload` / `generated` / `captured` / `imported`
    pub source: String,
    pub title: Option<String>,
    /// 相对数据目录的路径；完整路径用 `Catalog::asset_path`
    pub file_path: Option<String>,
    pub thumb_path: Option<String>,
    pub text_content: Option<String>,
    pub mime: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub sha256: Option<String>,
    /// 感知哈希（16 位十六进制），找相似图用
    pub phash: Option<String>,
    pub origin_batch_id: Option<String>,
    pub origin_job_id: Option<String>,
    pub starred: bool,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Asset {
    pub(crate) fn from_record(record: AssetRecord, tags: Vec<String>) -> Self {
        Self {
            id: record.id,
            kind: record.kind,
            product_id: record.product_id,
            source: record.source,
            title: record.title,
            file_path: record.file_path,
            thumb_path: record.thumb_path,
            text_content: record.text_content,
            mime: record.mime,
            width: record.width,
            height: record.height,
            sha256: record.sha256,
            phash: record.phash,
            origin_batch_id: record.origin_batch_id,
            origin_job_id: record.origin_job_id,
            starred: record.starred,
            tags,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

/// 导入图片素材的选项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImportOptions {
    pub product_id: Option<String>,
    pub source: String,
    /// 不填时用文件名（不含扩展名）
    pub title: Option<String>,
    pub tags: Vec<String>,
    /// 生成的图片记下来自哪个批次、哪个任务
    pub origin_batch_id: Option<String>,
    pub origin_job_id: Option<String>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            product_id: None,
            source: "upload".into(),
            title: None,
            tags: Vec::new(),
            origin_batch_id: None,
            origin_job_id: None,
        }
    }
}

/// 导入结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedAsset {
    pub asset: Asset,
    /// 库里已有同样内容的图片，返回的是已有的那条
    pub deduplicated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TextAssetInput {
    pub product_id: Option<String>,
    pub source: String,
    pub title: Option<String>,
    pub content: String,
    pub tags: Vec<String>,
}

impl Default for TextAssetInput {
    fn default() -> Self {
        Self {
            product_id: None,
            source: "upload".into(),
            title: None,
            content: String::new(),
            tags: Vec::new(),
        }
    }
}

/// 素材列表的筛选条件。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AssetFilter {
    pub product_id: Option<String>,
    pub kind: Option<String>,
    pub source: Option<String>,
    pub tag: Option<String>,
    /// 在标题、文案内容、标签里搜索
    pub keyword: Option<String>,
    pub starred_only: bool,
    pub limit: i64,
    pub offset: i64,
}

// ---------- 文案 ----------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CopyInput {
    pub product_id: String,
    /// 不填表示通用文案，不针对某个店铺
    pub shop_id: Option<String>,
    pub language: String,
    /// `title` / `bullets` / `description` / `keywords` / `attributes` / `selling_points`
    pub field: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Copy {
    pub id: String,
    pub product_id: String,
    pub shop_id: Option<String>,
    pub language: String,
    pub field: String,
    pub content: String,
    pub approved: bool,
    pub origin_job_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<CopyRecord> for Copy {
    fn from(record: CopyRecord) -> Self {
        Self {
            approved: record.status == "approved",
            id: record.id,
            product_id: record.product_id,
            shop_id: record.shop_id,
            language: record.language,
            field: record.field,
            content: record.content,
            origin_job_id: record.origin_job_id,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

// ---------- JSON 字段 ----------

/// 库里的 JSON 数组。解析失败（手工改坏）按空处理并记日志，不让整页读不出来。
fn json_list(owner: &str, field: &str, text: Option<&str>) -> Vec<String> {
    let Some(text) = text else {
        return Vec::new();
    };
    serde_json::from_str(text).unwrap_or_else(|error| {
        tracing::warn!(owner, field, "JSON 字段解析失败，按空值处理：{error}");
        Vec::new()
    })
}

fn json_pairs(owner: &str, field: &str, text: Option<&str>) -> Vec<Attribute> {
    let Some(text) = text else {
        return Vec::new();
    };
    match pairs::from_json(text) {
        Ok(pairs) => pairs
            .into_iter()
            .map(|(name, value)| Attribute { name, value })
            .collect(),
        Err(error) => {
            tracing::warn!(owner, field, "JSON 字段解析失败，按空值处理：{error}");
            Vec::new()
        }
    }
}

/// 写库用：空列表存 NULL。
pub(crate) fn list_to_json(values: &[String]) -> Option<String> {
    if values.is_empty() {
        None
    } else {
        serde_json::to_string(values).ok()
    }
}

pub(crate) fn pairs_to_json(values: &[(String, String)]) -> Option<String> {
    if values.is_empty() {
        None
    } else {
        Some(pairs::to_json(values))
    }
}

/// 导入时显示用的文件名（不含扩展名）。
pub(crate) fn file_title(path: &std::path::Path) -> Option<String> {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.trim().is_empty())
}
