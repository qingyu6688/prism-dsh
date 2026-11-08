//! `catalog.*`、`rules.*`、`search.global` 的测试：直接调方法分发，检查参数、返回值的格式和错误码。
//! 业务规则本身由各业务 crate 的测试覆盖，这里只确认接线和 JSON 形状。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use image::{ImageBuffer, Rgb};
use prism_service::methods::dispatch;
use prism_service::protocol::{BUSINESS_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND, RpcError};
use prism_service::{Context, DB_FILE};
use prism_store::Store;
use serde_json::{Value, json};
use tempfile::TempDir;

struct Env {
    ctx: Arc<Context>,
    /// 放测试用的表格、图片，和数据目录分开
    files: TempDir,
    _data: TempDir,
}

async fn env() -> Env {
    let data = TempDir::new().unwrap();
    let store = Store::open(&data.path().join(DB_FILE)).await.unwrap();
    Env {
        ctx: Context::new(store, data.path().to_path_buf()),
        files: TempDir::new().unwrap(),
        _data: data,
    }
}

impl Env {
    async fn ok(&self, method: &str, params: Value) -> Value {
        match dispatch(&self.ctx, method, params).await {
            Ok(value) => value,
            Err(error) => panic!("{method} 应该成功：{error:?}"),
        }
    }

