//! CSV / TSV：识别编码和分隔符，读成行列。值一律当文本，不猜数字（编码、条码的前导 0 要保留）。

use encoding_rs::{Encoding, GB18030};

use crate::table::{CellValue, MAX_ROWS, text_cell};
use crate::{IoError, Result};

/// 读 CSV。`delimiter` 为空时自动识别。
pub(crate) fn parse(bytes: &[u8], delimiter: Option<u8>) -> Result<Vec<Vec<CellValue>>> {
    let text = decode(bytes);
    let delimiter = delimiter.unwrap_or_else(|| detect_delimiter(&text));
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(text.as_bytes());
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|error| {
            let line = error.position().map_or(0, csv::Position::line);
            IoError::Corrupted(format!("CSV 第 {line} 行格式不对：{error}"))
        })?;
        // 表头前可能有几行标题，多留一点余量，具体行数在整理表格时再卡
        if rows.len() > MAX_ROWS + 20 {
            return Err(IoError::TooManyRows { limit: MAX_ROWS });
        }
        rows.push(record.iter().map(text_cell).collect());
    }
    Ok(rows)
}

/// 有 BOM 按 BOM；没有时先按 UTF-8，不合法就按 GB18030（兼容 GBK）。
pub(crate) fn decode(bytes: &[u8]) -> String {
    if let Some((encoding, bom_length)) = Encoding::for_bom(bytes) {
        let (text, _) = encoding.decode_without_bom_handling(&bytes[bom_length..]);
        return text.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => GB18030.decode_without_bom_handling(bytes).0.into_owned(),
    }
}

/// 看第一行逗号、制表符、分号哪个最多。
fn detect_delimiter(text: &str) -> u8 {
    let first_line = text.lines().next().unwrap_or_default();
    [b',', b'\t', b';']
        .into_iter()
        .max_by_key(|&candidate| first_line.bytes().filter(|&b| b == candidate).count())
        .filter(|&candidate| first_line.bytes().any(|b| b == candidate))
        .unwrap_or(b',')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf8_gbk_and_bom() {
        let gbk = GB18030.encode("商品名称,价格").0.into_owned();
        assert_eq!(decode(&gbk), "商品名称,价格");
        assert_eq!(decode("商品".as_bytes()), "商品");
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend_from_slice("商品".as_bytes());
        assert_eq!(decode(&bom), "商品");
        // UTF-16 LE 带 BOM（Excel 另存为 Unicode 文本）
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "商品".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode(&utf16), "商品");
    }

    #[test]
    fn detects_delimiters() {
        assert_eq!(detect_delimiter("a,b,c\n1,2,3"), b',');
        assert_eq!(detect_delimiter("a\tb\tc"), b'\t');
        assert_eq!(detect_delimiter("a;b;c"), b';');
        assert_eq!(detect_delimiter("single"), b',');
    }

    #[test]
    fn values_stay_text_and_quotes_work() {
        let rows = parse("编码,名称\n007,\"T 恤, 白色\"\n,\n".as_bytes(), None).unwrap();
        assert_eq!(rows[1][0], CellValue::Text("007".into()));
        assert_eq!(rows[1][1], CellValue::Text("T 恤, 白色".into()));
        assert_eq!(rows[2], [CellValue::Empty, CellValue::Empty]);
    }
}
