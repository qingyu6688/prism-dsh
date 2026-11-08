//! 真实 SQLite、素材文件和本地模拟接口的完整作图链路，不调用外部服务。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use mock_image_api::{MockConfig, MockServer};
use prism_catalog::{Catalog, ImportOptions};
use prism_core::{BatchStatus, now_ms};
use prism_engine::{
    Engine, EngineConfig, EngineEvent, EventSink, JobHandler, JobRequest, KeyedFactory,
};
use prism_pipeline::{ImageArgs, ImagePipeline};
use prism_provider::ApiKey;
use prism_store::{NewApiProfile, Store};
use tempfile::TempDir;

struct Events;
impl EventSink for Events {
    fn emit(&self, _: EngineEvent) {}
}

#[tokio::test]
async fn generation_and_fission_use_image_profiles_and_persist_results() {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("test.db")).await.unwrap();
    let catalog = Arc::new(Catalog::new(store.clone(), dir.path()));
    let pipeline = Arc::new(ImagePipeline::new(catalog.clone()));
    let server = MockServer::start(MockConfig::default()).await.unwrap();
    let profile = store
        .create_profile(
            &NewApiProfile {
                name: "本地生图".into(),
                purpose: "image".into(),
                access_type: "relay".into(),
                provider_type: "openai_images".into(),
                base_url: server.base_url(),
                model: "gpt-image-test".into(),
                model_is_custom: true,
                max_concurrency: 2,
                rpm_limit: None,
                weight: 1,
                timeout_secs: 5,
                transparent_method: "api".into(),
                capabilities: None,
                custom_spec: None,
            },
            now_ms(),
        )
        .await
        .unwrap();
    let conversation = store
        .create_conversation("对话作图", now_ms())
        .await
        .unwrap();
    let args: ImageArgs =
        serde_json::from_value(serde_json::json!({"prompt":"一只蓝色水杯"})).unwrap();
    let generation = pipeline
        .submit(&conversation.id, &args, false)
        .await
        .unwrap();
    assert_eq!(store.get_batch(&generation).await.unwrap().mode, "generate");
    assert_eq!(store.list_jobs(&generation).await.unwrap().len(), 1);
    assert!(
        pipeline
            .submit(&conversation.id, &args, true)
            .await
            .is_err()
    );

    let bytes = prism_imaging::encode_png(&prism_imaging::RgbaImage::from_pixel(
        32,
        32,
        prism_imaging::Rgba([20, 80, 220, 255]),
    ))
    .unwrap();
    let original = catalog
        .import_image_bytes(&bytes, &ImportOptions::default())
        .await
        .unwrap();
    let fission_args = ImageArgs {
        count: 2,
        reference_ids: vec![original.asset.id.clone()],
        ..args
    };
    let fission = pipeline
        .submit(&conversation.id, &fission_args, true)
        .await
        .unwrap();
    let jobs = store.list_jobs(&fission).await.unwrap();
    assert_eq!(jobs.len(), 2);
    let JobRequest::Image(request) = pipeline.build_request(&jobs[0]).await.unwrap() else {
        panic!("应生成图片请求")
    };
    assert_eq!(request.references[0].bytes, bytes);
    assert!(request.prompt.contains("第 1 个"));
    let engine = Engine::start(
        store.clone(),
        pipeline.clone(),
        Arc::new(KeyedFactory::new().with_key(&profile.id, ApiKey::new("local-test"))),
        Arc::new(Events),
        EngineConfig {
            idle_wakeup: Duration::from_millis(20),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    engine.notify();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if store.get_batch(&generation).await.unwrap().status == BatchStatus::Completed
                && store.get_batch(&fission).await.unwrap().status == BatchStatus::Completed
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("作图应在超时前完成");
    for id in [&generation, &fission] {
        let results = pipeline.results(id).await.unwrap();
        assert!(!results.is_empty());
        for id in results {
            let asset = catalog.get_asset(&id).await.unwrap();
            assert_eq!(asset.source, "generated");
            assert!(catalog.asset_path(&asset).unwrap().exists());
        }
    }
    let stats = server.stats();
    assert_eq!(stats.images_generated, 3);
    assert_eq!(
        stats.requests_by_path.get("/v1/images/generations"),
        Some(&1)
    );
    assert_eq!(stats.requests_by_path.get("/v1/images/edits"), Some(&2));
    engine.shutdown().await;
    store.close().await;
}

#[test]
fn rejects_invalid_image_requests() {
    for value in [
        serde_json::json!({"prompt":""}),
        serde_json::json!({"prompt":"水杯","count":201}),
        serde_json::json!({"prompt":"水杯","size":"90000x90000"}),
    ] {
        let args: ImageArgs = serde_json::from_value(value).unwrap();
        assert!(args.validate(false).is_err());
    }
}

#[test]
fn accepts_batch_size_boundaries() {
    for count in [1, 40, 41, 200] {
        let args: ImageArgs =
            serde_json::from_value(serde_json::json!({"prompt":"水杯","count":count})).unwrap();
        assert!(args.validate(false).is_ok());
    }
}
