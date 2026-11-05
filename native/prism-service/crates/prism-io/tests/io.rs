//! 表格读写的集成测试：真实的 xlsx / csv / zip 文件。

// 测试里的辅助函数出错就该直接失败，用 unwrap 最直接
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Read;

use prism_io::{
    CellValue, ExportValue, FieldSpec, IoError, Mapping, ReadOptions, RowData,
    SAVED_MAPPING_MIN_OVERLAP, ZipEntry, ZipSource, apply_mapping, auto_map, header_overlap,
    read_table, read_table_bytes, write_xlsx, write_zip,
};
use prism_rules::{ExportTemplate, Resource, validate};
use rust_xlsxwriter::{ExcelDateTime, Format, Workbook};
use serde_json::json;
use tempfile::TempDir;

fn text(value: &str) -> CellValue {
    CellValue::Text(value.into())
}

/// 一个典型的后台报表：前两行是标题和空行，第三行才是表头。
fn report_xlsx() -> Vec<u8> {
    let mut workbook = Workbook::new();
    let date = Format::new().set_num_format("yyyy-mm-dd");
    let sheet = workbook.add_worksheet();
    sheet.set_name("日报").unwrap();
    sheet.write_string(0, 0, "店铺日报（2026-11-03）").unwrap();
    for (col, header) in ["日期", "货号", "商品", "访客", "转化率"]
        .iter()
        .enumerate()
    {
        sheet
            .write_string(2, u16::try_from(col).unwrap(), *header)
            .unwrap();
    }
    let day = ExcelDateTime::from_ymd(2026, 11, 3).unwrap();
    sheet.write_datetime_with_format(3, 0, &day, &date).unwrap();
    sheet.write_string(3, 1, "007").unwrap();
    sheet.write_string(3, 2, "  T 恤  ").unwrap();
    sheet.write_number(3, 3, 120).unwrap();
    sheet.write_number(3, 4, 0.025).unwrap();
    let other = workbook.add_worksheet();
    other.set_name("说明").unwrap();
    other.write_string(0, 0, "备注").unwrap();
    other.write_string(1, 0, "第二个工作表").unwrap();
    workbook.save_to_buffer().unwrap()
}

#[test]
fn reads_xlsx_with_titles_above_the_header() {
    let table = read_table_bytes(report_xlsx(), "XLSX", &ReadOptions::default()).unwrap();
    assert_eq!(table.sheet_names, ["日报", "说明"]);
    assert_eq!(table.sheet, "日报");
    assert_eq!(table.header_line, 3);
    assert_eq!(table.headers, ["日期", "货号", "商品", "访客", "转化率"]);
    assert_eq!(table.rows.len(), 1);
    let row = &table.rows[0];
    assert_eq!(row.line, 4);
    assert_eq!(
        row.cells,
        [
            text("2026-11-03"),
            text("007"),
            text("T 恤"),
            CellValue::Number(120.0),
            CellValue::Number(0.025)
        ]
    );

    let other = read_table_bytes(
        report_xlsx(),
        "xlsx",
        &ReadOptions {
            sheet: Some("说明".into()),
            ..ReadOptions::default()
        },
    )
    .unwrap();
    assert_eq!(other.headers, ["备注"]);
    assert!(matches!(
        read_table_bytes(
            report_xlsx(),
            "xlsx",
            &ReadOptions {
                sheet: Some("不存在".into()),
                ..ReadOptions::default()
            }
        ),
        Err(IoError::SheetNotFound(_))
    ));
}

#[test]
fn reads_gbk_csv_from_disk() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("商品.csv");
    // GB18030 兼容 GBK，常用汉字的编码一样
    let (gbk, _, had_errors) =
        encoding_rs::GB18030.encode("商品名称,货号,成本\r\n纯棉T恤,007,23.5\r\n");
    assert!(!had_errors);
    std::fs::write(&path, gbk).unwrap();
    let table = read_table(&path, &ReadOptions::default()).unwrap();
    assert_eq!(table.headers, ["商品名称", "货号", "成本"]);
    assert_eq!(
        table.rows[0].cells,
        [text("纯棉T恤"), text("007"), text("23.5")]
    );
}

