//! 从表格导入商品（详细设计 9.5）：预览（找保存过的映射或自动对列）→ 确认映射 → 逐行写入。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use prism_core::now_ms;
use prism_io::{
    CellValue, FieldSpec, Mapping, ReadOptions, RowProblem, SAVED_MAPPING_MIN_OVERLAP, Table,
    TableRow, apply_mapping, auto_map, header_overlap, read_table,
};
use prism_store::NewImportMapping;
use serde::{Deserialize, Serialize};

use crate::{Catalog, CatalogError, Product, ProductInput, Result};

/// 保存映射时的导入目标。
const PRODUCT_TARGET: &str = "product";
/// 预览返回前几行。
const PREVIEW_ROWS: usize = 5;
/// 金额最大多少元，防止把条码之类的长数字当成金额。
const MONEY_MAX_UNITS: f64 = 1e12;
const CENTS_PER_UNIT: f64 = 100.0;

/// 可导入的商品字段（详细设计 9.5 的表）。
pub fn product_import_fields() -> Vec<FieldSpec> {
    vec![
        FieldSpec::new(
            "product.name",
            "商品名称",
            &[
                "名称",
                "标题",
                "品名",
                "商品标题",
                "Product Name",
                "Title",
                "Name",
            ],
            true,
        ),
        FieldSpec::new(
            "product.code",
            "货号",
            &["商品编码", "编码", "款号", "商家编码", "SKU", "Item Code"],
            false,
        ),
        FieldSpec::new("product.category", "类目", &["分类", "Category"], false),
        FieldSpec::new("product.brand", "品牌", &["Brand"], false),
        FieldSpec::new(
            "product.description",
            "商品说明",
            &["描述", "简介", "Description"],
            false,
        ),
        FieldSpec::new(
            "product.selling_points",
            "卖点",
            &["商品卖点", "Selling Points"],
            false,
        ),
        FieldSpec::new(
            "product.cost",
            "成本",
            &["采购价", "进价", "成本价", "Cost"],
            false,
        ),
        FieldSpec::new("product.currency", "币种", &["Currency"], false),
        FieldSpec::new(
            "product.target_markets",
            "目标市场",
            &["市场", "站点", "Markets"],
            false,
        ),
    ]
}

/// 保存过的列映射。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedMapping {
    pub id: String,
    pub name: String,
    pub headers: Vec<String>,
    pub mapping: Mapping,
    pub header_line: Option<usize>,
    pub use_count: i64,
    pub last_used_at: Option<i64>,
}

/// 预览里套用的保存过的映射。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchedMapping {
    pub id: String,
    pub name: String,
    /// 表头重合度，0～1
    pub overlap: f64,
}

/// 导入前的预览。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub sheet_names: Vec<String>,
    pub sheet: String,
    pub header_line: usize,
    pub headers: Vec<String>,
    /// 前 5 行
    pub sample: Vec<TableRow>,
    pub total_rows: usize,
    pub fields: Vec<FieldSpec>,
    /// 建议的映射：套用保存过的，或者自动对列
    pub mapping: Mapping,
    /// 套用了哪条保存过的映射
    pub matched: Option<MatchedMapping>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    /// 全部新建，货号和已有商品重复的行跳过
    CreateOnly,
    /// 货号相同时更新已有商品，只改表格里有对应列、且这一格不为空的字段
    #[default]
    Upsert,
}

/// 一次导入的参数。
#[derive(Debug, Clone, PartialEq)]
pub struct ImportRequest {
    pub path: PathBuf,
    pub read: ReadOptions,
    pub mapping: Mapping,
    pub mode: ImportMode,
    /// 不为空时把映射用这个名称存下来
    pub save_as: Option<String>,
    /// 套用的是哪条保存过的映射，导入后记一次使用
    pub matched_mapping_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub created: usize,
    pub updated: usize,
    /// 有问题、没导入的行数
    pub skipped: usize,
    pub problems: Vec<RowProblem>,
    /// 这次存下来的映射
    pub saved_mapping_id: Option<String>,
}

