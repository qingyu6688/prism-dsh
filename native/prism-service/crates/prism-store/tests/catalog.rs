//! 通用平台表（迁移 0002）的测试：店铺、商品、规格、素材、标签、文案，以及没有原图的任务。

// 测试里的辅助函数出错就该直接失败，用 unwrap 最直接
#![allow(clippy::unwrap_used, clippy::expect_used)]

use prism_core::{BatchStatus, JobKind};
use prism_store::{
    AssetQuery, NewAsset, NewBatch, NewCopy, NewImage, NewJob, NewProduct, NewShop, NewSku,
    ProductQuery, Store, StoreError,
};
use tempfile::TempDir;

const T0: i64 = 1_800_000_000_000;

struct TestDb {
    store: Store,
    _dir: TempDir,
}

async fn open() -> TestDb {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("prism.db")).await.unwrap();
    TestDb { store, _dir: dir }
}

fn product(name: &str, code: Option<&str>) -> NewProduct {
    NewProduct {
        code: code.map(str::to_owned),
        name: name.into(),
        currency: "CNY".into(),
        ..NewProduct::default()
    }
}

fn shop(name: &str, platform: &str) -> NewShop {
    NewShop {
        name: name.into(),
        platform: platform.into(),
        site: Some("US".into()),
        language: "en-US".into(),
        currency: "USD".into(),
        notes: None,
    }
}

fn image_asset(path: &str, sha: &str) -> NewAsset {
    NewAsset {
        kind: "image".into(),
        source: "upload".into(),
        file_path: Some(path.into()),
        sha256: Some(sha.into()),
        mime: Some("image/png".into()),
        ..NewAsset::default()
    }
}

fn tags(list: &[&str]) -> Vec<String> {
    list.iter().map(|t| (*t).to_owned()).collect()
}

fn page() -> ProductQuery {
    ProductQuery {
        limit: 50,
        ..ProductQuery::default()
    }
}

#[tokio::test]
async fn shops_can_be_listed_by_platform() {
    let db = open().await;
    let us = db
        .store
        .create_shop(&shop("美国站", "amazon"), T0)
        .await
        .unwrap();
    db.store
        .create_shop(&shop("Temu 店", "temu"), T0)
        .await
        .unwrap();
    assert_eq!(db.store.list_shops(None).await.unwrap().len(), 2);
    assert_eq!(
        db.store.list_shops(Some("amazon")).await.unwrap()[0].id,
        us.id
    );

    let mut changed = shop("美国站（主）", "amazon");
    changed.site = Some("CA".into());
    let updated = db
        .store
        .update_shop(&us.id, &changed, T0 + 1)
        .await
        .unwrap();
    assert_eq!(updated.site.as_deref(), Some("CA"));
    assert_eq!(updated.updated_at, T0 + 1);
}

