//! 按导出模板写 xlsx（详细设计 11.3）。只按模板的来源路径取值，不知道是哪个平台的表格。

use std::collections::BTreeMap;

use prism_rules::{CellFormat, ExportColumn, ExportTemplate};
use rust_xlsxwriter::{Format, Workbook, Worksheet, XlsxError};
use serde::Serialize;

use crate::table::format_number;
use crate::{IoError, Result};

/// Excel 单元格最多能放的字符数。
const EXCEL_CELL_MAX_CHARS: usize = 32_767;
/// 列表默认的连接符。
const DEFAULT_SEPARATOR: &str = "; ";
const MONEY_FORMAT: &str = "0.00";
const CENTS_PER_UNIT: f64 = 100.0;

/// 一行里某个来源路径的值，由业务模块准备。
#[derive(Debug, Clone, PartialEq)]
pub enum ExportValue {
    Text(String),
    Integer(i64),
    Decimal(f64),
    /// 金额，单位是分
    Money(i64),
    Bool(bool),
    List(Vec<String>),
}

/// 一行数据：来源路径（如 `product.name`、`image.main[0]`）→ 值。
pub type RowData = BTreeMap<String, ExportValue>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    /// 写了多少行数据
    pub rows: usize,
    pub problems: Vec<ExportProblem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProblem {
    /// 第几行数据（从 1 开始）
    pub row: usize,
    /// 在表格里是第几行
    pub line: usize,
    /// 哪一列（表头）
    pub column: String,
    pub message: String,
}

/// 按模板生成 xlsx 文件内容和导出报告。必填列取不到值时记入报告，表格照样生成。
pub fn write_xlsx(template: &ExportTemplate, rows: &[RowData]) -> Result<(Vec<u8>, ExportReport)> {
    let mut workbook = Workbook::new();
    let money = Format::new().set_num_format(MONEY_FORMAT);
    let sheet = workbook.add_worksheet();
    sheet
        .set_name(template.sheet_name.trim())
        .map_err(generate)?;

    let mut line: u32 = 0;
    for preamble in &template.preamble {
        for (col, text) in preamble.iter().enumerate() {
            if !text.is_empty() {
                sheet
                    .write_string(line, col_num(col)?, text)
                    .map_err(generate)?;
            }
        }
        line += 1;
    }
    for (col, column) in template.columns.iter().enumerate() {
        sheet
            .write_string(line, col_num(col)?, column.header.trim())
            .map_err(generate)?;
    }
    line += 1;

    let mut report = ExportReport::default();
    for (index, row) in rows.iter().enumerate() {
        for (col, column) in template.columns.iter().enumerate() {
            let mut cell = Cell {
                sheet: &mut *sheet,
                row: line,
                col: col_num(col)?,
                money: &money,
                problems: &mut report.problems,
                row_number: index + 1,
                column,
            };
            cell.write(row)?;
        }
        line += 1;
        report.rows += 1;
    }
    let bytes = workbook.save_to_buffer().map_err(generate)?;
    Ok((bytes, report))
}

fn col_num(index: usize) -> Result<u16> {
    u16::try_from(index).map_err(|_| IoError::Generate("列数超过 Excel 上限".into()))
}

fn generate(error: XlsxError) -> IoError {
    IoError::Generate(error.to_string())
}

/// 写一个单元格需要的上下文。
struct Cell<'a> {
    sheet: &'a mut Worksheet,
    row: u32,
    col: u16,
    money: &'a Format,
    problems: &'a mut Vec<ExportProblem>,
    row_number: usize,
    column: &'a ExportColumn,
}

impl Cell<'_> {
    fn problem(&mut self, message: impl Into<String>) {
        self.problems.push(ExportProblem {
            row: self.row_number,
            // 行号从 0 开始，表格里看到的从 1 开始
            line: usize::try_from(self.row)
                .unwrap_or(usize::MAX)
                .saturating_add(1),
            column: self.column.header.trim().to_owned(),
            message: message.into(),
        });
    }

    fn write(&mut self, row: &RowData) -> Result<()> {
        let value = match lookup(&self.column.source, row) {
            Some(value) => value,
            None => match &self.column.default {
                Some(default) => ExportValue::Text(default.clone()),
                None => {
                    if self.column.required {
                        self.problem("必填，但没有数据");
                    }
                    return Ok(());
                }
            },
        };
        match self.column.format {
            CellFormat::Text => self.text(&value),
            CellFormat::Integer => match number(&value) {
                Some(n) if n.fract() == 0.0 => self.number(n, None),
                _ => self.fallback(&value, "不是整数"),
            },
            CellFormat::Decimal => match number(&value) {
                Some(n) => self.number(n, None),
                None => self.fallback(&value, "不是数字"),
            },
            CellFormat::Money => match money(&value) {
                Some(n) => self.number(n, Some(())),
                None => self.fallback(&value, "不是金额"),
            },
            CellFormat::Boolean => match boolean(&value) {
                Some(b) => self
                    .sheet
                    .write_boolean(self.row, self.col, b)
                    .map(drop)
                    .map_err(generate),
                None => self.fallback(&value, "不是是 / 否"),
            },
        }
    }

    fn text(&mut self, value: &ExportValue) -> Result<()> {
        let separator = self
            .column
            .separator
            .as_deref()
            .unwrap_or(DEFAULT_SEPARATOR);
        let mut text = to_text(value, separator);
        if text.chars().count() > EXCEL_CELL_MAX_CHARS {
            text = text.chars().take(EXCEL_CELL_MAX_CHARS).collect();
            self.problem(format!(
                "超过 Excel 单元格上限 {EXCEL_CELL_MAX_CHARS} 字，已截断"
            ));
        }
        self.sheet
            .write_string(self.row, self.col, text)
            .map(drop)
            .map_err(generate)
    }

    fn number(&mut self, value: f64, money: Option<()>) -> Result<()> {
        let result = match money {
            Some(()) => self
                .sheet
                .write_number_with_format(self.row, self.col, value, self.money),
            None => self.sheet.write_number(self.row, self.col, value),
        };
        result.map(drop).map_err(generate)
    }

    /// 转换失败时照原样按文本写，并记一条问题，用户在表格里能看到原值。
    fn fallback(&mut self, value: &ExportValue, reason: &str) -> Result<()> {
        let text = to_text(value, DEFAULT_SEPARATOR);
        self.problem(format!("「{text}」{reason}，按文本写入"));
        self.text(value)
    }
}

