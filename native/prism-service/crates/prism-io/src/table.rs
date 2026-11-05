//! 读表格（详细设计 11.1）：xlsx / xls / ods 用 calamine，csv / tsv 用 csv 库；找表头行、整理表头。

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

use calamine::{Data, Reader};
use serde::Serialize;

use crate::csv_read;
use crate::{IoError, Result};

/// 表格文件大小上限。
pub const MAX_TABLE_BYTES: u64 = 50 * 1024 * 1024;
/// 数据行数上限（不含表头）。
pub const MAX_ROWS: usize = 50_000;

/// 不指定表头行时，在前多少行里找。
const HEADER_SCAN_ROWS: usize = 20;
/// 非空单元格数达到最宽那行的这个比例，就当作表头行。
const HEADER_WIDTH_RATIO: f64 = 0.6;

/// 单元格的值。日期已经转成 `YYYY-MM-DD` 文本。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CellValue {
    Empty,
    Text(String),
    Number(f64),
    Bool(bool),
}

impl CellValue {
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// 转成文本。整数形式的数字不带 `.0`（编码、条码常被存成数字）。
    pub fn as_text(&self) -> Option<String> {
        match self {
            Self::Empty => None,
            Self::Text(text) => Some(text.clone()),
            Self::Number(number) => Some(format_number(*number)),
            Self::Bool(value) => Some(value.to_string()),
        }
    }
}

/// 整数部分不超过这个数时，没有小数的数字写成整数形式。2^53 以内 f64 能精确表示整数。
const EXACT_INTEGER_LIMIT: f64 = 9_007_199_254_740_992.0;

pub(crate) fn format_number(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < EXACT_INTEGER_LIMIT {
        format!("{number:.0}")
    } else {
        number.to_string()
    }
}

/// 读表格的选项。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadOptions {
    /// 工作表名称，不填用第一个
    pub sheet: Option<String>,
    /// 表头在第几行（从 1 开始，和 Excel 里看到的一致），不填自动找
    pub header_line: Option<usize>,
}

/// 读出来的表格。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    /// 文件里所有工作表的名称；CSV 为空
    pub sheet_names: Vec<String>,
    /// 读的是哪个工作表；CSV 为空
    pub sheet: String,
    /// 表头在第几行（从 1 开始）
    pub header_line: usize,
    pub headers: Vec<String>,
    pub rows: Vec<TableRow>,
}

/// 一行数据，`cells` 和 `headers` 一一对应。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableRow {
    /// 在表格里是第几行（从 1 开始）
    pub line: usize,
    pub cells: Vec<CellValue>,
}

/// 读表格文件，按扩展名判断格式。
pub fn read_table(path: &Path, options: &ReadOptions) -> Result<Table> {
    let metadata = std::fs::metadata(path).map_err(|error| IoError::read(path, error))?;
    if metadata.len() > MAX_TABLE_BYTES {
        return Err(too_large());
    }
    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned())
        .unwrap_or_default();
    let bytes = std::fs::read(path).map_err(|error| IoError::read(path, error))?;
    read_table_bytes(bytes, &extension, options)
}

/// 读内存里的表格，`extension` 是不带点的扩展名，如 `xlsx`。
pub fn read_table_bytes(bytes: Vec<u8>, extension: &str, options: &ReadOptions) -> Result<Table> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_TABLE_BYTES {
        return Err(too_large());
    }
    match extension.to_ascii_lowercase().as_str() {
        "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" => read_workbook(bytes, options),
        "csv" | "txt" => {
            let grid = csv_read::parse(&bytes, None)?;
            build_table(Vec::new(), String::new(), grid, 1, options)
        }
        "tsv" => {
            let grid = csv_read::parse(&bytes, Some(b'\t'))?;
            build_table(Vec::new(), String::new(), grid, 1, options)
        }
        other => Err(IoError::Unsupported(if other.is_empty() {
            "没有扩展名".into()
        } else {
            format!(".{other}")
        })),
    }
}

fn too_large() -> IoError {
    IoError::TooLarge {
        limit_mb: MAX_TABLE_BYTES / 1024 / 1024,
    }
}

