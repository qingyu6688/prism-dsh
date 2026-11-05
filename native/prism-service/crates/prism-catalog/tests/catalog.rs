//! 商品库的集成测试：真实 SQLite 和临时数据目录。

// 测试里的辅助函数出错就该直接失败，用 unwrap 最直接
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Cursor;
use std::path::Path;

use image::{ImageBuffer, Rgb};
use prism_catalog::{
    AssetFilter, Attribute, Catalog, CatalogError, CopyInput, ImportOptions, ProductFilter,
    ProductInput, ShopInput, SkuInput, TextAssetInput,
};
use prism_core::{ApiError, ApiErrorCode};
use prism_store::{NewAsset, Store, StoreError};
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

fn png(width: u32, height: u32, shade: u8) -> Vec<u8> {
    let buffer: ImageBuffer<Rgb<u8>, Vec<u8>> =
        ImageBuffer::from_pixel(width, height, Rgb([shade, shade, shade]));
    let mut bytes = Vec::new();
    buffer
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    bytes
}

fn invalid_message<T: std::fmt::Debug>(result: Result<T, CatalogError>) -> String {
    match result {
        Err(CatalogError::Invalid(message)) => message,
        other => panic!("应该是输入错误，实际是 {other:?}"),
    }
}

fn product(name: &str) -> ProductInput {
    ProductInput {
        name: name.into(),
        ..ProductInput::default()
    }
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

// ---------- 店铺 ----------

#[tokio::test]
async fn shops_are_normalized_and_filtered_by_platform() {
    let env = env().await;
    let amazon = env
        .catalog
        .create_shop(&ShopInput {
            name: " 美国站 ".into(),
            platform: "Amazon".into(),
            site: Some("us".into()),
            language: "en_us".into(),
            currency: "usd".into(),
            notes: Some("  ".into()),
        })
        .await
        .unwrap();
    assert_eq!(amazon.name, "美国站");
    assert_eq!(amazon.platform, "amazon");
    assert_eq!(amazon.site.as_deref(), Some("US"));
    assert_eq!(
        (amazon.language.as_str(), amazon.currency.as_str()),
        ("en-US", "USD")
    );
    assert_eq!(amazon.notes, None);

    env.catalog
        .create_shop(&ShopInput {
            name: "淘宝店".into(),
            platform: "taobao".into(),
            ..ShopInput::default()
        })
        .await
        .unwrap();
    assert_eq!(env.catalog.list_shops(None).await.unwrap().len(), 2);
    let only = env.catalog.list_shops(Some("AMAZON")).await.unwrap();
    assert_eq!(only.len(), 1);
    assert_eq!(only[0].id, amazon.id);

    let bad = env
        .catalog
        .create_shop(&ShopInput {
            name: "店".into(),
            platform: "tiktok shop".into(),
            ..ShopInput::default()
        })
        .await;
    assert!(invalid_message(bad).contains("平台标识"));
}

// ---------- 商品 ----------

#[tokio::test]
async fn product_fields_round_trip_in_order() {
    let env = env().await;
    let created = env
        .catalog
        .create_product(&ProductInput {
            code: Some(" TS-001 ".into()),
            name: " 纯棉 T 恤 ".into(),
            category: Some("女装 > T 恤".into()),
            selling_points: strings(&["100% 纯棉", "", " 不起球 "]),
            attributes: vec![
                Attribute::new("材质", "棉"),
                Attribute::new("产地", "广州"),
                Attribute::new("厚度", "适中"),
            ],
            cost_cents: Some(2350),
            currency: "cny".into(),
            target_markets: strings(&["us", "DE", "US"]),
            ..ProductInput::default()
        })
        .await
        .unwrap();
    assert_eq!(created.code.as_deref(), Some("TS-001"));
    assert_eq!(created.name, "纯棉 T 恤");
    assert_eq!(created.selling_points, ["100% 纯棉", "不起球"]);
    // 参数按填写顺序，不按名称排序
    let names: Vec<&str> = created.attributes.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["材质", "产地", "厚度"]);
    assert_eq!(created.target_markets, ["US", "DE"]);
    assert!(!created.archived);
    assert_eq!(env.catalog.get_product(&created.id).await.unwrap(), created);

    let updated = env
        .catalog
        .update_product(&created.id, &product("改名"))
        .await
        .unwrap();
    assert_eq!(updated.name, "改名");
    assert!(updated.selling_points.is_empty() && updated.attributes.is_empty());
    assert_eq!(updated.code, None);
}