#[test]
fn rejects_bad_files() {
    assert!(matches!(
        read_table_bytes(b"x".to_vec(), "pdf", &ReadOptions::default()),
        Err(IoError::Unsupported(ext)) if ext == ".pdf"
    ));
    assert!(matches!(
        read_table_bytes(b"not a workbook".to_vec(), "xlsx", &ReadOptions::default()),
        Err(IoError::Corrupted(_))
    ));
    assert!(matches!(
        read_table_bytes(b",,\n,,\n".to_vec(), "csv", &ReadOptions::default()),
        Err(IoError::Empty)
    ));

    // 超过 50 MB：看文件大小就拒绝，不读内容
    let dir = TempDir::new().unwrap();
    let big = dir.path().join("大.csv");
    let file = std::fs::File::create(&big).unwrap();
    file.set_len(prism_io::MAX_TABLE_BYTES + 1).unwrap();
    drop(file);
    let error = read_table(&big, &ReadOptions::default()).unwrap_err();
    assert_eq!(error.to_string(), "表格文件超过 50 MB，请拆分后再导入");
}

fn product_fields() -> Vec<FieldSpec> {
    vec![
        FieldSpec::new("product.name", "商品名称", &["标题", "Product Name"], true),
        FieldSpec::new("product.code", "货号", &["商品编码"], false),
        FieldSpec::new("product.brand", "品牌", &[], false),
        FieldSpec::new("product.cost", "成本", &["采购价"], false),
    ]
}

fn product_csv(header: &str, count: usize) -> Vec<u8> {
    let mut csv = format!("{header}\n");
    for i in 1..=count {
        csv.push_str(&format!("商品 {i},P-{i:03},自有品牌,{}.5,随便\n", 10 + i));
    }
    csv.into_bytes()
}

#[test]
fn saved_mappings_are_recognized_on_the_next_import() {
    // 需求验收 1：50 个商品，第一次手工调整映射并保存，第二次导入直接套用
    let fields = product_fields();
    let first = read_table_bytes(
        product_csv("名称（必填）,商品编码,品牌,采购价,备注", 50),
        "csv",
        &ReadOptions::default(),
    )
    .unwrap();
    let mut mapping = auto_map(&first.headers, &fields);
    // "名称（必填）"自动对不上"商品名称"，用户手工指定
    assert!(!mapping.columns.contains_key("product.name"));
    mapping
        .columns
        .insert("product.name".into(), "名称（必填）".into());
    let saved_headers = first.headers.clone();
    let saved = serde_json::to_string(&mapping).unwrap();

    // 第二次：表头多了空格，列顺序也变了
    let second = read_table_bytes(
        product_csv("名称 （必填）,商品编码, 品牌,采购价,备注", 50),
        "csv",
        &ReadOptions::default(),
    )
    .unwrap();
    assert!(header_overlap(&saved_headers, &second.headers) >= SAVED_MAPPING_MIN_OVERLAP);
    // 保存的是当时的表头写法，套用前对回新表头
    let reused: Mapping = serde_json::from_str(&saved).unwrap();
    let reused = reused.rebind(&second.headers);
    let mapped = apply_mapping(&second, &reused, &fields).unwrap();
    assert_eq!(mapped.rows.len(), 50);
    assert!(mapped.problems.is_empty());
    let first_row = &mapped.rows[0].values;
    assert_eq!(first_row["product.name"], text("商品 1"));
    assert_eq!(first_row["product.code"], text("P-001"));
    assert_eq!(first_row["product.cost"], text("11.5"));
}

fn template(value: serde_json::Value) -> ExportTemplate {
    match validate(value.to_string().as_bytes()).unwrap() {
        Resource::ExportTemplate(template) => template,
        other => panic!("{other:?}"),
    }
}