fn read_workbook(bytes: Vec<u8>, options: &ReadOptions) -> Result<Table> {
    let mut workbook = calamine::open_workbook_auto_from_rs(Cursor::new(bytes))
        .map_err(|error| IoError::Corrupted(error.to_string()))?;
    let names = workbook.sheet_names();
    let sheet = match &options.sheet {
        Some(name) if names.contains(name) => name.clone(),
        Some(name) => return Err(IoError::SheetNotFound(name.clone())),
        None => names.first().cloned().ok_or(IoError::Empty)?,
    };
    let range = workbook
        .worksheet_range(&sheet)
        .map_err(|error| IoError::Corrupted(error.to_string()))?;
    if range.height() > MAX_ROWS + HEADER_SCAN_ROWS {
        return Err(IoError::TooManyRows { limit: MAX_ROWS });
    }
    // 表格内容不一定从 A1 开始，补上前面的空行列，行号和列号才对得上
    let (start_row, start_col) = range.start().unwrap_or((0, 0));
    let leading = usize::try_from(start_col).unwrap_or(0);
    let grid = range
        .rows()
        .map(|row| {
            let mut cells = vec![CellValue::Empty; leading];
            cells.extend(row.iter().map(convert));
            cells
        })
        .collect();
    let first_line = usize::try_from(start_row).unwrap_or(0) + 1;
    build_table(names, sheet, grid, first_line, options)
}

fn convert(data: &Data) -> CellValue {
    match data {
        Data::Empty => CellValue::Empty,
        Data::String(text) => text_cell(text),
        // 2^53 以上的整数会丢精度，表格里这么大的数基本只有长编码，而长编码通常存成文本
        Data::Int(number) => CellValue::Number(*number as f64),
        Data::Float(number) => CellValue::Number(*number),
        Data::Bool(value) => CellValue::Bool(*value),
        Data::DateTime(value) if value.is_duration() => CellValue::Number(value.as_f64()),
        Data::DateTime(value) => match value.as_datetime() {
            Some(datetime) => CellValue::Text(date_text(&datetime.to_string())),
            None => CellValue::Number(value.as_f64()),
        },
        Data::DateTimeIso(text) | Data::DurationIso(text) => text_cell(text),
        Data::Error(error) => CellValue::Text(error.to_string()),
    }
}

/// chrono 的默认显示是 `YYYY-MM-DD HH:MM:SS[.小数]`：去掉秒的小数（Excel 的浮点误差），
/// 零点的只留日期。
fn date_text(display: &str) -> String {
    let without_fraction = display.split('.').next().unwrap_or(display);
    without_fraction
        .strip_suffix(" 00:00:00")
        .unwrap_or(without_fraction)
        .to_owned()
}

/// 文本去首尾空格，只有空白的当作空。
pub(crate) fn text_cell(text: &str) -> CellValue {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        CellValue::Empty
    } else {
        CellValue::Text(trimmed.to_owned())
    }
}

/// 从原始的行列数据里找表头、整理表头、取数据行。`first_line` 是 `grid[0]` 在表格里的行号。
fn build_table(
    sheet_names: Vec<String>,
    sheet: String,
    grid: Vec<Vec<CellValue>>,
    first_line: usize,
    options: &ReadOptions,
) -> Result<Table> {
    let filled = |row: &[CellValue]| row.iter().filter(|cell| !cell.is_empty()).count();
    if grid.iter().all(|row| filled(row) == 0) {
        return Err(IoError::Empty);
    }
    let header_index = match options.header_line {
        Some(line) => {
            let total = first_line - 1 + grid.len();
            if line < first_line || line > total {
                return Err(IoError::HeaderOutOfRange { line, total });
            }
            line - first_line
        }
        None => guess_header(&grid),
    };

    let used_width = |row: &[CellValue]| {
        row.iter()
            .rposition(|cell| !cell.is_empty())
            .map_or(0, |i| i + 1)
    };
    let width = grid[header_index..]
        .iter()
        .map(|row| used_width(row))
        .max()
        .unwrap_or(0);
    let headers = clean_headers(&grid[header_index], width);

    let mut rows = Vec::new();
    for (offset, row) in grid.iter().enumerate().skip(header_index + 1) {
        if filled(row) == 0 {
            continue;
        }
        if rows.len() == MAX_ROWS {
            return Err(IoError::TooManyRows { limit: MAX_ROWS });
        }
        let mut cells: Vec<CellValue> = row.iter().take(width).cloned().collect();
        cells.resize(width, CellValue::Empty);
        rows.push(TableRow {
            line: first_line + offset,
            cells,
        });
    }
    Ok(Table {
        sheet_names,
        sheet,
        header_line: first_line + header_index,
        headers,
        rows,
    })
}

/// 前 20 行里第一个"非空单元格数 ≥ 最宽那行 60%"的行。很多报表前几行是标题和日期。
fn guess_header(grid: &[Vec<CellValue>]) -> usize {
    let counts: Vec<usize> = grid
        .iter()
        .take(HEADER_SCAN_ROWS)
        .map(|row| row.iter().filter(|cell| !cell.is_empty()).count())
        .collect();
    let widest = counts.iter().copied().max().unwrap_or(0);
    // 行数最多几十，转成 f64 不会丢精度
    let threshold = ((widest as f64) * HEADER_WIDTH_RATIO).ceil().max(1.0);
    counts
        .iter()
        .position(|&count| count as f64 >= threshold)
        .unwrap_or(0)
}