#[tokio::test]
async fn product_input_errors_name_the_field() {
    let env = env().await;
    let cases: Vec<(ProductInput, &str)> = vec![
        (product("  "), "商品名称不能为空"),
        (
            ProductInput {
                selling_points: vec!["a".into(), "长".repeat(201)],
                ..product("T 恤")
            },
            "卖点第 2 条超过 200 字",
        ),
        (
            ProductInput {
                attributes: vec![Attribute::new("材质", "棉"), Attribute::new("材质", "麻")],
                ..product("T 恤")
            },
            "参数「材质」重复了",
        ),
        (
            ProductInput {
                cost_cents: Some(-1),
                ..product("T 恤")
            },
            "成本不能是负数",
        ),
        (
            ProductInput {
                target_markets: strings(&["USA"]),
                ..product("T 恤")
            },
            "目标市场「USA」应该是两位国家或地区代码，如 US、DE",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(
            invalid_message(env.catalog.create_product(&input).await),
            expected
        );
    }

    env.catalog
        .create_product(&ProductInput {
            code: Some("TS-001".into()),
            ..product("A")
        })
        .await
        .unwrap();
    let duplicate = env
        .catalog
        .create_product(&ProductInput {
            code: Some("TS-001".into()),
            ..product("B")
        })
        .await
        .unwrap_err();
    assert!(matches!(
        duplicate,
        CatalogError::Store(StoreError::Duplicate { .. })
    ));
    let api: ApiError = duplicate.into();
    assert_eq!(api.code, ApiErrorCode::InvalidInput);
    assert_eq!(api.message, "商品的编码「TS-001」已经存在");
}

#[tokio::test]
async fn products_are_paged_searched_and_archived() {
    let env = env().await;
    let mut ids = Vec::new();
    for i in 0..5 {
        let p = env
            .catalog
            .create_product(&product(&format!("连衣裙 {i}")))
            .await
            .unwrap();
        ids.push(p.id);
    }
    env.catalog.create_product(&product("帽子")).await.unwrap();

    let page = env
        .catalog
        .list_products(&ProductFilter {
            keyword: Some(" 连衣裙 ".into()),
            limit: 2,
            ..ProductFilter::default()
        })
        .await
        .unwrap();
    assert_eq!((page.items.len(), page.total), (2, 5));

    env.catalog.archive_product(&ids[0]).await.unwrap();
    let active = env
        .catalog
        .list_products(&ProductFilter::default())
        .await
        .unwrap();
    assert_eq!(active.total, 5);
    let archived = env
        .catalog
        .list_products(&ProductFilter {
            archived: true,
            ..ProductFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(archived.total, 1);
    assert!(archived.items[0].archived);
    env.catalog.restore_product(&ids[0]).await.unwrap();
    assert!(!env.catalog.get_product(&ids[0]).await.unwrap().archived);

    assert!(matches!(
        env.catalog.archive_product("nope").await,
        Err(CatalogError::Store(StoreError::NotFound { .. }))
    ));
}

// ---------- 规格 ----------

fn sku(code: Option<&str>, spec: &[(&str, &str)]) -> SkuInput {
    SkuInput {
        code: code.map(str::to_owned),
        spec: spec.iter().map(|(n, v)| Attribute::new(*n, *v)).collect(),
        price_cents: Some(5900),
        ..SkuInput::default()
    }
}

#[tokio::test]
async fn skus_keep_spec_order_and_reject_duplicates() {
    let env = env().await;
    let p = env.catalog.create_product(&product("T 恤")).await.unwrap();
    let skus = env
        .catalog
        .replace_skus(
            &p.id,
            &[
                sku(Some("TS-R-M"), &[("颜色", "红"), ("尺码", "M")]),
                sku(Some("TS-R-L"), &[("颜色", "红"), ("尺码", "L")]),
            ],
        )
        .await
        .unwrap();
    assert_eq!(skus.len(), 2);
    assert_eq!(
        skus[0].spec,
        [Attribute::new("颜色", "红"), Attribute::new("尺码", "M")]
    );
    assert_eq!(skus[1].sort_order, 1);

    let same_spec = env
        .catalog
        .replace_skus(
            &p.id,
            &[sku(None, &[("颜色", "红")]), sku(None, &[("颜色", " 红 ")])],
        )
        .await;
    assert_eq!(invalid_message(same_spec), "第 2 个规格和第 1 个重复了");

    let same_code = env
        .catalog
        .replace_skus(
            &p.id,
            &[
                sku(Some("A"), &[("颜色", "红")]),
                sku(Some("A"), &[("颜色", "蓝")]),
            ],
        )
        .await;
    assert_eq!(
        invalid_message(same_code),
        "第 2 个规格的编码「A」和第 1 个重复了"
    );

    let empty = env.catalog.replace_skus(&p.id, &[sku(None, &[])]).await;
    assert!(invalid_message(empty).contains("没有填规格"));

    let mut negative = sku(None, &[("颜色", "红")]);
    negative.stock = Some(-3);
    assert_eq!(
        invalid_message(env.catalog.replace_skus(&p.id, &[negative]).await),
        "第 1 个规格的库存不能是负数"
    );

    // 校验失败不改动原来的规格
    let detail = env.catalog.product_detail(&p.id).await.unwrap();
    assert_eq!(detail.skus.len(), 2);
    assert_eq!(detail.summary.sku_count, 2);
}

// ---------- 素材 ----------

#[tokio::test]
async fn importing_an_image_stores_it_by_content() {
    let env = env().await;
    let p = env.catalog.create_product(&product("T 恤")).await.unwrap();
    let file = env.dir.path().join("主图 1.png");
    std::fs::write(&file, png(40, 30, 200)).unwrap();

    let first = env
        .catalog
        .import_image(
            &file,
            &ImportOptions {
                tags: strings(&["白底", "主图"]),
                ..ImportOptions::default()
            },
        )
        .await
        .unwrap();
    assert!(!first.deduplicated);
    let asset = &first.asset;
    assert_eq!(asset.kind, "image");
    assert_eq!(asset.title.as_deref(), Some("主图 1"));
    assert_eq!(asset.mime.as_deref(), Some("image/png"));
    assert_eq!((asset.width, asset.height), (Some(40), Some(30)));
    let sha = asset.sha256.clone().unwrap();
    let relative = asset.file_path.clone().unwrap();
    assert_eq!(relative, format!("assets/{}/{sha}.png", &sha[..2]));
    let stored = env.catalog.asset_path(asset).unwrap();
    assert_eq!(
        std::fs::read(&stored).unwrap(),
        std::fs::read(&file).unwrap()
    );
    // 旁边是缩略图，没留下临时文件
    let thumb = env.dir.path().join(asset.thumb_path.as_deref().unwrap());
    assert_eq!(thumb, stored.with_file_name(format!("{sha}.thumb.webp")));
    let thumb_info = prism_imaging::probe(&std::fs::read(&thumb).unwrap()).unwrap();
    assert_eq!(
        (thumb_info.width, thumb_info.height),
        (40, 30),
        "小图不放大"
    );
    let siblings = std::fs::read_dir(stored.parent().unwrap()).unwrap().count();
    assert_eq!(siblings, 2);
    assert_eq!(asset.phash.as_deref().map(str::len), Some(16));

    // 同样内容换个文件名再导入：返回已有的，顺带关联商品、合并标签
    let copy = env.dir.path().join("另一个名字.png");
    std::fs::copy(&file, &copy).unwrap();
    let second = env
        .catalog
        .import_image(
            &copy,
            &ImportOptions {
                product_id: Some(p.id.clone()),
                tags: strings(&["主图", "夏季"]),
                ..ImportOptions::default()
            },
        )
        .await
        .unwrap();
    assert!(second.deduplicated);
    assert_eq!(second.asset.id, asset.id);
    assert_eq!(second.asset.product_id.as_deref(), Some(p.id.as_str()));
    let mut tags = second.asset.tags.clone();
    tags.sort();
    assert_eq!(tags, ["主图", "夏季", "白底"]);
    let page = env
        .catalog
        .list_assets(&AssetFilter::default())
        .await
        .unwrap();
    assert_eq!(page.total, 1);
}

#[tokio::test]
async fn bad_images_are_rejected_without_leaving_files() {
    let env = env().await;
    let assets_dir = env.dir.path().join("assets");

    let text = env.dir.path().join("不是图.png");
    std::fs::write(&text, "hello").unwrap();
    let error = env
        .catalog
        .import_image(&text, &ImportOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(error, CatalogError::Imaging(_)));
    let api: ApiError = error.into();
    assert_eq!(api.code, ApiErrorCode::InvalidInput);
    assert_eq!(api.message, "不支持的图片格式，只支持 PNG、JPEG、WebP");

    let empty = env.dir.path().join("空.png");
    std::fs::write(&empty, b"").unwrap();
    assert_eq!(
        invalid_message(
            env.catalog
                .import_image(&empty, &ImportOptions::default())
                .await
        ),
        "图片文件是空的"
    );

    // 超过 20 MB：看文件大小就拒绝，不读内容
    let big = env.dir.path().join("大.png");
    let file = std::fs::File::create(&big).unwrap();
    file.set_len(prism_catalog::MAX_IMAGE_BYTES + 1).unwrap();
    drop(file);
    assert_eq!(
        invalid_message(
            env.catalog
                .import_image(&big, &ImportOptions::default())
                .await
        ),
        "图片超过 20 MB，请压缩后再导入"
    );

    let missing = env
        .catalog
        .import_image(Path::new("D:/不存在/x.png"), &ImportOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(missing, CatalogError::ReadFile { .. }));
    assert_eq!(ApiError::from(missing).code, ApiErrorCode::InvalidInput);

    // 商品不存在：在写文件之前就报错
    let no_product = env
        .catalog
        .import_image_bytes(
            &png(4, 4, 1),
            &ImportOptions {
                product_id: Some("nope".into()),
                ..ImportOptions::default()
            },
        )
        .await;
    assert!(matches!(
        no_product,
        Err(CatalogError::Store(StoreError::NotFound { .. }))
    ));
    assert_eq!(
        invalid_message(
            env.catalog
                .import_image_bytes(
                    &png(4, 4, 1),
                    &ImportOptions {
                        source: "camera".into(),
                        ..ImportOptions::default()
                    }
                )
                .await
        ),
        "素材来源「camera」不支持，可选：upload / generated / captured / imported"
    );
    assert!(!assets_dir.exists());
}

#[tokio::test]
async fn concurrent_imports_of_one_image_create_one_asset() {
    let env = env().await;
    let bytes = png(16, 16, 9);
    let options = ImportOptions::default();
    let results = import_four_times_at_once(&env.catalog, &bytes, &options).await;
    let created = results.iter().filter(|r| !r.deduplicated).count();
    assert_eq!(created, 1);
    assert!(results.iter().all(|r| r.asset.id == results[0].asset.id));
}

async fn import_four_times_at_once(
    catalog: &Catalog,
    bytes: &[u8],
    options: &ImportOptions,
) -> Vec<prism_catalog::ImportedAsset> {
    let (a, b, c, d) = tokio::join!(
        catalog.import_image_bytes(bytes, options),
        catalog.import_image_bytes(bytes, options),
        catalog.import_image_bytes(bytes, options),
        catalog.import_image_bytes(bytes, options),
    );
    vec![a.unwrap(), b.unwrap(), c.unwrap(), d.unwrap()]
}

#[tokio::test]
async fn deleting_assets_removes_files_nobody_uses() {
    let env = env().await;
    let imported = env
        .catalog
        .import_image_bytes(&png(8, 8, 3), &ImportOptions::default())
        .await
        .unwrap()
        .asset;
    let path = env.catalog.asset_path(&imported).unwrap();
    // 另一条记录引用同一个文件（比如以后从生成结果直接登记）
    let other = env
        .catalog
        .store()
        .create_asset(
            &NewAsset {
                kind: "image".into(),
                source: "generated".into(),
                file_path: imported.file_path.clone(),
                ..NewAsset::default()
            },
            &[],
            1,
        )
        .await
        .unwrap();

    env.catalog.delete_asset(&imported.id).await.unwrap();
    assert!(path.exists(), "还有素材在用，文件不能删");
    env.catalog.delete_asset(&other.id).await.unwrap();
    assert!(!path.exists());
    assert!(
        !path.with_extension("thumb.webp").exists(),
        "缩略图一起删掉"
    );
    assert!(matches!(
        env.catalog.delete_asset(&other.id).await,
        Err(CatalogError::Store(StoreError::NotFound { .. }))
    ));
}

#[tokio::test]
async fn assets_are_listed_with_tags_and_filters() {
    let env = env().await;
    let p = env.catalog.create_product(&product("T 恤")).await.unwrap();
    for shade in 0..3u8 {
        env.catalog
            .import_image_bytes(
                &png(4, 4, shade),
                &ImportOptions {
                    product_id: (shade == 0).then(|| p.id.clone()),
                    tags: if shade < 2 {
                        strings(&["白底  主图"])
                    } else {
                        strings(&["场景"])
                    },
                    ..ImportOptions::default()
                },
            )
            .await
            .unwrap();
    }
    let text = env
        .catalog
        .add_text_asset(&TextAssetInput {
            product_id: Some(p.id.clone()),
            content: " 夏季清凉 ".into(),
            tags: strings(&["卖点"]),
            ..TextAssetInput::default()
        })
        .await
        .unwrap();
    assert_eq!(text.text_content.as_deref(), Some("夏季清凉"));
    env.catalog.star_asset(&text.id, true).await.unwrap();

    let all = env
        .catalog
        .list_assets(&AssetFilter::default())
        .await
        .unwrap();
    assert_eq!(all.total, 4);
    assert!(all.items.iter().all(|a| a.tags.len() == 1));

    let tagged = env
        .catalog
        .list_assets(&AssetFilter {
            tag: Some("白底 \t主图".into()),
            ..AssetFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(tagged.total, 2);
    assert_eq!(tagged.items[0].tags, ["白底 主图"]);

    let of_product = env
        .catalog
        .list_assets(&AssetFilter {
            product_id: Some(p.id.clone()),
            kind: Some("image".into()),
            ..AssetFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(of_product.total, 1);
    let starred = env
        .catalog
        .list_assets(&AssetFilter {
            starred_only: true,
            ..AssetFilter::default()
        })
        .await
        .unwrap();
    assert_eq!(starred.items[0].id, text.id);
    assert!(
        invalid_message(
            env.catalog
                .list_assets(&AssetFilter {
                    kind: Some("video".into()),
                    ..AssetFilter::default()
                })
                .await
        )
        .starts_with("素材类型「video」不支持")
    );

    let retagged = env
        .catalog
        .set_asset_tags(&text.id, &strings(&["卖点", "夏季", "夏季"]))
        .await
        .unwrap();
    assert_eq!(retagged.tags, ["卖点", "夏季"]);
    let tags = env.catalog.list_tags().await.unwrap();
    assert_eq!(tags[0], ("白底 主图".to_owned(), 2));

    env.catalog.link_asset(&text.id, None).await.unwrap();
    assert_eq!(
        env.catalog.get_asset(&text.id).await.unwrap().product_id,
        None
    );
    let detail = env.catalog.product_detail(&p.id).await.unwrap();
    assert_eq!(
        (detail.summary.image_assets, detail.summary.text_assets),
        (1, 0)
    );
}

// ---------- 文案 ----------

#[tokio::test]
async fn copies_go_back_to_draft_when_edited() {
    let env = env().await;
    let p = env.catalog.create_product(&product("T 恤")).await.unwrap();
    let shop = env
        .catalog
        .create_shop(&ShopInput {
            name: "美国站".into(),
            platform: "amazon".into(),
            ..ShopInput::default()
        })
        .await
        .unwrap();
    let copy = env
        .catalog
        .save_copy(&CopyInput {
            product_id: p.id.clone(),
            shop_id: Some(shop.id.clone()),
            language: "en_us".into(),
            field: "title".into(),
            content: " Cotton T-Shirt ".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        (copy.language.as_str(), copy.content.as_str()),
        ("en-US", "Cotton T-Shirt")
    );
    assert!(!copy.approved);

    let approved = env.catalog.approve_copy(&copy.id).await.unwrap();
    assert!(approved.approved);
    // 内容没变，不影响确认状态
    assert!(
        env.catalog
            .update_copy(&copy.id, "Cotton T-Shirt ")
            .await
            .unwrap()
            .approved
    );
    let edited = env
        .catalog
        .update_copy(&copy.id, "Soft Cotton Tee")
        .await
        .unwrap();
    assert!(!edited.approved);
    assert_eq!(edited.content, "Soft Cotton Tee");

    assert_eq!(
        env.catalog
            .list_copies(&p.id, None, Some("EN-us"))
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        env.catalog
            .list_copies(&p.id, None, Some("zh-CN"))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        invalid_message(
            env.catalog
                .save_copy(&CopyInput {
                    product_id: p.id.clone(),
                    language: "en".into(),
                    field: "slogan".into(),
                    content: "x".into(),
                    ..CopyInput::default()
                })
                .await
        )
        .starts_with("文案字段「slogan」不支持")
    );
    assert!(matches!(
        env.catalog
            .save_copy(&CopyInput {
                product_id: "nope".into(),
                language: "en".into(),
                field: "title".into(),
                content: "x".into(),
                ..CopyInput::default()
            })
            .await,
        Err(CatalogError::Store(StoreError::NotFound { .. }))
    ));

    // 删店铺后文案保留，变成通用文案
    env.catalog.delete_shop(&shop.id).await.unwrap();
    let copies = env.catalog.list_copies(&p.id, None, None).await.unwrap();
    assert_eq!(copies[0].shop_id, None);
    env.catalog.delete_copy(&copy.id).await.unwrap();
    assert!(
        env.catalog
            .list_copies(&p.id, None, None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn deleting_a_product_keeps_its_assets() {
    let env = env().await;
    let p = env.catalog.create_product(&product("T 恤")).await.unwrap();
    let asset = env
        .catalog
        .import_image_bytes(
            &png(4, 4, 7),
            &ImportOptions {
                product_id: Some(p.id.clone()),
                ..ImportOptions::default()
            },
        )
        .await
        .unwrap()
        .asset;
    env.catalog.delete_product(&p.id).await.unwrap();
    let kept = env.catalog.get_asset(&asset.id).await.unwrap();
    assert_eq!(kept.product_id, None);
    assert!(env.catalog.asset_path(&kept).unwrap().exists());
}