impl Catalog {
    /// 读表格，给出建议的映射。
    pub async fn preview_product_import(
        &self,
        path: &Path,
        read: &ReadOptions,
    ) -> Result<ImportPreview> {
        let table = read_in_background(path, read).await?;
        let fields = product_import_fields();
        let (mapping, matched) = match self.best_saved_mapping(&table.headers).await? {
            Some((saved, overlap)) => (
                saved.mapping.rebind(&table.headers),
                Some(MatchedMapping {
                    id: saved.id,
                    name: saved.name,
                    overlap,
                }),
            ),
            None => (auto_map(&table.headers, &fields), None),
        };
        Ok(ImportPreview {
            sheet_names: table.sheet_names,
            sheet: table.sheet,
            header_line: table.header_line,
            headers: table.headers,
            total_rows: table.rows.len(),
            sample: table.rows.into_iter().take(PREVIEW_ROWS).collect(),
            fields,
            mapping,
            matched,
        })
    }

    /// 保存过的商品映射里，表头重合度最高、且达到 0.8 的那条。重合度一样时用得多的优先（列表已经按这个排好）。
    async fn best_saved_mapping(&self, headers: &[String]) -> Result<Option<(SavedMapping, f64)>> {
        let mut best: Option<(SavedMapping, f64)> = None;
        for saved in self.list_import_mappings().await? {
            let overlap = header_overlap(&saved.headers, headers);
            if overlap >= SAVED_MAPPING_MIN_OVERLAP
                && best.as_ref().is_none_or(|(_, b)| overlap > *b)
            {
                best = Some((saved, overlap));
            }
        }
        Ok(best)
    }

