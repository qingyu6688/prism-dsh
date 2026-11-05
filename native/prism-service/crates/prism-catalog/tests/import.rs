//! 从表格导入商品的集成测试（详细设计 9.5，需求验收 1）。

// 测试里的辅助函数出错就该直接失败，用 unwrap 最直接
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use prism_catalog::{Catalog, ImportMode, ImportRequest, ProductFilter, ProductInput};
use prism_io::ReadOptions;
use prism_store::Store;
use tempfile::TempDir;

struct Env {
    catalog: Catalog,
    dir: TempDir,
}

async fn env() -> Env {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("prism.db")).await.unwrap();
    Env {
        catalog: Catalog::new(store, dir.path()),
        dir,
    }
}

impl Env {
    fn csv(&self, name: &str, content: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        path
    }
}

fn request(path: PathBuf, mapping: prism_io::Mapping) -> ImportRequest {
    ImportRequest {
        path,
        read: ReadOptions::default(),
        mapping,
        mode: ImportMode::Upsert,
        save_as: None,
        matched_mapping_id: None,
    }
}

fn products_csv(header: &str, count: usize, cost_offset: usize) -> String {
    let mut csv = format!("{header}\n");
    for i in 1..=count {
        csv.push_str(&format!(
            "纯棉T恤 {i},P-{i:03},自有品牌,{}.50,\"US, DE\"\n",
            10 + i + cost_offset
        ));
    }
    csv
}

#[tokio::test]
async fn second_import_reuses_the_saved_mapping() {
    // 需求验收 1：导入 50 个商品，列映射保存后第二次导入不用重新映射
    let env = env().await;
    let first = env.csv(
        "第一批.csv",
        &products_csv("Article,货号,品牌,采购价,目标市场", 50, 0),
    );
    let preview = env
        .catalog
        .preview_product_import(&first, &ReadOptions::default())
        .await
        .unwrap();
    assert_eq!(preview.total_rows, 50);
    assert_eq!(preview.sample.len(), 5);
    assert!(preview.matched.is_none());
    // "Article" 认不出来，其他的自动对上了
    assert!(!preview.mapping.columns.contains_key("product.name"));
    assert_eq!(preview.mapping.columns["product.code"], "货号");
    assert_eq!(preview.mapping.columns["product.cost"], "采购价");
    assert_eq!(
        preview.mapping.columns["product.target_markets"],
        "目标市场"
    );

    // 用户手工指定商品名称那一列，并保存映射
    let mut mapping = preview.mapping.clone();
    mapping
        .columns
        .insert("product.name".into(), "Article".into());
    let result = env
        .catalog
        .import_products(&ImportRequest {
            save_as: Some("供应商货品表".into()),
            ..request(first, mapping)
        })
        .await
        .unwrap();
    assert_eq!((result.created, result.updated, result.skipped), (50, 0, 0));
    let saved_id = result.saved_mapping_id.clone().unwrap();

    let p1 = env
        .catalog
        .store()
        .find_product_by_code("P-001")
        .await
        .unwrap()
        .unwrap();
    let p1 = env.catalog.get_product(&p1.id).await.unwrap();
    assert_eq!(p1.name, "纯棉T恤 1");
    assert_eq!(p1.brand.as_deref(), Some("自有品牌"));
    assert_eq!(p1.cost_cents, Some(1150));
    assert_eq!(p1.target_markets, ["US", "DE"]);

    // 第二批：表头写法有出入，成本变了
    let second = env.csv(
        "第二批.csv",
        &products_csv(" ARTICLE ,货号, 品牌,采购价,目标市场", 50, 100),
    );
    let preview = env
        .catalog
        .preview_product_import(&second, &ReadOptions::default())
        .await
        .unwrap();
    let matched = preview.matched.clone().unwrap();
    assert_eq!(matched.id, saved_id);
    assert_eq!(matched.name, "供应商货品表");
    assert_eq!(preview.mapping.columns["product.name"], "ARTICLE");
    assert_eq!(preview.mapping.columns["product.brand"], "品牌");

    let result = env
        .catalog
        .import_products(&ImportRequest {
            matched_mapping_id: Some(matched.id.clone()),
            ..request(second, preview.mapping)
        })
        .await
        .unwrap();
    assert_eq!((result.created, result.updated, result.skipped), (0, 50, 0));
    let p1 = env.catalog.get_product(&p1.id).await.unwrap();
    assert_eq!(p1.cost_cents, Some(11_150));
    let saved = env.catalog.list_import_mappings().await.unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].use_count, 1);
    assert_eq!(
        env.catalog
            .list_products(&ProductFilter::default())
            .await
            .unwrap()
            .total,
        50
    );
}