/// 表头去空格；空的叫"第 C 列"；重名的加 (2)、(3)。
fn clean_headers(row: &[CellValue], width: usize) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    (0..width)
        .map(|index| {
            let name = row
                .get(index)
                .and_then(CellValue::as_text)
                .map(|text| text.trim().to_owned())
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| format!("第 {} 列", column_letter(index)));
            let count = seen.entry(name.clone()).or_insert(0);
            *count += 1;
            if *count == 1 {
                name
            } else {
                format!("{name} ({count})")
            }
        })
        .collect()
}

/// 0 → A，25 → Z，26 → AA。
pub(crate) fn column_letter(index: usize) -> String {
    let mut letters = Vec::new();
    let mut n = index + 1;
    while n > 0 {
        let remainder = (n - 1) % 26;
        // remainder < 26，转成 u8 不会溢出
        letters.push(char::from(b'A' + u8::try_from(remainder).unwrap_or(0)));
        n = (n - 1) / 26;
    }
    letters.iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> CellValue {
        CellValue::Text(value.into())
    }

    #[test]
    fn numbers_print_like_people_expect() {
        assert_eq!(
            CellValue::Number(6_901_234_567_892.0).as_text().unwrap(),
            "6901234567892"
        );
        assert_eq!(CellValue::Number(12.5).as_text().unwrap(), "12.5");
        assert_eq!(CellValue::Number(-3.0).as_text().unwrap(), "-3");
        assert_eq!(CellValue::Empty.as_text(), None);
    }

    #[test]
    fn dates_drop_midnight_and_fractions() {
        assert_eq!(date_text("2026-11-03 00:00:00"), "2026-11-03");
        assert_eq!(date_text("2026-11-03 08:30:00"), "2026-11-03 08:30:00");
        assert_eq!(date_text("2026-11-03 23:59:59.999"), "2026-11-03 23:59:59");
    }

    #[test]
    fn column_letters() {
        assert_eq!(column_letter(0), "A");
        assert_eq!(column_letter(25), "Z");
        assert_eq!(column_letter(26), "AA");
        assert_eq!(column_letter(701), "ZZ");
        assert_eq!(column_letter(702), "AAA");
    }

    #[test]
    fn header_row_is_found_below_titles() {
        let grid = vec![
            vec![text("店铺日报 2026-11-03")],
            vec![],
            vec![text("日期"), text("商品"), text("访客"), text("订单")],
            vec![
                text("2026-11-03"),
                text("T 恤"),
                CellValue::Number(120.0),
                CellValue::Number(3.0),
            ],
        ];
        let table =
            build_table(Vec::new(), String::new(), grid, 1, &ReadOptions::default()).unwrap();
        assert_eq!(table.header_line, 3);
        assert_eq!(table.headers, ["日期", "商品", "访客", "订单"]);
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0].line, 4);
    }

    #[test]
    fn headers_are_cleaned_and_rows_padded() {
        let grid = vec![
            vec![text("名称"), CellValue::Empty, text("名称")],
            vec![
                text("A"),
                CellValue::Empty,
                CellValue::Empty,
                text("多出来的"),
            ],
            vec![CellValue::Empty, CellValue::Empty],
            vec![text("B")],
        ];
        let table =
            build_table(Vec::new(), String::new(), grid, 1, &ReadOptions::default()).unwrap();
        assert_eq!(table.headers, ["名称", "第 B 列", "名称 (2)", "第 D 列"]);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[1].line, 4);
        assert_eq!(table.rows[1].cells.len(), 4);
    }

    #[test]
    fn explicit_header_line_is_checked() {
        let grid = vec![vec![text("a")], vec![text("b")]];
        let options = ReadOptions {
            header_line: Some(2),
            ..ReadOptions::default()
        };
        let table = build_table(Vec::new(), String::new(), grid.clone(), 1, &options).unwrap();
        assert_eq!(table.headers, ["b"]);
        assert!(table.rows.is_empty());
        let options = ReadOptions {
            header_line: Some(3),
            ..ReadOptions::default()
        };
        assert!(matches!(
            build_table(Vec::new(), String::new(), grid, 1, &options),
            Err(IoError::HeaderOutOfRange { line: 3, total: 2 })
        ));
        assert!(matches!(
            build_table(
                Vec::new(),
                String::new(),
                vec![vec![CellValue::Empty]],
                1,
                &ReadOptions::default()
            ),
            Err(IoError::Empty)
        ));
    }
}