    /// 按确认过的映射导入商品。某行出错只跳过这一行。
    pub async fn import_products(&self, request: &ImportRequest) -> Result<ImportResult> {
        let table = read_in_background(&request.path, &request.read).await?;
        let fields = product_import_fields();
        let mapped = apply_mapping(&table, &request.mapping, &fields)?;
        let labels: HashMap<&str, &str> = fields
            .iter()
            .map(|f| (f.key.as_str(), f.label.as_str()))
            .collect();

        let mut result = ImportResult {
            problems: mapped.problems,
            ..ImportResult::default()
        };
        let mut seen_codes: HashMap<String, usize> = HashMap::new();
        for row in &mapped.rows {
            let problem = |field: &str, message: String| RowProblem {
                line: row.line,
                field: labels.get(field).copied().unwrap_or(field).to_owned(),
                message,
            };
            let patch = match ProductPatch::from_values(&row.values) {
                Ok(patch) => patch,
                Err((field, message)) => {
                    result.problems.push(problem(field, message));
                    continue;
                }
            };
            if let Some(code) = &patch.code {
                if let Some(first) = seen_codes.get(code) {
                    result.problems.push(problem(
                        "product.code",
                        format!("货号「{code}」和第 {first} 行重复了"),
                    ));
                    continue;
                }
                seen_codes.insert(code.clone(), row.line);
            }
            match self.write_row(&patch, request.mode).await {
                Ok(Written::Created) => result.created += 1,
                Ok(Written::Updated) => result.updated += 1,
                Err(RowError::Problem(field, message)) => {
                    result.problems.push(problem(field, message))
                }
                Err(RowError::Fatal(error)) => return Err(error),
            }
        }
        result.problems.sort_by_key(|p| p.line);
        let mut lines: Vec<usize> = result.problems.iter().map(|p| p.line).collect();
        lines.dedup();
        result.skipped = lines.len();

        if let Some(name) = request
            .save_as
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            let saved = self
                .save_import_mapping(
                    name,
                    &table.headers,
                    &request.mapping,
                    request.read.header_line,
                )
                .await?;
            result.saved_mapping_id = Some(saved.id);
        } else if let Some(id) = &request.matched_mapping_id {
            self.store.mark_import_mapping_used(id, now_ms()).await?;
        }
        Ok(result)
    }

    async fn write_row(
        &self,
        patch: &ProductPatch,
        mode: ImportMode,
    ) -> std::result::Result<Written, RowError> {
        let existing = match &patch.code {
            Some(code) => self
                .store
                .find_product_by_code(code)
                .await
                .map_err(|e| RowError::Fatal(e.into()))?,
            None => None,
        };
        match (existing, mode) {
            (Some(_), ImportMode::CreateOnly) => Err(RowError::Problem(
                "product.code",
                format!(
                    "货号「{}」已经存在",
                    patch.code.as_deref().unwrap_or_default()
                ),
            )),
            (Some(record), ImportMode::Upsert) => {
                let id = record.id.clone();
                let mut input = ProductInput::from(&Product::from(record));
                patch.apply(&mut input);
                self.update_product(&id, &input)
                    .await
                    .map_err(RowError::from)?;
                Ok(Written::Updated)
            }
            (None, _) => {
                let mut input = ProductInput::default();
                patch.apply(&mut input);
                self.create_product(&input).await.map_err(RowError::from)?;
                Ok(Written::Created)
            }
        }
    }

    // ---------- 保存的映射 ----------

    /// 保存过的商品映射，用得多的在前。存储内容损坏的条目跳过并记日志。
    pub async fn list_import_mappings(&self) -> Result<Vec<SavedMapping>> {
        let records = self.store.list_import_mappings(PRODUCT_TARGET).await?;
        let mut mappings = Vec::with_capacity(records.len());
        for record in records {
            let headers = serde_json::from_str(&record.headers);
            let columns = serde_json::from_str(&record.columns);
            match (headers, columns) {
                (Ok(headers), Ok(columns)) => mappings.push(SavedMapping {
                    id: record.id,
                    name: record.name,
                    headers,
                    mapping: Mapping { columns },
                    header_line: record.header_line.and_then(|l| usize::try_from(l).ok()),
                    use_count: record.use_count,
                    last_used_at: record.last_used_at,
                }),
                _ => tracing::warn!(id = %record.id, "保存的列映射内容损坏，已跳过"),
            }
        }
        Ok(mappings)
    }

    /// 保存映射；表头和已有的一条完全相同时覆盖那条。
    pub async fn save_import_mapping(
        &self,
        name: &str,
        headers: &[String],
        mapping: &Mapping,
        header_line: Option<usize>,
    ) -> Result<SavedMapping> {
        let name = crate::validate::required("映射名称", name, crate::validate::NAME_MAX)?;
        let record = self
            .store
            .save_import_mapping(
                &NewImportMapping {
                    target: PRODUCT_TARGET.into(),
                    name,
                    headers: to_json(&headers),
                    columns: to_json(&mapping.columns),
                    header_line: header_line.and_then(|l| i64::try_from(l).ok()),
                },
                now_ms(),
            )
            .await?;
        Ok(SavedMapping {
            id: record.id,
            name: record.name,
            headers: headers.to_vec(),
            mapping: mapping.clone(),
            header_line,
            use_count: record.use_count,
            last_used_at: record.last_used_at,
        })
    }

    pub async fn delete_import_mapping(&self, id: &str) -> Result<()> {
        Ok(self.store.delete_import_mapping(id).await?)
    }
}

/// 序列化成 JSON 文本。这里只用于字符串列表和字符串映射，不会失败。
fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// 读表格是同步的文件操作，放到阻塞线程池里，不卡住异步运行时。
async fn read_in_background(path: &Path, read: &ReadOptions) -> Result<Table> {
    let path = path.to_owned();
    let read = read.clone();
    tokio::task::spawn_blocking(move || read_table(&path, &read))
        .await
        .map_err(|error| CatalogError::Invalid(format!("读取表格时出错：{error}")))?
        .map_err(CatalogError::from)
}

enum Written {
    Created,
    Updated,
}

