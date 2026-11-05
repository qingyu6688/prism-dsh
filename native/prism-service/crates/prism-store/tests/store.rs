//! prism-store 的集成测试：每个测试用一个临时数据库文件。

// 测试里的辅助函数出错就该直接失败，用 unwrap 最直接
#![allow(clippy::unwrap_used, clippy::expect_used)]

use prism_core::{BatchStatus, JobKind, JobStatus};
use prism_store::{NewApiProfile, NewBatch, NewImage, NewJob, NewSourceImage, Store, StoreError};
use tempfile::TempDir;

const T0: i64 = 1_800_000_000_000;

struct TestDb {
    store: Store,
    // 持有临时目录，测试结束时自动删除
    _dir: TempDir,
}

async fn open() -> TestDb {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("prism.db")).await.unwrap();
    TestDb { store, _dir: dir }
}

fn new_batch() -> NewBatch {
    NewBatch {
        module: "image".into(),
        product_id: None,
        conversation_id: None,
        mode: "similar".into(),
        variants_per_source: 4,
        params: "{}".into(),
    }
}

fn new_source(batch_id: &str, seq: i64) -> NewSourceImage {
    NewSourceImage {
        batch_id: batch_id.into(),
        seq,
        ref_no: Some(format!("S{seq}")),
        parent_image_id: None,
        file_path: format!("images/sources/{seq}.png"),
        sha256: format!("hash-{seq}"),
        width: 1024,
        height: 1024,
        phash: None,
    }
}

fn job(batch_id: &str, source_id: &str, kind: JobKind, variant_index: i64) -> NewJob {
    NewJob {
        batch_id: batch_id.into(),
        source_id: Some(source_id.into()),
        kind,
        task_type: "image.fission".into(),
        product_id: None,
        variant_index,
        axis: None,
        instruction: None,
        priority: 0,
        max_attempts: 3,
        request_params: None,
    }
}

fn image(variant_index: i64) -> NewImage {
    NewImage {
        variant_index,
        ref_no: None,
        file_path: format!("images/results/{variant_index}.png"),
        thumb_path: None,
        format: "png".into(),
        width: 1024,
        height: 1024,
        sha256: format!("result-{variant_index}"),
        phash: None,
        dist_to_source: None,
        flags: None,
        actual_params: None,
    }
}

/// 建一个批次：`sources` 张原图，每张 `variants` 个生图任务。返回 (批次 ID, 原图 ID 列表)。
async fn seed(store: &Store, sources: i64, variants: i64) -> (String, Vec<String>) {
    let batch = store.create_batch(&new_batch(), T0).await.unwrap();
    let mut source_ids = Vec::new();
    let mut jobs = Vec::new();
    for seq in 1..=sources {
        let source = store
            .insert_source(&new_source(&batch.id, seq), T0)
            .await
            .unwrap();
        for v in 1..=variants {
            jobs.push(job(&batch.id, &source.id, JobKind::Generate, v));
        }
        source_ids.push(source.id);
    }
    store.insert_jobs(&jobs, T0).await.unwrap();
    (batch.id, source_ids)
}

/// 把派发列表转成 "S原图序号-变体序号"，方便断言顺序。
async fn dispatch_order(store: &Store, now: i64) -> Vec<String> {
    store
        .dispatchable_jobs(now, None, 100)
        .await
        .unwrap()
        .iter()
        .map(|j| match j.kind {
            JobKind::Plan => format!("P{}", j.source_seq),
            JobKind::Generate => format!("S{}-{}", j.source_seq, j.variant_index),
        })
        .collect()
}

async fn claim_all(store: &Store, now: i64) -> Vec<String> {
    let jobs = store.dispatchable_jobs(now, None, 1000).await.unwrap();
    let mut ids = Vec::new();
    for j in jobs {
        assert!(store.claim_job(&j.id, None, now).await.unwrap());
        ids.push(j.id);
    }
    ids
}

