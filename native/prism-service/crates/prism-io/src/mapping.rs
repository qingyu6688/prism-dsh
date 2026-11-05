//! 列映射（详细设计 11.2）：自动对列、认出保存过的映射、套用映射逐行取值。

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::table::{CellValue, Table};
use crate::{IoError, Result};

/// 两组表头的重合度达到这个值，就直接套用保存过的映射。
pub const SAVED_MAPPING_MIN_OVERLAP: f64 = 0.8;

/// 自动对列时，"表头包含别名"至少要这么多个字符才算，避免"名"这种单字乱匹配。
const MIN_CONTAINS_LEN: usize = 2;

/// 要导入的一个字段，由调用方给出。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldSpec {
    /// 字段键，如 `product.name`
    pub key: String,
    /// 显示名，如"商品名称"
    pub label: String,
    /// 表头的其他常见写法，如 "标题"、"Title"、"Product Name"
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub required: bool,
}

impl FieldSpec {
    pub fn new(key: &str, label: &str, aliases: &[&str], required: bool) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
            required,
        }
    }

    fn names(&self) -> impl Iterator<Item = String> + '_ {
        std::iter::once(normalize(&self.label)).chain(self.aliases.iter().map(|a| normalize(a)))
    }
}

/// 字段键 → 表头。保存映射时存这个和当时的表头。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mapping {
    pub columns: BTreeMap<String, String>,
}

impl Mapping {
    /// 把保存过的映射套到新表格上：表头写法有出入（空格、全角、大小写）的对回新表头的写法，
    /// 新表格里没有的列去掉。
    pub fn rebind(&self, headers: &[String]) -> Mapping {
        let columns = self
            .columns
            .iter()
            .filter_map(|(key, saved)| {
                let saved = normalize(saved);
                headers
                    .iter()
                    .find(|header| normalize(header) == saved)
                    .map(|header| (key.clone(), header.clone()))
            })
            .collect();
        Mapping { columns }
    }
}