#[test]
fn exports_follow_the_template() {
    let template = template(json!({
        "kind": "export_template", "formatVersion": 1, "id": "demo", "revision": 1,
        "name": { "zh-CN": "示例", "en": "Demo" }, "status": "draft", "updatedAt": "2026-11-04",
        "sheetName": "Listing", "rowPer": "sku",
        "preamble": [["TemplateType=demo", "Version=1"]],
        "columns": [
            { "header": "Title", "source": "copy.title", "required": true },
            { "header": "SKU", "source": "sku.code" },
            { "header": "Price", "source": "sku.price", "format": "money" },
            { "header": "Stock", "source": "sku.stock", "format": "integer" },
            { "header": "Bullets", "source": "copy.bullets", "separator": "\n" },
            { "header": "Brand", "source": "product.brand", "default": "Generic" },
            { "header": "Condition", "source": "const:New" },
            { "header": "Gift", "source": "product.gift", "format": "boolean" }
        ]
    }));
    let mut first = RowData::new();
    first.insert("copy.title".into(), ExportValue::Text("Cotton Tee".into()));
    first.insert("sku.code".into(), ExportValue::Text("007".into()));
    first.insert("sku.price".into(), ExportValue::Money(5990));
    first.insert("sku.stock".into(), ExportValue::Integer(30));
    first.insert(
        "copy.bullets".into(),
        ExportValue::List(vec!["Soft".into(), "Breathable".into()]),
    );
    first.insert("product.gift".into(), ExportValue::Text("是".into()));
    let mut second = RowData::new();
    second.insert("sku.code".into(), ExportValue::Text("008".into()));
    second.insert("sku.stock".into(), ExportValue::Text("很多".into()));

    let (bytes, report) = write_xlsx(&template, &[first, second]).unwrap();
    assert_eq!(report.rows, 2);
    let problems: Vec<(usize, usize, &str, &str)> = report
        .problems
        .iter()
        .map(|p| (p.row, p.line, p.column.as_str(), p.message.as_str()))
        .collect();
    assert_eq!(
        problems,
        [
            (2, 4, "Title", "必填，但没有数据"),
            (2, 4, "Stock", "「很多」不是整数，按文本写入"),
        ]
    );

    let table = read_table_bytes(
        bytes,
        "xlsx",
        &ReadOptions {
            header_line: Some(2),
            ..ReadOptions::default()
        },
    )
    .unwrap();
    assert_eq!(table.sheet, "Listing");
    assert_eq!(
        table.headers,
        [
            "Title",
            "SKU",
            "Price",
            "Stock",
            "Bullets",
            "Brand",
            "Condition",
            "Gift"
        ]
    );
    assert_eq!(
        table.rows[0].cells,
        [
            text("Cotton Tee"),
            text("007"),
            CellValue::Number(59.9),
            CellValue::Number(30.0),
            text("Soft\nBreathable"),
            text("Generic"),
            text("New"),
            CellValue::Bool(true)
        ]
    );
    assert_eq!(table.rows[1].cells[0], CellValue::Empty);
    assert_eq!(table.rows[1].cells[3], text("很多"));
}

#[test]
fn zips_files_and_bytes_safely() {
    let dir = TempDir::new().unwrap();
    let image = dir.path().join("a.png");
    std::fs::write(&image, b"png-bytes").unwrap();
    let target = dir.path().join("导出.zip");
    let report = write_zip(
        &target,
        &[
            ZipEntry {
                name: "images/主图_1.png".into(),
                source: ZipSource::File(image.clone()),
            },
            ZipEntry {
                name: "images/主图_1.png".into(),
                source: ZipSource::Bytes(b"second".to_vec()),
            },
            ZipEntry {
                name: "../刊登表.csv".into(),
                source: ZipSource::Bytes("标题\n".repeat(100).into_bytes()),
            },
        ],
    )
    .unwrap();
    assert_eq!(report.files, 3);
    assert_eq!(
        report.renamed,
        [
            (
                "images/主图_1.png".to_owned(),
                "images/主图_1 (2).png".to_owned()
            ),
            ("../刊登表.csv".to_owned(), "刊登表.csv".to_owned())
        ]
    );
    assert_eq!(report.bytes, std::fs::metadata(&target).unwrap().len());
    assert!(!dir.path().join("导出.zip.tmp").exists());

    let mut archive = zip::ZipArchive::new(std::fs::File::open(&target).unwrap()).unwrap();
    let mut names: Vec<String> = archive
        .file_names()
        .map(|name| name.unwrap().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["images/主图_1 (2).png", "images/主图_1.png", "刊登表.csv"]
    );
    let mut content = String::new();
    archive
        .by_name("images/主图_1.png")
        .unwrap()
        .read_to_string(&mut content)
        .unwrap();
    assert_eq!(content, "png-bytes");
    assert_eq!(
        archive.by_name("images/主图_1.png").unwrap().compression(),
        zip::CompressionMethod::Stored
    );
    assert_eq!(
        archive.by_name("刊登表.csv").unwrap().compression(),
        zip::CompressionMethod::Deflated
    );

    // 源文件不存在：报错，不留下半个 zip
    let broken = dir.path().join("坏.zip");
    let error = write_zip(
        &broken,
        &[ZipEntry {
            name: "x.png".into(),
            source: ZipSource::File(dir.path().join("不存在.png")),
        }],
    )
    .unwrap_err();
    assert!(matches!(error, IoError::Read { .. }));
    assert!(!broken.exists());
    assert!(!dir.path().join("坏.zip.tmp").exists());
}