#[tokio::test]
async fn product_codes_must_be_unique_when_given() {
    let db = open().await;
    db.store
        .create_product(&product("T 恤", Some("TS-001")), T0)
        .await
        .unwrap();
    let err = db
        .store
        .create_product(&product("另一件", Some("TS-001")), T0)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::Duplicate { .. }));
    assert_eq!(err.to_string(), "商品的编码「TS-001」已经存在");

    // 没填编码的商品可以有很多个
    db.store
        .create_product(&product("无编码 1", None), T0)
        .await
        .unwrap();
    db.store
        .create_product(&product("无编码 2", None), T0)
        .await
        .unwrap();
    assert_eq!(db.store.count_products(&page()).await.unwrap(), 3);
    assert!(
        db.store
            .find_product_by_code("TS-001")
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn products_can_be_searched_and_archived() {
    let db = open().await;
    let mut tee = product("纯棉圆领 T 恤", Some("TS-1"));
    tee.category = Some("女装 > T 恤".into());
    let tee = db.store.create_product(&tee, T0).await.unwrap();
    db.store
        .create_product(&product("手机壳 50%_off", Some("PC-1")), T0 + 1)
        .await
        .unwrap();
    db.store
        .create_product(&product("装饰画", Some("ART-1")), T0 + 2)
        .await
        .unwrap();

    let search = |keyword: &str| ProductQuery {
        keyword: Some(keyword.into()),
        ..page()
    };
    assert_eq!(
        db.store.list_products(&search("T 恤")).await.unwrap().len(),
        1
    );
    assert_eq!(
        db.store.list_products(&search("ts-")).await.unwrap().len(),
        1,
        "编码搜索不区分大小写"
    );
    // % 和 _ 按字面匹配，不当通配符
    assert_eq!(
        db.store.list_products(&search("50%_")).await.unwrap().len(),
        1
    );
    assert_eq!(db.store.list_products(&search("%")).await.unwrap().len(), 1);
    // 最近更新的在前
    let all = db.store.list_products(&page()).await.unwrap();
    assert_eq!(all[0].name, "装饰画");

    db.store
        .set_product_status(&tee.id, "archived", T0 + 3)
        .await
        .unwrap();
    assert_eq!(db.store.count_products(&page()).await.unwrap(), 2);
    let archived = ProductQuery {
        status: Some("archived".into()),
        ..page()
    };
    assert_eq!(
        db.store.list_products(&archived).await.unwrap()[0].id,
        tee.id
    );
}

#[tokio::test]
async fn skus_are_replaced_as_a_whole() {
    let db = open().await;
    let p = db
        .store
        .create_product(&product("T 恤", None), T0)
        .await
        .unwrap();
    let sku = |spec: &str| NewSku {
        code: None,
        spec: spec.into(),
        price_cents: Some(9900),
        cost_cents: None,
        stock: Some(10),
    };
    db.store
        .replace_skus(&p.id, &[sku(r#"{"尺码":"S"}"#), sku(r#"{"尺码":"M"}"#)], T0)
        .await
        .unwrap();
    let skus = db
        .store
        .replace_skus(&p.id, &[sku(r#"{"尺码":"L"}"#)], T0)
        .await
        .unwrap();
    assert_eq!(skus.len(), 1);
    assert_eq!(skus[0].spec, r#"{"尺码":"L"}"#);
    assert!(matches!(
        db.store
            .replace_skus("no-such-product", &[], T0)
            .await
            .unwrap_err(),
        StoreError::NotFound { .. }
    ));
}

#[tokio::test]
async fn assets_are_tagged_filtered_and_deduplicated() {
    let db = open().await;
    let p = db
        .store
        .create_product(&product("T 恤", None), T0)
        .await
        .unwrap();
    let mut main = image_asset("assets/a.png", "sha-a");
    main.product_id = Some(p.id.clone());
    let a = db
        .store
        .create_asset(&main, &tags(&[" 主图 ", "白底", "主图", ""]), T0)
        .await
        .unwrap();
    db.store
        .create_asset(
            &image_asset("assets/b.png", "sha-b"),
            &tags(&["场景"]),
            T0 + 1,
        )
        .await
        .unwrap();
    let text = NewAsset {
        kind: "text".into(),
        source: "generated".into(),
        text_content: Some("100% 纯棉，透气不闷汗".into()),
        ..NewAsset::default()
    };
    db.store
        .create_asset(&text, &tags(&["卖点"]), T0 + 2)
        .await
        .unwrap();

    // 标签去掉首尾空格、去重、忽略空的
    assert_eq!(
        db.store.asset_tags(&a.id).await.unwrap(),
        vec!["主图", "白底"]
    );

    let store = &db.store;
    let query = |q: AssetQuery| async move {
        store
            .list_assets(&AssetQuery { limit: 50, ..q })
            .await
            .unwrap()
    };
    assert_eq!(
        query(AssetQuery {
            tag: Some("主图".into()),
            ..AssetQuery::default()
        })
        .await
        .len(),
        1
    );
    assert_eq!(
        query(AssetQuery {
            product_id: Some(p.id.clone()),
            ..AssetQuery::default()
        })
        .await
        .len(),
        1
    );
    assert_eq!(
        query(AssetQuery {
            kind: Some("image".into()),
            ..AssetQuery::default()
        })
        .await
        .len(),
        2
    );
    assert_eq!(
        query(AssetQuery::default()).await[0].kind,
        "text",
        "最新的在前"
    );

    db.store.set_asset_starred(&a.id, true, T0).await.unwrap();
    assert_eq!(
        query(AssetQuery {
            starred_only: true,
            ..AssetQuery::default()
        })
        .await[0]
            .id,
        a.id
    );

    assert_eq!(
        db.store
            .find_asset_by_sha256("sha-a")
            .await
            .unwrap()
            .unwrap()
            .id,
        a.id
    );
    let counts = db.store.list_asset_tags().await.unwrap();
    assert!(counts.contains(&("主图".to_owned(), 1)));

    db.store
        .set_asset_tags(&a.id, &tags(&["精选"]), T0)
        .await
        .unwrap();
    assert_eq!(db.store.asset_tags(&a.id).await.unwrap(), vec!["精选"]);
}

#[tokio::test]
async fn image_assets_need_a_file() {
    let db = open().await;
    let broken = NewAsset {
        kind: "image".into(),
        source: "upload".into(),
        ..NewAsset::default()
    };
    let err = db.store.create_asset(&broken, &[], T0).await.unwrap_err();
    assert!(matches!(err, StoreError::Invalid(_)), "{err:?}");

    let orphan = NewAsset {
        product_id: Some("no-such-product".into()),
        ..image_asset("x.png", "x")
    };
    let err = db.store.create_asset(&orphan, &[], T0).await.unwrap_err();
    assert!(matches!(err, StoreError::Invalid(_)), "{err:?}");
}

#[tokio::test]
async fn deleting_an_asset_reports_shared_files() {
    let db = open().await;
    let a = db
        .store
        .create_asset(&image_asset("same.png", "s"), &[], T0)
        .await
        .unwrap();
    db.store
        .create_asset(&image_asset("same.png", "s"), &[], T0)
        .await
        .unwrap();
    let deleted = db.store.delete_asset(&a.id).await.unwrap();
    assert_eq!(deleted.file_path.as_deref(), Some("same.png"));
    // 还有一个素材在用这个文件，调用方就不该删文件
    assert_eq!(
        db.store.count_assets_with_file("same.png").await.unwrap(),
        1
    );
}

#[tokio::test]
async fn copies_follow_products_and_survive_shop_deletion() {
    let db = open().await;
    let p = db
        .store
        .create_product(&product("T 恤", None), T0)
        .await
        .unwrap();
    let us = db
        .store
        .create_shop(&shop("美国站", "amazon"), T0)
        .await
        .unwrap();
    let copy = |shop_id: Option<String>, language: &str, field: &str| NewCopy {
        product_id: p.id.clone(),
        shop_id,
        language: language.into(),
        field: field.into(),
        content: "Soft Cotton Tee".into(),
        origin_job_id: None,
    };
    let title = db
        .store
        .create_copy(&copy(Some(us.id.clone()), "en-US", "title"), T0)
        .await
        .unwrap();
    db.store
        .create_copy(&copy(None, "zh-CN", "title"), T0)
        .await
        .unwrap();
    assert_eq!(title.status, "draft");

    assert_eq!(
        db.store
            .list_copies(&p.id, Some(&us.id), None)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.store
            .list_copies(&p.id, None, Some("zh-CN"))
            .await
            .unwrap()
            .len(),
        1
    );

    db.store
        .update_copy(&title.id, "Soft Cotton Crew Tee", "approved", T0 + 1)
        .await
        .unwrap();
    assert!(matches!(
        db.store
            .update_copy(&title.id, "x", "published", T0)
            .await
            .unwrap_err(),
        StoreError::Invalid(_)
    ));
    assert!(matches!(
        db.store
            .create_copy(&copy(None, "en-US", "slogan"), T0)
            .await
            .unwrap_err(),
        StoreError::Invalid(_)
    ));

    // 删店铺：文案保留，不再关联店铺
    db.store.delete_shop(&us.id).await.unwrap();
    let kept = db.store.get_copy(&title.id).await.unwrap();
    assert_eq!(kept.shop_id, None);
    assert_eq!(kept.status, "approved");
}

#[tokio::test]
async fn deleting_a_product_keeps_assets_but_removes_skus_and_copies() {
    let db = open().await;
    let p = db
        .store
        .create_product(&product("T 恤", None), T0)
        .await
        .unwrap();
    db.store
        .replace_skus(
            &p.id,
            &[NewSku {
                code: None,
                spec: "{}".into(),
                price_cents: None,
                cost_cents: None,
                stock: None,
            }],
            T0,
        )
        .await
        .unwrap();
    let mut asset = image_asset("p.png", "p");
    asset.product_id = Some(p.id.clone());
    let asset = db.store.create_asset(&asset, &[], T0).await.unwrap();
    let copy = NewCopy {
        product_id: p.id.clone(),
        shop_id: None,
        language: "zh-CN".into(),
        field: "title".into(),
        content: "标题".into(),
        origin_job_id: None,
    };
    db.store.create_copy(&copy, T0).await.unwrap();

    db.store.delete_product(&p.id).await.unwrap();
    assert!(db.store.list_skus(&p.id).await.unwrap().is_empty());
    assert!(
        db.store
            .list_copies(&p.id, None, None)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        db.store.get_asset(&asset.id).await.unwrap().product_id,
        None
    );
}

#[tokio::test]
async fn jobs_without_a_source_image_run_through_the_queue() {
    // 文案等模块的任务没有原图（迁移 0002 把 job.source_id 改成可空）
    let db = open().await;
    let p = db
        .store
        .create_product(&product("T 恤", None), T0)
        .await
        .unwrap();
    let batch = db
        .store
        .create_batch(
            &NewBatch {
                module: "listing".into(),
                product_id: Some(p.id.clone()),
                conversation_id: None,
                mode: "copy".into(),
                variants_per_source: 1,
                params: "{}".into(),
            },
            T0,
        )
        .await
        .unwrap();
    assert_eq!(batch.module, "listing");
    assert_eq!(batch.mode, "copy");

    // 图裂变批次仍只接受四种模式
    let wrong = db
        .store
        .create_batch(
            &NewBatch {
                module: "image".into(),
                product_id: None,
                conversation_id: None,
                mode: "copy".into(),
                variants_per_source: 4,
                params: "{}".into(),
            },
            T0,
        )
        .await;
    assert!(matches!(wrong, Err(StoreError::Invalid(_))));

    let job = NewJob {
        batch_id: batch.id.clone(),
        source_id: None,
        kind: JobKind::Generate,
        task_type: "listing.copy".into(),
        product_id: Some(p.id.clone()),
        variant_index: 1,
        axis: None,
        instruction: Some("生成英文标题".into()),
        priority: 0,
        max_attempts: 3,
        request_params: None,
    };
    let ids = db.store.insert_jobs(&[job.clone(), job], T0).await.unwrap();
    let queued = db.store.dispatchable_jobs(T0, None, 10).await.unwrap();
    assert_eq!(queued.len(), 2);
    assert_eq!(queued[0].task_type, "listing.copy");
    assert_eq!(queued[0].source_id, None);
    assert_eq!(queued[0].source_seq, 0);

    for id in &ids {
        assert!(db.store.claim_job(id, None, T0).await.unwrap());
    }
    // 没有原图的任务不能写裂变结果图
    let image = NewImage {
        variant_index: 1,
        ref_no: None,
        file_path: "x.png".into(),
        thumb_path: None,
        format: "png".into(),
        width: 1,
        height: 1,
        sha256: "x".into(),
        phash: None,
        dist_to_source: None,
        flags: None,
        actual_params: None,
    };
    assert!(matches!(
        db.store
            .succeed_job(&ids[0], &[image], T0)
            .await
            .unwrap_err(),
        StoreError::Invalid(_)
    ));
    for id in &ids {
        db.store.succeed_job(id, &[], T0).await.unwrap();
    }
    let batch = db.store.get_batch(&batch.id).await.unwrap();
    assert_eq!(batch.status, BatchStatus::Completed);
    assert_eq!(batch.succeeded_jobs, 2);
}

#[tokio::test]
async fn jobs_must_reference_a_source_in_the_same_batch() {
    let db = open().await;
    let new_batch = || NewBatch {
        module: "image".into(),
        product_id: None,
        conversation_id: None,
        mode: "similar".into(),
        variants_per_source: 4,
        params: "{}".into(),
    };
    let a = db.store.create_batch(&new_batch(), T0).await.unwrap();
    let b = db.store.create_batch(&new_batch(), T0).await.unwrap();
    let source = db
        .store
        .insert_source(
            &prism_store::NewSourceImage {
                batch_id: a.id.clone(),
                seq: 1,
                ref_no: None,
                parent_image_id: None,
                file_path: "s.png".into(),
                sha256: "s".into(),
                width: 1,
                height: 1,
                phash: None,
            },
            T0,
        )
        .await
        .unwrap();
    let job = NewJob {
        batch_id: b.id,
        source_id: Some(source.id),
        kind: JobKind::Generate,
        task_type: "image.fission".into(),
        product_id: None,
        variant_index: 1,
        axis: None,
        instruction: None,
        priority: 0,
        max_attempts: 3,
        request_params: None,
    };
    assert!(matches!(
        db.store.insert_jobs(&[job], T0).await.unwrap_err(),
        StoreError::NotFound { .. }
    ));
}

#[tokio::test]
async fn product_summary_counts_related_records() {
    let db = open().await;
    let p = db
        .store
        .create_product(&product("T 恤", Some("TS-9")), T0)
        .await
        .unwrap();
    let empty = db.store.product_summary(&p.id).await.unwrap();
    assert_eq!(empty, prism_store::ProductSummaryRecord::default());

    let sku = |spec: &str| NewSku {
        code: None,
        spec: spec.into(),
        price_cents: None,
        cost_cents: None,
        stock: None,
    };
    db.store
        .replace_skus(
            &p.id,
            &[sku("{\"颜色\":\"红\"}"), sku("{\"颜色\":\"蓝\"}")],
            T0,
        )
        .await
        .unwrap();
    let mut first = image_asset("assets/aa/1.png", "s1");
    first.product_id = Some(p.id.clone());
    let first = db.store.create_asset(&first, &[], T0).await.unwrap();
    db.store
        .set_asset_starred(&first.id, true, T0)
        .await
        .unwrap();
    let text = NewAsset {
        kind: "text".into(),
        source: "generated".into(),
        product_id: Some(p.id.clone()),
        text_content: Some("卖点".into()),
        ..NewAsset::default()
    };
    db.store.create_asset(&text, &[], T0).await.unwrap();
    for (language, field) in [("en-US", "title"), ("en-US", "bullets"), ("zh-CN", "title")] {
        let copy = db
            .store
            .create_copy(
                &NewCopy {
                    product_id: p.id.clone(),
                    shop_id: None,
                    language: language.into(),
                    field: field.into(),
                    content: "内容".into(),
                    origin_job_id: None,
                },
                T0,
            )
            .await
            .unwrap();
        if field == "title" && language == "en-US" {
            db.store
                .update_copy(&copy.id, "内容", "approved", T0)
                .await
                .unwrap();
        }
    }
    for (module, at) in [("image", T0 + 5), ("listing", T0 + 9), ("image", T0 + 7)] {
        db.store
            .create_batch(
                &NewBatch {
                    module: module.into(),
                    product_id: Some(p.id.clone()),
                    conversation_id: None,
                    mode: if module == "image" { "similar" } else { "copy" }.into(),
                    variants_per_source: 1,
                    params: "{}".into(),
                },
                at,
            )
            .await
            .unwrap();
    }

    let summary = db.store.product_summary(&p.id).await.unwrap();
    assert_eq!(summary.sku_count, 2);
    assert_eq!(
        (
            summary.image_assets,
            summary.text_assets,
            summary.starred_assets
        ),
        (1, 1, 1)
    );
    assert_eq!(
        summary.copies_by_language,
        [("en-US".to_owned(), 2, 1), ("zh-CN".to_owned(), 1, 0)]
    );
    assert_eq!(
        summary.batches_by_module,
        [("image".to_owned(), 2), ("listing".to_owned(), 1)]
    );
    assert_eq!(summary.last_batch_at, Some(T0 + 9));
    assert!(matches!(
        db.store.product_summary("nope").await,
        Err(StoreError::NotFound { .. })
    ));
}

#[tokio::test]
async fn asset_pages_come_with_counts_and_tags() {
    let db = open().await;
    let mut ids = Vec::new();
    for i in 0..5 {
        let tag_list = if i % 2 == 0 {
            tags(&["白底", "主图"])
        } else {
            tags(&["场景"])
        };
        let asset = db
            .store
            .create_asset(
                &image_asset(&format!("a/{i}.png"), &format!("h{i}")),
                &tag_list,
                T0 + i,
            )
            .await
            .unwrap();
        ids.push(asset.id);
    }
    let query = AssetQuery {
        tag: Some("白底".into()),
        limit: 2,
        ..AssetQuery::default()
    };
    assert_eq!(db.store.count_assets(&query).await.unwrap(), 3);
    assert_eq!(db.store.list_assets(&query).await.unwrap().len(), 2);

    let pairs = db.store.tags_of_assets(&ids[..2]).await.unwrap();
    let mut expected = vec![
        (ids[0].clone(), "主图".to_owned()),
        (ids[0].clone(), "白底".to_owned()),
        (ids[1].clone(), "场景".to_owned()),
    ];
    expected.sort();
    assert_eq!(pairs, expected);
    assert!(db.store.tags_of_assets(&[]).await.unwrap().is_empty());
}

#[tokio::test]
async fn import_mappings_are_saved_reused_and_counted() {
    let db = open().await;
    let mapping = |name: &str, headers: &str, columns: &str| prism_store::NewImportMapping {
        target: "product".into(),
        name: name.into(),
        headers: headers.into(),
        columns: columns.into(),
        header_line: None,
    };
    let first = db
        .store
        .save_import_mapping(
            &mapping("货品表", r#"["名称","货号"]"#, r#"{"product.name":"名称"}"#),
            T0,
        )
        .await
        .unwrap();
    // 表头完全相同的再保存一次：覆盖原来那条
    let again = db
        .store
        .save_import_mapping(
            &mapping(
                "货品表 v2",
                r#"["名称","货号"]"#,
                r#"{"product.name":"名称","product.code":"货号"}"#,
            ),
            T0 + 1,
        )
        .await
        .unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.name, "货品表 v2");
    let other = db
        .store
        .save_import_mapping(
            &mapping("另一张", r#"["标题"]"#, r#"{"product.name":"标题"}"#),
            T0 + 2,
        )
        .await
        .unwrap();

    db.store
        .mark_import_mapping_used(&first.id, T0 + 5)
        .await
        .unwrap();
    let list = db.store.list_import_mappings("product").await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, first.id, "用过的排前面");
    assert_eq!((list[0].use_count, list[0].last_used_at), (1, Some(T0 + 5)));
    assert!(
        db.store
            .list_import_mappings("report")
            .await
            .unwrap()
            .is_empty()
    );

    db.store.delete_import_mapping(&other.id).await.unwrap();
    assert!(matches!(
        db.store.delete_import_mapping(&other.id).await,
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        db.store.mark_import_mapping_used("nope", T0).await,
        Err(StoreError::NotFound { .. })
    ));
}

#[tokio::test]
async fn search_finds_assets_conversations_and_batches() {
    let db = open().await;
    let titled = NewAsset {
        title: Some("夏季 50%折扣 主图".into()),
        ..image_asset("s/1.png", "s1")
    };
    db.store
        .create_asset(&titled, &tags(&["白底"]), T0)
        .await
        .unwrap();
    db.store
        .create_asset(&image_asset("s/2.png", "s2"), &tags(&["场景图"]), T0 + 1)
        .await
        .unwrap();
    let count = |keyword: &str| {
        let query = AssetQuery {
            keyword: Some(keyword.into()),
            limit: 50,
            ..AssetQuery::default()
        };
        let store = db.store.clone();
        async move { store.count_assets(&query).await.unwrap() }
    };
    // 标题、标签都能搜到；% 当普通字符，不是通配符
    assert_eq!(count("夏季").await, 1);
    assert_eq!(count("场景").await, 1);
    assert_eq!(count("50%").await, 1);
    assert_eq!(count("0%折").await, 1);
    assert_eq!(count("%").await, 1);

    // 会话：标题或消息内容
    let first = db.store.create_conversation("Temu 上架", T0).await.unwrap();
    let second = db
        .store
        .create_conversation("新会话", T0 + 1)
        .await
        .unwrap();
    db.store
        .append_message(
            &second.id,
            "user",
            r#"{"text":"帮我写 Temu 标题"}"#,
            "done",
            T0 + 2,
        )
        .await
        .unwrap();
    let found = db.store.search_conversations("temu", 10).await.unwrap();
    // SQLite 的 LIKE 对英文不分大小写；最近有动静的在前
    assert_eq!(
        found.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        vec![second.id.as_str(), first.id.as_str()]
    );
    assert!(
        db.store
            .search_conversations("  ", 10)
            .await
            .unwrap()
            .is_empty()
    );

    // 批次按批次号
    let batch = db
        .store
        .create_batch(
            &NewBatch {
                module: "listing".into(),
                product_id: None,
                conversation_id: None,
                mode: "copy".into(),
                variants_per_source: 1,
                params: "{}".into(),
            },
            T0,
        )
        .await
        .unwrap();
    let by_seq = db.store.batch_by_seq(batch.seq).await.unwrap().unwrap();
    assert_eq!(by_seq.id, batch.id);
    assert!(
        db.store
            .batch_by_seq(batch.seq + 100)
            .await
            .unwrap()
            .is_none()
    );
}