/// 套用映射后的结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MappedTable {
    /// 没有问题的行
    pub rows: Vec<MappedRow>,
    /// 有问题、没有导入的行
    pub problems: Vec<RowProblem>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MappedRow {
    /// 在表格里是第几行
    pub line: usize,
    /// 字段键 → 值；空单元格不放进来
    pub values: BTreeMap<String, CellValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowProblem {
    pub line: usize,
    /// 字段显示名
    pub field: String,
    pub message: String,
}

/// 统一大小写，去掉空白、标点，全角转半角。"商品 名称：" 和 "商品名称" 算同一个。
fn normalize(text: &str) -> String {
    text.chars()
        .map(|c| match u32::from(c) {
            // 全角 ASCII（！到～）转半角
            code @ 0xFF01..=0xFF5E => char::from_u32(code - 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation() && !is_cjk_punctuation(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_cjk_punctuation(c: char) -> bool {
    matches!(u32::from(c), 0x3000..=0x303F | 0x2010..=0x2027 | 0x00B7)
}

/// 自动对列：先找名称完全相同的，再找"表头包含别名"且只有一个候选的。对不上的字段不放进映射。
pub fn auto_map(headers: &[String], fields: &[FieldSpec]) -> Mapping {
    let normalized: Vec<String> = headers.iter().map(|h| normalize(h)).collect();
    let mut used: HashSet<usize> = HashSet::new();
    let mut columns = BTreeMap::new();

    for field in fields {
        let names: Vec<String> = field.names().collect();
        if let Some(index) = (0..headers.len())
            .find(|&i| !used.contains(&i) && names.iter().any(|name| *name == normalized[i]))
        {
            used.insert(index);
            columns.insert(field.key.clone(), headers[index].clone());
        }
    }
    for field in fields {
        if columns.contains_key(&field.key) {
            continue;
        }
        let names: Vec<String> = field
            .names()
            .filter(|name| name.chars().count() >= MIN_CONTAINS_LEN)
            .collect();
        let candidates: Vec<usize> = (0..headers.len())
            .filter(|i| {
                !used.contains(i)
                    && names
                        .iter()
                        .any(|name| normalized[*i].contains(name.as_str()))
            })
            .collect();
        if let [index] = candidates.as_slice() {
            used.insert(*index);
            columns.insert(field.key.clone(), headers[*index].clone());
        }
    }
    Mapping { columns }
}

/// 两组表头的重合度：交集 / 并集（比较前统一写法）。用来认出保存过的映射。
pub fn header_overlap(a: &[String], b: &[String]) -> f64 {
    let a: HashSet<String> = a.iter().map(|h| normalize(h)).collect();
    let b: HashSet<String> = b.iter().map(|h| normalize(h)).collect();
    let union = a.union(&b).count();
    if union == 0 {
        return 0.0;
    }
    // 表头最多几百个，转成 f64 不会丢精度
    a.intersection(&b).count() as f64 / union as f64
}

/// 套用映射，逐行取值。必填字段没对上列时整体报错；某行必填字段为空时这一行记为问题、不导入。
pub fn apply_mapping(
    table: &Table,
    mapping: &Mapping,
    fields: &[FieldSpec],
) -> Result<MappedTable> {
    let unmapped: Vec<&str> = fields
        .iter()
        .filter(|f| f.required && !mapping.columns.contains_key(&f.key))
        .map(|f| f.label.as_str())
        .collect();
    if !unmapped.is_empty() {
        return Err(IoError::Mapping(format!(
            "必填字段还没有对应的列：{}",
            unmapped.join("、")
        )));
    }
    let mut targets = Vec::new();
    for field in fields {
        let Some(header) = mapping.columns.get(&field.key) else {
            continue;
        };
        let index = table
            .headers
            .iter()
            .position(|h| h == header)
            .ok_or_else(|| {
                IoError::Mapping(format!(
                    "「{}」对应的列「{header}」在表格里找不到",
                    field.label
                ))
            })?;
        targets.push((field, index));
    }

    let mut rows = Vec::new();
    let mut problems = Vec::new();
    for row in &table.rows {
        let mut values = BTreeMap::new();
        let mut ok = true;
        for (field, index) in &targets {
            match row.cells.get(*index) {
                Some(cell) if !cell.is_empty() => {
                    values.insert(field.key.clone(), cell.clone());
                }
                _ if field.required => {
                    ok = false;
                    problems.push(RowProblem {
                        line: row.line,
                        field: field.label.clone(),
                        message: format!("{}是必填的，这一行是空的", field.label),
                    });
                }
                _ => {}
            }
        }
        if ok {
            rows.push(MappedRow {
                line: row.line,
                values,
            });
        }
    }
    Ok(MappedTable { rows, problems })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::TableRow;

    fn headers(list: &[&str]) -> Vec<String> {
        list.iter().map(|h| (*h).to_owned()).collect()
    }

    fn fields() -> Vec<FieldSpec> {
        vec![
            FieldSpec::new("product.name", "商品名称", &["标题", "Product Name"], true),
            FieldSpec::new("product.code", "货号", &["商品编码", "SKU"], false),
            FieldSpec::new("product.brand", "品牌", &["Brand"], false),
            FieldSpec::new("product.cost", "成本", &["采购价"], false),
        ]
    }

    #[test]
    fn normalizes_spacing_case_and_punctuation() {
        assert_eq!(normalize(" 商品 名称： "), "商品名称");
        assert_eq!(normalize("Product_Name"), "productname");
        assert_eq!(normalize("ＳＫＵ（必填）"), "sku必填");
    }

    #[test]
    fn auto_mapping_prefers_exact_names() {
        let mapping = auto_map(
            &headers(&["商品编码", "PRODUCT NAME", "品牌名称", "采购价(元)", "备注"]),
            &fields(),
        );
        assert_eq!(mapping.columns["product.name"], "PRODUCT NAME");
        assert_eq!(mapping.columns["product.code"], "商品编码");
        // 包含匹配："品牌名称"包含"品牌"，"采购价(元)"包含"采购价"
        assert_eq!(mapping.columns["product.brand"], "品牌名称");
        assert_eq!(mapping.columns["product.cost"], "采购价(元)");
    }

    #[test]
    fn ambiguous_contains_matches_are_left_alone() {
        let mapping = auto_map(&headers(&["品牌中文", "品牌英文"]), &fields());
        assert!(!mapping.columns.contains_key("product.brand"));
    }

    #[test]
    fn overlap_ignores_spacing() {
        let saved = headers(&["商品名称", "货号", "品牌", "成本", "备注"]);
        let today = headers(&["商品 名称", "货号", "品牌", "成本", "备注"]);
        assert!((header_overlap(&saved, &today) - 1.0).abs() < f64::EPSILON);
        let other = headers(&["日期", "访客", "订单", "成本", "备注"]);
        assert!(header_overlap(&saved, &other) < SAVED_MAPPING_MIN_OVERLAP);
        assert!(header_overlap(&[], &[]).abs() < f64::EPSILON);
    }

    #[test]
    fn saved_mappings_follow_header_spelling() {
        let mut saved = Mapping::default();
        saved
            .columns
            .insert("product.name".into(), "名称（必填）".into());
        saved.columns.insert("product.brand".into(), "品牌".into());
        let rebound = saved.rebind(&headers(&["名称 (必填)", "货号"]));
        assert_eq!(rebound.columns.len(), 1);
        assert_eq!(rebound.columns["product.name"], "名称 (必填)");
    }

    #[test]
    fn rows_with_missing_required_values_are_reported() {
        let table = Table {
            sheet_names: Vec::new(),
            sheet: String::new(),
            header_line: 1,
            headers: headers(&["标题", "货号"]),
            rows: vec![
                TableRow {
                    line: 2,
                    cells: vec![CellValue::Text("T 恤".into()), CellValue::Number(7.0)],
                },
                TableRow {
                    line: 3,
                    cells: vec![CellValue::Empty, CellValue::Text("B-2".into())],
                },
            ],
        };
        let fields = fields();
        let mapping = auto_map(&table.headers, &fields);
        let mapped = apply_mapping(&table, &mapping, &fields).unwrap();
        assert_eq!(mapped.rows.len(), 1);
        assert_eq!(
            mapped.rows[0].values["product.code"].as_text().unwrap(),
            "7"
        );
        assert_eq!(
            mapped.problems,
            [RowProblem {
                line: 3,
                field: "商品名称".into(),
                message: "商品名称是必填的，这一行是空的".into()
            }]
        );

        let error = apply_mapping(&table, &Mapping::default(), &fields).unwrap_err();
        assert_eq!(error.to_string(), "必填字段还没有对应的列：商品名称");
        let mut stale = mapping.clone();
        stale.columns.insert("product.brand".into(), "品牌".into());
        assert_eq!(
            apply_mapping(&table, &stale, &fields)
                .unwrap_err()
                .to_string(),
            "「品牌」对应的列「品牌」在表格里找不到"
        );
    }
}
