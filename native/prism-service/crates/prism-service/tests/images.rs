//! 生图 RPC 使用真实数据库、系统凭据和本地模拟接口，核对实际生成文件及重开结果。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use prism_service::{Context, DB_FILE, methods::dispatch};
use prism_store::Store;
use serde_json::json;

#[tokio::test]
async fn invalid_requests_do_not_create_batches() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join(DB_FILE)).await.unwrap();
    let ctx = Context::new(store.clone(), dir.path().into());
    for args in [
        json!({"prompt":"", "count":1}),
        json!({"prompt":"杯子","count":201}),
        json!({"prompt":"杯子","count":0}),
    ] {
        assert!(
            dispatch(
                &ctx,
                "image.submitBatch",
                json!({"owner":"session-test", "args":args})
            )
            .await
            .is_err()
        );
    }
    assert!(
        dispatch(
            &ctx,
            "image.submitBatch",
            json!({"owner":"session-test","fission":true,"args":{"prompt":"杯子","count":1}})
        )
        .await
        .is_err()
    );
    assert_eq!(
        dispatch(&ctx, "image.listBatches", json!({}))
            .await
            .unwrap(),
        json!([])
    );
    store.close().await;
}

// Windows 系统凭据是此用例的被测对象，其他平台由引擎的无凭据用例覆盖。
#[cfg(windows)]
#[tokio::test]
async fn profiles_generation_fission_cancellation_and_reopen() {
    use prism_service::methods::secrets::Secrets;
    use std::{sync::Arc, time::Duration};
    struct Cleanup {
        secrets: Arc<Secrets>,
        key: String,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Err(error) = self.secrets.delete(&self.key) {
                eprintln!("清理测试凭据失败：{}", error.message);
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(mock_image_api::serve(
        listener,
        mock_image_api::MockConfig::default(),
        async {
            let _ = stopped.await;
        },
    ));
    let store = Store::open(&dir.path().join(DB_FILE)).await.unwrap();
    let ctx = Context::new(store.clone(), dir.path().into());
    let profile = dispatch(&ctx, "image.saveProfile", json!({"name":"模拟接口","baseUrl":format!("http://{addr}/v1"),"model":"gpt-image-test","providerType":"openai_images","apiKey":"prism-test-secret","maxConcurrency":2,"timeoutSecs":30})).await.unwrap();
    let id = profile["id"].as_str().unwrap();
    let _credential = Cleanup {
        secrets: ctx.images.secrets.clone(),
        key: format!("image/{id}"),
    };
    assert_eq!(profile["hasKey"], true);
    assert!(!profile.to_string().contains("prism-test-secret"));
    assert_eq!(
        dispatch(&ctx, "image.testProfile", json!({"id":id}))
            .await
            .unwrap()["supported"],
        true
    );
    let batch = dispatch(&ctx, "image.submitBatch", json!({"owner":"dsh-session-without-legacy-conversation","args":{"prompt":"一只蓝色水杯","count":2}})).await.unwrap();
    assert_eq!(batch["owner"], "dsh-session-without-legacy-conversation");
    let batch_id = batch["id"].as_str().unwrap();
    let finished = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let status = dispatch(&ctx, "image.batchStatus", json!({"id":batch_id}))
                .await
                .unwrap();
            if status["status"] == "completed" {
                break status;
            }
            assert_ne!(status["status"], "failed", "{status}");
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(finished["succeeded"], 2);
    let hits = dispatch(
        &ctx,
        "search.global",
        json!({"keyword":format!("#{}",finished["seq"])}),
    )
    .await
    .unwrap();
    assert_eq!(hits["batches"][0]["id"], finished["id"]);
    let asset = &finished["assets"][0];
    assert!(
        dir.path()
            .join(asset["filePath"].as_str().unwrap())
            .is_file()
    );
    // 同一配置切换异步接口后，参考图变体也必须真正完成入库。
    dispatch(&ctx, "image.saveProfile", json!({"id":id,"name":"异步模拟接口","baseUrl":format!("http://{addr}/v1"),"model":"gpt-image-test","providerType":"async_task","maxConcurrency":2,"timeoutSecs":30})).await.unwrap();
    let variant = dispatch(&ctx, "image.submitBatch", json!({"owner":"dsh-session-without-legacy-conversation","fission":true,"args":{"prompt":"白色背景","count":1,"referenceIds":[asset["id"]]}})).await.unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let status = dispatch(&ctx, "image.batchStatus", json!({"id":variant["id"]}))
                .await
                .unwrap();
            if status["status"] == "completed" {
                assert_eq!(status["succeeded"], 1);
                break;
            }
            assert_ne!(status["status"], "failed", "{status}");
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    let fission = dispatch(&ctx, "image.submitBatch", json!({"owner":"dsh-session-without-legacy-conversation","fission":true,"args":{"prompt":"不同场景","count":41,"referenceIds":[asset["id"]]}})).await.unwrap();
    let cancelled = dispatch(&ctx, "image.cancelBatch", json!({"id":fission["id"]}))
        .await
        .unwrap();
    assert_eq!(cancelled["status"], "cancelled");
    ctx.images.stop().await;
    store.close().await;
    let reopened = Store::open(&dir.path().join(DB_FILE)).await.unwrap();
    let again = Context::new(reopened.clone(), dir.path().into());
    assert_eq!(
        dispatch(&again, "image.batchStatus", json!({"id":batch_id}))
            .await
            .unwrap()["assets"],
        finished["assets"]
    );
    // 删除素材不能让整页历史批次失效，计数仍表示已完成的任务数。
    dispatch(&again, "catalog.deleteAsset", json!({"id":asset["id"]}))
        .await
        .unwrap();
    let history = dispatch(&again, "image.batchStatus", json!({"id":batch_id}))
        .await
        .unwrap();
    assert_eq!(history["status"], "completed");
    assert_eq!(history["succeeded"], 2);
    assert!(
        history["assets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["id"] != asset["id"])
    );
    assert!(
        dispatch(&again, "image.listBatches", json!({}))
            .await
            .is_ok()
    );
    dispatch(&again, "image.deleteProfile", json!({"id":id}))
        .await
        .unwrap();
    assert!(
        again
            .images
            .secrets
            .get(&format!("image/{id}"))
            .unwrap()
            .is_none()
    );
    reopened.close().await;
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
}