/// 一行的错误：可以跳过这行的问题，或者要中止整个导入的错误（比如数据库出错）。
enum RowError {
    Problem(&'static str, String),
    Fatal(CatalogError),
}

impl From<CatalogError> for RowError {
    fn from(error: CatalogError) -> Self {
        match error {
            CatalogError::Invalid(message) => Self::Problem("", message),
            CatalogError::Store(prism_store::StoreError::Duplicate { .. }) => {
                Self::Problem("product.code", error.to_string())
            }
            other => Self::Fatal(other),
        }
    }
}

/// 一行表格里取到的商品字段，`None` 表示这一格没有（不改原值）。
#[derive(Debug, Default)]
struct ProductPatch {
    name: Option<String>,
    code: Option<String>,
    category: Option<String>,
    brand: Option<String>,
    description: Option<String>,
    selling_points: Option<Vec<String>>,
    cost_cents: Option<i64>,
    currency: Option<String>,
    target_markets: Option<Vec<String>>,
}

impl ProductPatch {
    /// 出错时返回 (字段键, 原因)。
    fn from_values(
        values: &std::collections::BTreeMap<String, CellValue>,
    ) -> std::result::Result<Self, (&'static str, String)> {
        let text = |key: &str| values.get(key).and_then(CellValue::as_text);
        let cost_cents = match values.get("product.cost") {
            Some(cell) => Some(parse_money(cell).ok_or_else(|| {
                (
                    "product.cost",
                    format!("「{}」不是金额", cell.as_text().unwrap_or_default()),
                )
            })?),
            None => None,
        };
        Ok(Self {
            name: text("product.name"),
            code: text("product.code").map(|code| code.trim().to_owned()),
            category: text("product.category"),
            brand: text("product.brand"),
            description: text("product.description"),
            selling_points: text("product.selling_points").map(|t| split(&t, &['\n', ';', '；'])),
            cost_cents,
            currency: text("product.currency"),
            target_markets: text("product.target_markets")
                .map(|t| split(&t, &[',', '，', ';', '；', ' ', '/', '、'])),
        })
    }

    fn apply(&self, input: &mut ProductInput) {
        if let Some(name) = &self.name {
            input.name.clone_from(name);
        }
        if self.code.is_some() {
            input.code.clone_from(&self.code);
        }
        for (target, value) in [
            (&mut input.category, &self.category),
            (&mut input.brand, &self.brand),
            (&mut input.description, &self.description),
        ] {
            if value.is_some() {
                target.clone_from(value);
            }
        }
        if let Some(points) = &self.selling_points {
            input.selling_points.clone_from(points);
        }
        if self.cost_cents.is_some() {
            input.cost_cents = self.cost_cents;
        }
        if let Some(currency) = &self.currency {
            input.currency.clone_from(currency);
        }
        if let Some(markets) = &self.target_markets {
            input.target_markets.clone_from(markets);
        }
    }
}

fn split(text: &str, separators: &[char]) -> Vec<String> {
    text.split(separators)
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

/// 元转成分。文本里的货币符号、千分位逗号、"元"去掉；不是数字或大得离谱的返回 `None`。
fn parse_money(cell: &CellValue) -> Option<i64> {
    let units = match cell {
        CellValue::Number(number) => *number,
        CellValue::Text(text) => {
            let cleaned: String = text
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                .collect();
            if !cleaned.bytes().any(|b| b.is_ascii_digit()) {
                return None;
            }
            cleaned.parse().ok()?
        }
        CellValue::Empty | CellValue::Bool(_) => return None,
    };
    if !units.is_finite() || units.abs() > MONEY_MAX_UNITS {
        return None;
    }
    // 范围已经限制在 1e14 分以内，转 i64 不会溢出
    Some((units * CENTS_PER_UNIT).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_is_parsed_from_numbers_and_text() {
        assert_eq!(parse_money(&CellValue::Number(23.5)), Some(2350));
        assert_eq!(
            parse_money(&CellValue::Text("¥1,299.90元".into())),
            Some(129_990)
        );
        assert_eq!(parse_money(&CellValue::Text("0.1".into())), Some(10));
        assert_eq!(parse_money(&CellValue::Text("待定".into())), None);
        assert_eq!(
            parse_money(&CellValue::Number(6_901_234_567_892_000.0)),
            None
        );
    }

    #[test]
    fn lists_are_split() {
        assert_eq!(
            split("US, DE；JP/  ", &[',', '；', '/']),
            ["US", "DE", "JP"]
        );
    }
}