#[tokio::test]
async fn updates_only_touch_mapped_non_empty_cells() {
    let env = env().await;
    let existing = env
        .catalog
        .create_product(&ProductInput {
            code: Some("TS-1".into()),
            name: "旧名称".into(),
            brand: Some("老品牌".into()),
            description: Some("保留的说明".into()),
            cost_cents: Some(100),
            ..ProductInput::default()
        })
        .await
        .unwrap();
    // 只有货号、名称、成本三列；名称这一格是空的
    let path = env.csv("改价.csv", "货号,商品名称,成本\nTS-1,,¥25.80\n");
    let preview = env
        .catalog
        .preview_product_import(&path, &ReadOptions::default())
        .await
        .unwrap();
    let result = env
        .catalog
        .import_products(&request(path, preview.mapping))
        .await
        .unwrap();
    // 名称是必填字段，这一行名称为空，所以整行跳过
    assert_eq!(result.skipped, 1);
    assert_eq!(result.problems[0].field, "商品名称");

    let path = env.csv(
        "改价2.csv",
        "货号,商品名称,成本,品牌\nTS-1,新名称,¥25.80,\n",
    );
    let preview = env
        .catalog
        .preview_product_import(&path, &ReadOptions::default())
        .await
        .unwrap();
    let result = env
        .catalog
        .import_products(&request(path, preview.mapping))
        .await
        .unwrap();
    assert_eq!((result.created, result.updated), (0, 1));
    let updated = env.catalog.get_product(&existing.id).await.unwrap();
    assert_eq!(updated.name, "新名称");
    assert_eq!(updated.cost_cents, Some(2580));
    // 品牌那一格是空的、说明没有对应的列：都保持原样
    assert_eq!(updated.brand.as_deref(), Some("老品牌"));
    assert_eq!(updated.description.as_deref(), Some("保留的说明"));
}

#[tokio::test]
async fn bad_rows_are_skipped_and_reported() {
    let env = env().await;
    env.catalog
        .create_product(&ProductInput {
            code: Some("OLD".into()),
            name: "已有".into(),
            ..ProductInput::default()
        })
        .await
        .unwrap();
    let path = env.csv(
        "问题.csv",
        "商品名称,货号,成本,币种\n\
         正常,A1,10,\n\
         货号重复,A1,10,\n\
         成本不对,A2,待定,\n\
         ,A3,10,\n\
         币种不对,A4,10,人民币\n\
         已有货号,OLD,10,\n\
         没有货号,,10,usd\n",
    );
    let preview = env
        .catalog
        .preview_product_import(&path, &ReadOptions::default())
        .await
        .unwrap();
    let result = env
        .catalog
        .import_products(&ImportRequest {
            mode: ImportMode::CreateOnly,
            ..request(path, preview.mapping)
        })
        .await
        .unwrap();
    assert_eq!((result.created, result.updated, result.skipped), (2, 0, 5));
    let problems: Vec<(usize, &str, &str)> = result
        .problems
        .iter()
        .map(|p| (p.line, p.field.as_str(), p.message.as_str()))
        .collect();
    assert_eq!(
        problems,
        [
            (3, "货号", "货号「A1」和第 2 行重复了"),
            (4, "成本", "「待定」不是金额"),
            (5, "商品名称", "商品名称是必填的，这一行是空的"),
            (6, "", "币种「人民币」应该是三位字母代码，如 CNY、USD"),
            (7, "货号", "货号「OLD」已经存在"),
        ]
    );
    let all = env
        .catalog
        .list_products(&ProductFilter::default())
        .await
        .unwrap();
    assert_eq!(all.total, 3);
    assert!(
        all.items
            .iter()
            .any(|p| p.name == "没有货号" && p.currency == "USD")
    );
}

#[tokio::test]
async fn saved_mappings_can_be_listed_and_deleted() {
    let env = env().await;
    let headers: Vec<String> = ["名称", "货号"].iter().map(|h| (*h).to_owned()).collect();
    let mut mapping = prism_io::Mapping::default();
    mapping.columns.insert("product.name".into(), "名称".into());
    let saved = env
        .catalog
        .save_import_mapping(" 货品表 ", &headers, &mapping, Some(2))
        .await
        .unwrap();
    assert_eq!(saved.name, "货品表");
    let list = env.catalog.list_import_mappings().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].headers, headers);
    assert_eq!(list[0].mapping, mapping);
    assert_eq!(list[0].header_line, Some(2));
    assert!(
        env.catalog
            .save_import_mapping("  ", &headers, &mapping, None)
            .await
            .is_err()
    );
    env.catalog.delete_import_mapping(&saved.id).await.unwrap();
    assert!(env.catalog.list_import_mappings().await.unwrap().is_empty());
}