/// 取值：`const:` 开头的是固定值；空文本、空列表当作没有。
fn lookup(source: &str, row: &RowData) -> Option<ExportValue> {
    if let Some(constant) = source.strip_prefix("const:") {
        return Some(ExportValue::Text(constant.to_owned()));
    }
    match row.get(source)? {
        ExportValue::Text(text) if text.trim().is_empty() => None,
        ExportValue::List(items) if items.iter().all(|i| i.trim().is_empty()) => None,
        value => Some(value.clone()),
    }
}

fn to_text(value: &ExportValue, separator: &str) -> String {
    match value {
        ExportValue::Text(text) => text.clone(),
        ExportValue::Integer(n) => n.to_string(),
        ExportValue::Decimal(n) => format_number(*n),
        ExportValue::Money(cents) => {
            let sign = if *cents < 0 { "-" } else { "" };
            let abs = cents.unsigned_abs();
            format!("{sign}{}.{:02}", abs / 100, abs % 100)
        }
        ExportValue::Bool(b) => b.to_string(),
        ExportValue::List(items) => items
            .iter()
            .map(|i| i.trim())
            .filter(|i| !i.is_empty())
            .collect::<Vec<_>>()
            .join(separator),
    }
}

/// 金额在库里是分，写成元以内的数字；超过 2^53 分的金额不现实，精度损失可以忽略。
fn number(value: &ExportValue) -> Option<f64> {
    match value {
        ExportValue::Integer(n) => Some(*n as f64),
        ExportValue::Decimal(n) => Some(*n),
        ExportValue::Money(cents) => Some(*cents as f64 / CENTS_PER_UNIT),
        ExportValue::Text(text) => text.trim().parse().ok(),
        ExportValue::Bool(_) | ExportValue::List(_) => None,
    }
}

/// 金额列：整数当作分；小数和文本当作已经是元。
fn money(value: &ExportValue) -> Option<f64> {
    match value {
        ExportValue::Money(cents) | ExportValue::Integer(cents) => {
            Some(*cents as f64 / CENTS_PER_UNIT)
        }
        other => number(other),
    }
}

fn boolean(value: &ExportValue) -> Option<bool> {
    match value {
        ExportValue::Bool(b) => Some(*b),
        ExportValue::Integer(n) => Some(*n != 0),
        ExportValue::Text(text) => match text.trim().to_lowercase().as_str() {
            "true" | "yes" | "y" | "1" | "是" => Some(true),
            "false" | "no" | "n" | "0" | "否" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_convert_to_text() {
        assert_eq!(to_text(&ExportValue::Money(5990), ";"), "59.90");
        assert_eq!(to_text(&ExportValue::Money(-5), ";"), "-0.05");
        assert_eq!(to_text(&ExportValue::Decimal(2.0), ";"), "2");
        assert_eq!(
            to_text(
                &ExportValue::List(vec!["a".into(), " ".into(), "b".into()]),
                "\n"
            ),
            "a\nb"
        );
    }

    #[test]
    fn money_and_booleans() {
        assert_eq!(money(&ExportValue::Integer(1999)), Some(19.99));
        assert_eq!(money(&ExportValue::Text("19.9".into())), Some(19.9));
        assert_eq!(money(&ExportValue::Text("abc".into())), None);
        assert_eq!(boolean(&ExportValue::Text("是".into())), Some(true));
        assert_eq!(boolean(&ExportValue::Text("maybe".into())), None);
    }

    #[test]
    fn empty_values_count_as_missing() {
        let mut row = RowData::new();
        row.insert("product.name".into(), ExportValue::Text("  ".into()));
        row.insert("copy.bullets".into(), ExportValue::List(vec![]));
        assert_eq!(lookup("product.name", &row), None);
        assert_eq!(lookup("copy.bullets", &row), None);
        assert_eq!(
            lookup("const:New", &row),
            Some(ExportValue::Text("New".into()))
        );
    }
}