    async fn err(&self, method: &str, params: Value) -> RpcError {
        match dispatch(&self.ctx, method, params).await {
            Ok(value) => panic!("{method} 应该失败，实际返回 {value}"),
            Err(error) => error,
        }
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.files.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
}

fn png(width: u32, height: u32, shade: u8) -> Vec<u8> {
    let buffer: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_pixel(width, height, Rgb([shade, shade, shade]));
    let mut bytes = Vec::new();
    buffer
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

fn data_code(error: &RpcError) -> &str {
    error.data.as_ref().unwrap()["code"].as_str().unwrap()
}

#[tokio::test]
async fn shops_round_trip() {
    let env = env().await;
    let shop = env
        .ok(
            "catalog.createShop",
            json!({"input": {"name": "美国站", "platform": "amazon", "site": "US", "language": "en-US", "currency": "USD"}}),
        )
        .await;
    let id = shop["id"].as_str().unwrap().to_owned();
    assert_eq!(shop["platform"], "amazon");
    assert!(shop["sortOrder"].is_i64());

    let updated = env
        .ok(
            "catalog.updateShop",
            json!({"id": id, "input": {"name": "美国主站", "platform": "amazon", "site": "US"}}),
        )
        .await;
    assert_eq!(updated["name"], "美国主站");
    // 没传的字段用输入类型的默认值
    assert_eq!(updated["currency"], "CNY");

    let shops = env.ok("catalog.listShops", json!({})).await;
    assert_eq!(shops.as_array().unwrap().len(), 1);
    let filtered = env
        .ok("catalog.listShops", json!({"platform": "temu"}))
        .await;
    assert!(filtered.as_array().unwrap().is_empty());

    assert_eq!(
        env.ok("catalog.deleteShop", json!({"id": id})).await,
        json!({})
    );
    let error = env.err("catalog.deleteShop", json!({"id": id})).await;
    assert_eq!(error.code, BUSINESS_ERROR);
    assert_eq!(data_code(&error), "not_found");
}

#[tokio::test]
async fn products_list_detail_archive() {
    let env = env().await;
    let created = env
        .ok(
            "catalog.createProduct",
            json!({"input": {"code": "TS-1", "name": "纯棉T恤", "category": "上衣", "sellingPoints": ["透气"], "costCents": 2580}}),
        )
        .await;
    let id = created["id"].as_str().unwrap().to_owned();
    assert_eq!(created["sellingPoints"], json!(["透气"]));
    assert_eq!(created["archived"], false);
    env.ok(
        "catalog.createProduct",
        json!({"input": {"code": "MUG-1", "name": "陶瓷杯"}}),
    )
    .await;

    let page = env
        .ok(
            "catalog.listProducts",
            json!({"filter": {"keyword": "T恤"}}),
        )
        .await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["id"], id.as_str());
    // 不传筛选条件就是在用的全部商品
    assert_eq!(env.ok("catalog.listProducts", json!({})).await["total"], 2);

    let detail = env.ok("catalog.productDetail", json!({"id": id})).await;
    assert_eq!(detail["product"]["name"], "纯棉T恤");
    assert_eq!(detail["skus"], json!([]));
    assert_eq!(detail["summary"]["imageAssets"], 0);

    let updated = env
        .ok(
            "catalog.updateProduct",
            json!({"id": id, "input": {"code": "TS-1", "name": "纯棉短袖T恤"}}),
        )
        .await;
    assert_eq!(updated["name"], "纯棉短袖T恤");

    env.ok("catalog.archiveProduct", json!({"id": id})).await;
    let archived = env
        .ok(
            "catalog.listProducts",
            json!({"filter": {"archived": true}}),
        )
        .await;
    assert_eq!(archived["total"], 1);
    env.ok("catalog.restoreProduct", json!({"id": id})).await;
    env.ok("catalog.deleteProduct", json!({"id": id})).await;
    let error = env.err("catalog.productDetail", json!({"id": id})).await;
    assert_eq!(data_code(&error), "not_found");
}

#[tokio::test]
async fn bad_params_name_the_field() {
    let env = env().await;
    let error = env
        .err("catalog.listProducts", json!({"filter": {"limit": "很多"}}))
        .await;
    assert_eq!(error.code, INVALID_PARAMS);
    assert!(error.message.contains("filter.limit"), "{}", error.message);
    assert_eq!(data_code(&error), "invalid_input");

    let error = env.err("catalog.createProduct", json!({})).await;
    assert_eq!(error.code, INVALID_PARAMS);
    assert!(error.message.contains("input"), "{}", error.message);

    // 业务校验失败是业务错误，不是参数错误
    let error = env
        .err("catalog.createProduct", json!({"input": {"name": ""}}))
        .await;
    assert_eq!(error.code, BUSINESS_ERROR);
    assert_eq!(data_code(&error), "invalid_input");

    for method in ["catalog.nope", "rules.nope", "search.nope", "nope"] {
        assert_eq!(env.err(method, json!({})).await.code, METHOD_NOT_FOUND);
    }
}

#[tokio::test]
async fn import_products_from_csv() {
    let env = env().await;
    let path = env.file(
        "商品.csv",
        "货号,商品名称,类目,成本\nTS-1,纯棉T恤,上衣,25.8\nTS-2,亚麻衬衫,上衣,59\n".as_bytes(),
    );
    let preview = env
        .ok("catalog.previewProductImport", json!({"path": path}))
        .await;
    assert_eq!(preview["totalRows"], 2);
    assert_eq!(preview["headers"][1], "商品名称");

    let result = env
        .ok(
            "catalog.importProducts",
            json!({"path": path, "mapping": preview["mapping"], "saveAs": "供应商表"}),
        )
        .await;
    assert_eq!(result["created"], 2);
    assert_eq!(result["skipped"], 0);
    assert!(result["savedMappingId"].is_string());

    assert_eq!(env.ok("catalog.listProducts", json!({})).await["total"], 2);
    let mappings = env.ok("catalog.listImportMappings", json!({})).await;
    let mapping_id = mappings[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(mappings[0]["name"], "供应商表");
    env.ok("catalog.deleteImportMapping", json!({"id": mapping_id}))
        .await;
    assert_eq!(
        env.ok("catalog.listImportMappings", json!({})).await,
        json!([])
    );

    let error = env
        .err(
            "catalog.previewProductImport",
            json!({"path": env.files.path().join("没有这个文件.xlsx")}),
        )
        .await;
    assert_eq!(error.code, BUSINESS_ERROR);
}

#[tokio::test]
async fn assets_import_tag_star_link_delete() {
    let env = env().await;
    let product = env
        .ok(
            "catalog.createProduct",
            json!({"input": {"name": "纯棉T恤"}}),
        )
        .await;
    let product_id = product["id"].as_str().unwrap().to_owned();
    let good = env.file("主图.png", &png(64, 64, 200));
    let broken = env.file("坏图.png", b"not a png");

    let result = env
        .ok(
            "catalog.importAssetFiles",
            json!({"paths": [good, broken], "productId": product_id, "tags": ["主图"]}),
        )
        .await;
    assert_eq!(result["imported"].as_array().unwrap().len(), 1);
    assert_eq!(result["imported"][0]["deduplicated"], false);
    assert_eq!(result["failed"].as_array().unwrap().len(), 1);
    assert!(
        result["failed"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with("坏图.png")
    );
    let asset = &result["imported"][0]["asset"];
    let id = asset["id"].as_str().unwrap().to_owned();
    // 文件路径是相对数据目录的，文件确实在那里
    let thumb = asset["thumbPath"].as_str().unwrap();
    assert!(!PathBuf::from(thumb).is_absolute());
    assert!(env.ctx.data_dir().join(thumb).exists());

    // 同一张图再导入一次：去重，返回已有的那条
    let again = env
        .ok("catalog.importAssetFiles", json!({"paths": [good]}))
        .await;
    assert_eq!(again["imported"][0]["deduplicated"], true);
    assert_eq!(again["imported"][0]["asset"]["id"], id.as_str());

    let page = env
        .ok(
            "catalog.listAssets",
            json!({"filter": {"productId": product_id}}),
        )
        .await;
    assert_eq!(page["total"], 1);
    assert_eq!(
        env.ok("catalog.listAssetTags", json!({})).await,
        json!([{"tag": "主图", "count": 1}])
    );

    let tagged = env
        .ok(
            "catalog.setAssetTags",
            json!({"id": id, "tags": ["主图", "白底"]}),
        )
        .await;
    assert_eq!(tagged["tags"].as_array().unwrap().len(), 2);
    env.ok("catalog.starAsset", json!({"id": id, "starred": true}))
        .await;
    let starred = env
        .ok(
            "catalog.listAssets",
            json!({"filter": {"starredOnly": true}}),
        )
        .await;
    assert_eq!(starred["total"], 1);
    env.ok("catalog.linkAsset", json!({"id": id, "productId": null}))
        .await;
    let linked = env
        .ok(
            "catalog.listAssets",
            json!({"filter": {"productId": product_id}}),
        )
        .await;
    assert_eq!(linked["total"], 0);

    env.ok("catalog.deleteAsset", json!({"id": id})).await;
    assert_eq!(env.ok("catalog.listAssets", json!({})).await["total"], 0);
}

#[tokio::test]
async fn rules_list_import_and_remove() {
    let env = env().await;
    let overview = env.ok("rules.list", json!({})).await;
    let packs = overview["packs"].as_array().unwrap();
    assert!(packs.iter().any(|row| row["value"]["id"] == "amazon"));
    assert_eq!(overview["problems"], json!([]));

    // 内置的亚马逊规则包提高修订号后导入：盖住内置版本
    let builtin: Value =
        serde_json::from_str(include_str!("../../../rules/packs/amazon.json")).unwrap();
    let mut newer = builtin.clone();
    newer["revision"] = json!(builtin["revision"].as_u64().unwrap() + 1);
    let path = env.file("amazon.json", newer.to_string().as_bytes());
    let imported = env.ok("rules.importFile", json!({"path": path})).await;
    assert_eq!(imported["id"], "amazon");
    assert_eq!(imported["revision"], newer["revision"]);
    let kind = imported["kind"].clone();

    let overview = env.ok("rules.list", json!({})).await;
    let amazon = overview["packs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["value"]["id"] == "amazon")
        .unwrap();
    assert_eq!(amazon["value"]["revision"], newer["revision"]);
    assert_eq!(amazon["shadowedRevision"], builtin["revision"]);

    // 导入比现在旧的版本：拒绝
    let older = env.file("old.json", builtin.to_string().as_bytes());
    let error = env.err("rules.importFile", json!({"path": older})).await;
    assert_eq!(error.code, BUSINESS_ERROR);

    let error = env
        .err(
            "rules.importFile",
            json!({"path": env.file("bad.json", b"{}")}),
        )
        .await;
    assert_eq!(error.code, BUSINESS_ERROR);
    assert_eq!(data_code(&error), "invalid_input");

    assert_eq!(
        env.ok("rules.removeUser", json!({"kind": kind, "id": "amazon"}))
            .await,
        json!({"removed": true})
    );
    assert_eq!(
        env.ok("rules.removeUser", json!({"kind": kind, "id": "amazon"}))
            .await,
        json!({"removed": false})
    );
    let overview = env.ok("rules.list", json!({})).await;
    let amazon = overview["packs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["value"]["id"] == "amazon")
        .unwrap();
    assert_eq!(amazon["value"]["revision"], builtin["revision"]);
}

#[tokio::test]
async fn global_search_groups() {
    let env = env().await;
    env.ok(
        "catalog.createProduct",
        json!({"input": {"name": "纯棉T恤"}}),
    )
    .await;
    let found = env.ok("search.global", json!({"keyword": " T恤 "})).await;
    assert_eq!(found["products"].as_array().unwrap().len(), 1);
    assert_eq!(found["assets"], json!([]));
    assert_eq!(
        env.ok("search.global", json!({"keyword": "   "})).await,
        json!({"products": [], "assets": [], "batches": []})
    );
}