#[tokio::test]
async fn reopening_runs_migrations_only_once() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("prism.db");
    let store = Store::open(&path).await.unwrap();
    store.create_batch(&new_batch(), T0).await.unwrap();
    store.close().await;

    let store = Store::open(&path).await.unwrap();
    assert_eq!(store.list_batches(10, 0).await.unwrap().len(), 1);
}

#[tokio::test]
async fn schema_version_is_latest_after_open() {
    let db = open().await;
    assert_eq!(
        db.store.schema_version().await.unwrap(),
        Store::LATEST_SCHEMA_VERSION
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_batches_get_unique_sequence_numbers() {
    let db = open().await;
    let mut handles = Vec::new();
    for _ in 0..10 {
        let store = db.store.clone();
        handles.push(tokio::spawn(async move {
            store.create_batch(&new_batch(), T0).await.unwrap().seq
        }));
    }
    let mut seqs = Vec::new();
    for handle in handles {
        seqs.push(handle.await.unwrap());
    }
    seqs.sort();
    assert_eq!(seqs, (1..=10).collect::<Vec<_>>());
}

#[tokio::test]
async fn dispatch_order_round_robins_across_sources() {
    // UT-E11：2 张原图各 4 个任务，顺序为 S1-1、S2-1、S1-2、S2-2…
    let db = open().await;
    seed(&db.store, 2, 4).await;
    assert_eq!(
        dispatch_order(&db.store, T0).await,
        vec![
            "S1-1", "S2-1", "S1-2", "S2-2", "S1-3", "S2-3", "S1-4", "S2-4"
        ]
    );
}

#[tokio::test]
async fn plan_jobs_and_interactive_jobs_go_first() {
    // UT-E12：交互优先级的任务后入队也排在最前；规划任务排在生图任务前
    let db = open().await;
    let (batch_id, sources) = seed(&db.store, 2, 2).await;
    db.store
        .insert_jobs(&[job(&batch_id, &sources[1], JobKind::Plan, 0)], T0)
        .await
        .unwrap();
    let mut urgent = job(&batch_id, &sources[1], JobKind::Generate, 9);
    urgent.priority = 10;
    db.store.insert_jobs(&[urgent], T0).await.unwrap();

    let order = dispatch_order(&db.store, T0).await;
    assert_eq!(&order[..3], &["S2-9", "P2", "S1-1"]);
}

#[tokio::test]
async fn earlier_batches_run_first() {
    let db = open().await;
    seed(&db.store, 1, 1).await;
    let (second, _) = seed(&db.store, 1, 1).await;
    let jobs = db.store.dispatchable_jobs(T0, None, 10).await.unwrap();
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[1].batch_id, second);
}

#[tokio::test]
async fn retry_wait_jobs_wait_until_due() {
    let db = open().await;
    seed(&db.store, 1, 1).await;
    let id = claim_all(&db.store, T0).await.remove(0);
    db.store
        .schedule_retry(&id, T0 + 5_000, "upstream_error", "上游 502", T0)
        .await
        .unwrap();

    assert!(
        db.store
            .dispatchable_jobs(T0 + 4_999, None, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(db.store.next_retry_at().await.unwrap(), Some(T0 + 5_000));
    assert_eq!(
        db.store
            .dispatchable_jobs(T0 + 5_000, None, 10)
            .await
            .unwrap()
            .len(),
        1
    );

    assert!(db.store.claim_job(&id, None, T0 + 5_000).await.unwrap());
    let job = db.store.get_job(&id).await.unwrap();
    assert_eq!(job.attempt, 2);
    assert_eq!(job.next_run_at, None);
}

#[tokio::test]
async fn paused_batches_are_not_dispatched() {
    let db = open().await;
    let (batch_id, _) = seed(&db.store, 1, 2).await;
    db.store
        .pause_batch(&batch_id, "high_failure_rate", T0)
        .await
        .unwrap();
    assert!(
        db.store
            .dispatchable_jobs(T0, None, 10)
            .await
            .unwrap()
            .is_empty()
    );

    db.store.resume_batch(&batch_id, T0).await.unwrap();
    assert_eq!(
        db.store
            .dispatchable_jobs(T0, None, 10)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_one_worker_can_claim_a_job() {
    let db = open().await;
    seed(&db.store, 1, 1).await;
    let id = db.store.dispatchable_jobs(T0, None, 1).await.unwrap()[0]
        .id
        .clone();

    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = db.store.clone();
        let id = id.clone();
        handles.push(tokio::spawn(async move {
            store.claim_job(&id, None, T0).await.unwrap()
        }));
    }
    let mut wins = 0;
    for handle in handles {
        if handle.await.unwrap() {
            wins += 1;
        }
    }
    assert_eq!(wins, 1);
    let job = db.store.get_job(&id).await.unwrap();
    assert_eq!(job.status, JobStatus::Running);
    assert_eq!(job.attempt, 1);
}

#[tokio::test]
async fn rate_limited_requeue_refunds_the_attempt() {
    // UT-E09：429 不计入尝试次数
    let db = open().await;
    seed(&db.store, 1, 1).await;
    let id = claim_all(&db.store, T0).await.remove(0);
    db.store.requeue_job(&id, true, T0).await.unwrap();
    let job = db.store.get_job(&id).await.unwrap();
    assert_eq!(job.status, JobStatus::Queued);
    assert_eq!(job.attempt, 0);
}

#[tokio::test]
async fn batch_completes_when_every_job_is_done() {
    let db = open().await;
    let (batch_id, _) = seed(&db.store, 1, 3).await;
    let ids = claim_all(&db.store, T0).await;

    let image_ids = db
        .store
        .succeed_job(&ids[0], &[image(1)], T0)
        .await
        .unwrap();
    assert_eq!(image_ids.len(), 1);
    db.store
        .succeed_job(&ids[1], &[image(2)], T0)
        .await
        .unwrap();
    let batch = db.store.get_batch(&batch_id).await.unwrap();
    assert_eq!(batch.status, BatchStatus::Running);

    db.store
        .fail_job(&ids[2], "bad_request", "尺寸不合法", T0 + 1)
        .await
        .unwrap();
    let batch = db.store.get_batch(&batch_id).await.unwrap();
    assert_eq!(batch.total_jobs, 3);
    assert_eq!(batch.succeeded_jobs, 2);
    assert_eq!(batch.failed_jobs, 1);
    assert_eq!(batch.status, BatchStatus::Completed);
    assert_eq!(batch.finished_at, Some(T0 + 1));
}

#[tokio::test]
async fn batch_fails_when_nothing_succeeded() {
    let db = open().await;
    let (batch_id, _) = seed(&db.store, 1, 2).await;
    for id in claim_all(&db.store, T0).await {
        db.store
            .fail_job(&id, "auth", "Key 无效", T0)
            .await
            .unwrap();
    }
    assert_eq!(
        db.store.get_batch(&batch_id).await.unwrap().status,
        BatchStatus::Failed
    );
}

#[tokio::test]
async fn results_of_cancelled_jobs_are_rejected() {
    let db = open().await;
    let (batch_id, _) = seed(&db.store, 1, 2).await;
    let jobs = db.store.dispatchable_jobs(T0, None, 10).await.unwrap();
    // 第一个任务已发出，第二个还在排队
    assert!(db.store.claim_job(&jobs[0].id, None, T0).await.unwrap());

    let cancelled = db.store.cancel_batch(&batch_id, T0).await.unwrap();
    assert_eq!(cancelled, 1);
    assert_eq!(
        db.store.get_job(&jobs[1].id).await.unwrap().status,
        JobStatus::Cancelled
    );

    // 在途的那个完成了，结果照常入库，批次保持已取消
    db.store
        .succeed_job(&jobs[0].id, &[image(1)], T0)
        .await
        .unwrap();
    let batch = db.store.get_batch(&batch_id).await.unwrap();
    assert_eq!(batch.status, BatchStatus::Cancelled);
    assert_eq!(batch.succeeded_jobs, 1);

    // 已取消的任务不能再写结果
    let err = db
        .store
        .succeed_job(&jobs[1].id, &[image(2)], T0)
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::StateChanged { .. }));
}

#[tokio::test]
async fn cancelling_the_last_pending_job_finishes_the_batch() {
    let db = open().await;
    let (batch_id, _) = seed(&db.store, 1, 2).await;
    let jobs = db.store.dispatchable_jobs(T0, None, 10).await.unwrap();
    assert!(db.store.claim_job(&jobs[0].id, None, T0).await.unwrap());
    db.store
        .succeed_job(&jobs[0].id, &[image(1)], T0)
        .await
        .unwrap();

    db.store.cancel_job(&jobs[1].id, T0).await.unwrap();
    assert_eq!(
        db.store.get_batch(&batch_id).await.unwrap().status,
        BatchStatus::Completed
    );
}

#[tokio::test]
async fn plan_success_derives_generate_jobs() {
    let db = open().await;
    let batch = db.store.create_batch(&new_batch(), T0).await.unwrap();
    let source = db
        .store
        .insert_source(&new_source(&batch.id, 1), T0)
        .await
        .unwrap();
    db.store
        .insert_jobs(&[job(&batch.id, &source.id, JobKind::Plan, 0)], T0)
        .await
        .unwrap();
    // 规划任务不计入生图总数
    assert_eq!(db.store.get_batch(&batch.id).await.unwrap().total_jobs, 0);

    let plan_id = claim_all(&db.store, T0).await.remove(0);
    let generate: Vec<_> = (1..=4)
        .map(|v| job(&batch.id, &source.id, JobKind::Generate, v))
        .collect();
    let ids = db
        .store
        .succeed_plan_job(&plan_id, r#"{"subject":"橘猫"}"#, &generate, T0)
        .await
        .unwrap();

    assert_eq!(ids.len(), 4);
    assert_eq!(db.store.get_batch(&batch.id).await.unwrap().total_jobs, 4);
    let source = db.store.get_source(&source.id).await.unwrap();
    assert_eq!(source.analysis.as_deref(), Some(r#"{"subject":"橘猫"}"#));
    assert_eq!(
        dispatch_order(&db.store, T0).await,
        vec!["S1-1", "S1-2", "S1-3", "S1-4"]
    );
}

#[tokio::test]
async fn recovery_requeues_sync_jobs_and_resumes_async_ones() {
    let db = open().await;
    let (batch_id, _) = seed(&db.store, 1, 3).await;
    let ids = claim_all(&db.store, T0).await;
    // 第 1 个：同步请求途中崩溃；第 2 个：异步任务已提交；第 3 个：已经完成
    db.store
        .mark_polling(&ids[1], "remote-123", T0)
        .await
        .unwrap();
    db.store
        .succeed_job(&ids[2], &[image(3)], T0)
        .await
        .unwrap();

    let report = db.store.recover_after_restart(T0 + 1).await.unwrap();
    assert_eq!(report.resubmitted, 1);
    assert_eq!(report.resumed_polling, 0);

    assert_eq!(
        db.store.get_job(&ids[0]).await.unwrap().status,
        JobStatus::Queued
    );
    let polling = db.store.get_job(&ids[1]).await.unwrap();
    assert_eq!(polling.status, JobStatus::Polling);
    assert_eq!(polling.remote_task_id.as_deref(), Some("remote-123"));
    assert_eq!(
        db.store
            .get_batch(&batch_id)
            .await
            .unwrap()
            .resubmitted_jobs,
        1
    );
    assert_eq!(db.store.polling_jobs(10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn retrying_failed_jobs_reopens_the_batch() {
    let db = open().await;
    let (batch_id, _) = seed(&db.store, 1, 2).await;
    let ids = claim_all(&db.store, T0).await;
    db.store
        .succeed_job(&ids[0], &[image(1)], T0)
        .await
        .unwrap();
    db.store
        .fail_job(&ids[1], "timeout", "请求超时", T0)
        .await
        .unwrap();
    assert_eq!(
        db.store.get_batch(&batch_id).await.unwrap().status,
        BatchStatus::Completed
    );

    let retried = db
        .store
        .retry_failed_jobs(&batch_id, 10, T0 + 1)
        .await
        .unwrap();
    assert_eq!(retried, 1);
    let batch = db.store.get_batch(&batch_id).await.unwrap();
    assert_eq!(batch.status, BatchStatus::Running);
    assert_eq!(batch.failed_jobs, 0);
    assert_eq!(batch.finished_at, None);

    let job = db.store.get_job(&ids[1]).await.unwrap();
    assert_eq!(job.status, JobStatus::Queued);
    assert_eq!(job.attempt, 0);
    assert_eq!(job.priority, 10);
    assert_eq!(job.error_code, None);
}

#[tokio::test]
async fn profiles_can_be_created_listed_and_disabled() {
    let db = open().await;
    let mut profile = NewApiProfile {
        name: "主力".into(),
        purpose: "image".into(),
        access_type: "official".into(),
        provider_type: "openai_images".into(),
        base_url: "https://api.openai.com/v1".into(),
        model: "gpt-image-2.5-sunburst".into(),
        model_is_custom: false,
        max_concurrency: 8,
        rpm_limit: Some(60),
        weight: 3,
        timeout_secs: 600,
        transparent_method: "api".into(),
        capabilities: None,
        custom_spec: None,
    };
    let first = db.store.create_profile(&profile, T0).await.unwrap();
    profile.name = "备用".into();
    let second = db.store.create_profile(&profile, T0).await.unwrap();
    assert_eq!((first.sort_order, second.sort_order), (1, 2));
    assert!(first.enabled);

    profile.purpose = "text".into();
    db.store.create_profile(&profile, T0).await.unwrap();
    assert_eq!(
        db.store.list_profiles(Some("image")).await.unwrap().len(),
        2
    );
    assert_eq!(db.store.list_profiles(None).await.unwrap().len(), 3);

    db.store
        .set_profile_enabled(&first.id, false, Some("auth"), T0)
        .await
        .unwrap();
    let disabled = db.store.get_profile(&first.id).await.unwrap();
    assert!(!disabled.enabled);
    assert_eq!(disabled.disabled_reason.as_deref(), Some("auth"));

    db.store.delete_profile(&first.id).await.unwrap();
    assert!(matches!(
        db.store.get_profile(&first.id).await.unwrap_err(),
        StoreError::NotFound { .. }
    ));
}

#[tokio::test]
async fn settings_are_upserted() {
    let db = open().await;
    assert_eq!(
        db.store
            .get_setting("engine.global_image_concurrency")
            .await
            .unwrap(),
        None
    );
    db.store
        .set_setting("engine.global_image_concurrency", "10", T0)
        .await
        .unwrap();
    db.store
        .set_setting("engine.global_image_concurrency", "20", T0)
        .await
        .unwrap();
    assert_eq!(
        db.store
            .get_setting("engine.global_image_concurrency")
            .await
            .unwrap()
            .as_deref(),
        Some("20")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_completions_all_succeed() {
    // 引擎里几十个工作协程会同时完成任务；写事务之间不能互相报"database is locked"
    let db = open().await;
    let (batch_id, _) = seed(&db.store, 10, 5).await;
    let ids = claim_all(&db.store, T0).await;

    let mut handles = Vec::new();
    for (i, id) in ids.into_iter().enumerate() {
        let store = db.store.clone();
        handles.push(tokio::spawn(async move {
            if i % 5 == 0 {
                store.fail_job(&id, "timeout", "超时", T0).await
            } else {
                store.succeed_job(&id, &[image(1)], T0).await.map(|_| ())
            }
        }));
    }
    for handle in handles {
        handle.await.unwrap().unwrap();
    }
    let batch = db.store.get_batch(&batch_id).await.unwrap();
    assert_eq!((batch.succeeded_jobs, batch.failed_jobs), (40, 10));
    assert_eq!(batch.status, BatchStatus::Completed);
}
