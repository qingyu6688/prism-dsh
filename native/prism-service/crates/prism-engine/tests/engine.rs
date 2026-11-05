//! 引擎集成测试：真实的 SQLite + 模拟服务，对应 `docs/test.md` 的 IT-01～IT-09。

// 测试里的辅助函数出错就该直接失败，用 unwrap 最直接
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::http::StatusCode;
use axum::routing::post;
use mock_image_api::{MockConfig, MockServer};
use prism_core::{BatchStatus, JobKind, JobStatus};
use prism_engine::{
    Engine, EngineConfig, EngineEvent, EventSink, HandlerError, JobHandler, JobOutput, JobRequest,
    KeyedFactory, Purpose, Rewrite,
};
use prism_provider::{ApiKey, ChatMessage, ChatRequest, ImageInput, ImageRequest};
use prism_store::{
    BatchRecord, JobRecord, NewApiProfile, NewBatch, NewImage, NewJob, NewSourceImage, Store,
};
use serde_json::Value;
use tempfile::TempDir;

// ---------- 测试用的处理器和事件收集 ----------

/// 规划任务发文本请求、按返回的指令数派生生图任务；生图任务写一条结果图。
struct TestHandler {
    store: Store,
    variants: usize,
    rewrite: bool,
    rewrites: AtomicUsize,
}

impl TestHandler {
    fn new(store: &Store) -> Self {
        Self {
            store: store.clone(),
            variants: 4,
            rewrite: false,
            rewrites: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl JobHandler for TestHandler {
    fn purpose(&self, job: &JobRecord) -> Purpose {
        match job.kind {
            JobKind::Plan => Purpose::Text,
            JobKind::Generate => Purpose::Image,
        }
    }

    async fn build_request(&self, job: &JobRecord) -> Result<JobRequest, HandlerError> {
        Ok(match job.kind {
            JobKind::Plan => {
                let mut request = ChatRequest::new(vec![ChatMessage::user(format!(
                    "规划 variant_count={}",
                    self.variants
                ))]);
                request.json_output = true;
                JobRequest::Text(request)
            }
            JobKind::Generate => {
                let mut request = ImageRequest::new(job.instruction.clone().unwrap_or_default());
                request.references = vec![ImageInput {
                    bytes: vec![1, 2, 3],
                    mime: "image/png".into(),
                    file_name: "ref.png".into(),
                }];
                JobRequest::Image(request)
            }
        })
    }

    async fn on_output(&self, job: &JobRecord, output: JobOutput) -> Result<(), HandlerError> {
        let now = prism_core::now_ms();
        match output {
            JobOutput::Images(images) => {
                let image = NewImage {
                    variant_index: job.variant_index,
                    ref_no: None,
                    file_path: format!("results/{}.png", job.id),
                    thumb_path: None,
                    format: "png".into(),
                    width: 64,
                    height: 64,
                    sha256: format!("{}", images[0].bytes.len()),
                    phash: None,
                    dist_to_source: None,
                    flags: None,
                    actual_params: None,
                };
                self.store
                    .succeed_job(&job.id, &[image], now)
                    .await
                    .map_err(|e| HandlerError::fatal(e.to_string()))?;
            }
            JobOutput::Text(response) => {
                let plan: Value = serde_json::from_str(response.content.as_deref().unwrap_or(""))
                    .map_err(|_| HandlerError::retryable("规划结果不是 JSON"))?;
                let variants = plan["variants"].as_array().cloned().unwrap_or_default();
                let jobs: Vec<NewJob> = variants
                    .iter()
                    .enumerate()
                    .map(|(i, v)| NewJob {
                        batch_id: job.batch_id.clone(),
                        source_id: job.source_id.clone(),
                        kind: JobKind::Generate,
                        task_type: "image.fission".into(),
                        product_id: None,
                        variant_index: i64::try_from(i).unwrap() + 1,
                        axis: v["axis"].as_str().map(str::to_owned),
                        instruction: v["instruction"].as_str().map(str::to_owned),
                        priority: 0,
                        max_attempts: 3,
                        request_params: None,
                    })
                    .collect();
                self.store
                    .succeed_plan_job(&job.id, &plan["analysis"].to_string(), &jobs, now)
                    .await
                    .map_err(|e| HandlerError::fatal(e.to_string()))?;
            }
        }
        Ok(())
    }

    async fn rewrite_after_rejection(
        &self,
        _job: &JobRecord,
    ) -> Result<Option<Rewrite>, HandlerError> {
        if !self.rewrite {
            return Ok(None);
        }
        self.rewrites.fetch_add(1, Ordering::SeqCst);
        Ok(Some(Rewrite {
            instruction: "改写后的指令".into(),
            request_params: None,
        }))
    }
}

#[derive(Default)]
struct CollectSink(Mutex<Vec<EngineEvent>>);

impl EventSink for CollectSink {
    fn emit(&self, event: EngineEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl CollectSink {
    fn events(&self) -> Vec<EngineEvent> {
        self.0.lock().unwrap().clone()
    }
}

// ---------- 搭环境 ----------

struct Env {
    store: Store,
    sink: Arc<CollectSink>,
    _dir: TempDir,
}

async fn env() -> Env {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("prism.db")).await.unwrap();
    Env {
        store,
        sink: Arc::new(CollectSink::default()),
        _dir: dir,
    }
}

/// 测试用的快节奏参数：退避、冷却、轮询都缩短到毫秒级。
fn fast_config() -> EngineConfig {
    EngineConfig {
        base_backoff: Duration::from_millis(20),
        max_backoff: Duration::from_millis(100),
        default_rate_limit_wait: Duration::from_millis(50),
        breaker_cooldown: Duration::from_millis(200),
        rate_limit_cut_cooldown: Duration::from_millis(100),
        poll_interval: Duration::from_millis(50),
        idle_wakeup: Duration::from_millis(50),
        progress_interval: Duration::from_millis(20),
        seed: Some(1),
        ..EngineConfig::default()
    }
}

async fn add_profile(
    store: &Store,
    name: &str,
    provider_type: &str,
    purpose: &str,
    base_url: &str,
    max: i64,
) -> String {
    let profile = NewApiProfile {
        name: name.into(),
        purpose: purpose.into(),
        access_type: "relay".into(),
        provider_type: provider_type.into(),
        base_url: base_url.into(),
        model: "gpt-image-2.5-sunburst".into(),
        model_is_custom: false,
        max_concurrency: max,
        rpm_limit: None,
        weight: 1,
        timeout_secs: 5,
        transparent_method: "api".into(),
        capabilities: None,
        custom_spec: None,
    };
    store
        .create_profile(&profile, prism_core::now_ms())
        .await
        .unwrap()
        .id
}

async fn start_engine(env: &Env, handler: Arc<TestHandler>, config: EngineConfig) -> Engine {
    let factory = KeyedFactory::new()
        .with_default_key(ApiKey::new("sk-test"))
        .send_job_header(true);
    Engine::start(
        env.store.clone(),
        handler,
        Arc::new(factory),
        env.sink.clone(),
        config,
    )
    .await
    .unwrap()
}

/// 建批次：`sources` 张原图，每张 `variants` 个生图任务（`variants` 为 0 时改为每张一个规划任务）。
async fn seed(store: &Store, sources: i64, variants: i64, max_attempts: i64) -> String {
    let now = prism_core::now_ms();
    let batch = store
        .create_batch(
            &NewBatch {
                module: "image".into(),
                product_id: None,
                conversation_id: None,
                mode: "similar".into(),
                variants_per_source: variants.max(4),
                params: "{}".into(),
            },
            now,
        )
        .await
        .unwrap();
    let mut jobs = Vec::new();
    for seq in 1..=sources {
        let source = store
            .insert_source(
                &NewSourceImage {
                    batch_id: batch.id.clone(),
                    seq,
                    ref_no: None,
                    parent_image_id: None,
                    file_path: format!("s{seq}.png"),
                    sha256: format!("h{seq}"),
                    width: 64,
                    height: 64,
                    phash: None,
                },
                now,
            )
            .await
            .unwrap();
        let (kind, count) = if variants == 0 {
            (JobKind::Plan, 1)
        } else {
            (JobKind::Generate, variants)
        };
        for v in 1..=count {
            jobs.push(NewJob {
                batch_id: batch.id.clone(),
                source_id: Some(source.id.clone()),
                kind,
                task_type: "image.fission".into(),
                product_id: None,
                variant_index: if kind == JobKind::Plan { 0 } else { v },
                axis: None,
                instruction: Some(format!("指令 {seq}-{v}")),
                priority: 0,
                max_attempts,
                request_params: None,
            });
        }
    }
    store.insert_jobs(&jobs, now).await.unwrap();
    batch.id
}

/// 等批次离开"运行中"（完成、失败、取消或暂停），最多等 `timeout`。
async fn wait_settled(store: &Store, batch_id: &str, timeout: Duration) -> BatchRecord {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let batch = store.get_batch(batch_id).await.unwrap();
        if batch.status != BatchStatus::Running {
            return batch;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "批次 {timeout:?} 内没有结束：{batch:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn jobs(store: &Store, batch_id: &str) -> Vec<JobRecord> {
    store.list_jobs(batch_id).await.unwrap()
}

// ---------- 测试 ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plans_then_generates_every_variant() {
    // IT-01：5 张原图 × 4，规划完成后派生生图任务，全部成功
    let env = env().await;
    let server = MockServer::start(MockConfig::default()).await.unwrap();
    add_profile(
        &env.store,
        "文本",
        "openai_chat",
        "text",
        &server.base_url(),
        4,
    )
    .await;
    add_profile(
        &env.store,
        "生图",
        "openai_images",
        "image",
        &server.base_url(),
        8,
    )
    .await;
    let batch_id = seed(&env.store, 5, 0, 3).await;
    let engine = start_engine(&env, Arc::new(TestHandler::new(&env.store)), fast_config()).await;

    let batch = wait_settled(&env.store, &batch_id, Duration::from_secs(10)).await;
    engine.shutdown().await;

    assert_eq!(batch.status, BatchStatus::Completed);
    assert_eq!(batch.total_jobs, 20);
    assert_eq!(batch.succeeded_jobs, 20);
    let stats = server.stats();
    assert_eq!(stats.duplicate_jobs, 0);
    assert_eq!(stats.images_generated, 20);
    let events = env.sink.events();
    assert!(events.iter().any(|e| matches!(
        e,
        EngineEvent::BatchFinished {
            status: BatchStatus::Completed,
            ..
        }
    )));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, EngineEvent::Progress { .. }))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retries_server_errors_until_success_or_limit() {
    // IT-02：30% 的 500，每个任务最终成功或在 3 次后失败
    let env = env().await;
    let server = MockServer::start(MockConfig {
        error_500: 0.3,
        ..MockConfig::default()
    })
    .await
    .unwrap();
    add_profile(
        &env.store,
        "生图",
        "openai_images",
        "image",
        &server.base_url(),
        8,
    )
    .await;
    let batch_id = seed(&env.store, 10, 3, 3).await;
    let engine = start_engine(&env, Arc::new(TestHandler::new(&env.store)), fast_config()).await;

    let batch = wait_settled(&env.store, &batch_id, Duration::from_secs(15)).await;
    engine.shutdown().await;

    assert_eq!(batch.status, BatchStatus::Completed);
    assert_eq!(batch.succeeded_jobs + batch.failed_jobs, 30);
    for job in jobs(&env.store, &batch_id).await {
        assert!(job.status.is_terminal());
        if job.status == JobStatus::Failed {
            assert_eq!(job.attempt, 3);
            assert_eq!(job.error_code.as_deref(), Some("upstream_error"));
            assert!(job.error_message.is_some());
        }
    }
    let stats = server.stats();
    assert_eq!(
        stats.images_generated,
        u64::try_from(batch.succeeded_jobs).unwrap()
    );
    assert_eq!(stats.duplicate_jobs, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rate_limits_never_fail_jobs() {
    // IT-03：40% 的 429，任务全部成功（429 不计入尝试次数），配置的并发上限会降下来
    let env = env().await;
    let server = MockServer::start(MockConfig {
        error_429: 0.4,
        ..MockConfig::default()
    })
    .await
    .unwrap();
    add_profile(
        &env.store,
        "生图",
        "openai_images",
        "image",
        &server.base_url(),
        8,
    )
    .await;
    let batch_id = seed(&env.store, 5, 4, 3).await;
    let engine = start_engine(&env, Arc::new(TestHandler::new(&env.store)), fast_config()).await;

    // 上限降下来以后，每 10 次成功又会加 1，结束时可能已经回到 8；所以看过程中的最低值
    let mut lowest_limit = u32::MAX;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let batch = loop {
        lowest_limit = lowest_limit.min(engine.profile_snapshots()[0].current_limit);
        let batch = env.store.get_batch(&batch_id).await.unwrap();
        if batch.status != BatchStatus::Running {
            break batch;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "批次没有结束：{batch:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    engine.shutdown().await;

    assert_eq!(batch.status, BatchStatus::Completed);
    assert_eq!(batch.succeeded_jobs, 20);
    assert!(server.stats().responses_by_status[&429] > 0);
    assert!(lowest_limit < 8, "最低并发上限 {lowest_limit}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn breaker_probes_hit_by_rate_limits_do_not_stall_the_engine() {
    // 回归 PT-03：熔断半开时的试探请求遇到 429，以前试探名额不释放，引擎再也不派任务。
    // 熔断阈值设成 1，让熔断频繁打开，试探请求有很大概率碰上 429
    let env = env().await;
    let server = MockServer::start(MockConfig {
        error_429: 0.3,
        error_500: 0.2,
        ..MockConfig::default()
    })
    .await
    .unwrap();
    add_profile(
        &env.store,
        "生图",
        "openai_images",
        "image",
        &server.base_url(),
        4,
    )
    .await;
    let batch_id = seed(&env.store, 5, 4, 10).await;
    let config = EngineConfig {
        breaker_threshold: 1,
        breaker_cooldown: Duration::from_millis(30),
        failure_pause_ratio: 1.1,
        ..fast_config()
    };
    let engine = start_engine(&env, Arc::new(TestHandler::new(&env.store)), config).await;

    let batch = wait_settled(&env.store, &batch_id, Duration::from_secs(30)).await;
    engine.shutdown().await;

    assert_ne!(batch.status, BatchStatus::Paused, "{batch:?}");
    assert_eq!(batch.succeeded_jobs + batch.failed_jobs, 20);
    let stats = server.stats();
    assert!(stats.responses_by_status[&429] > 0);
    assert!(stats.responses_by_status[&500] > 0);
}

/// 一个永远返回 401 的服务。
async fn unauthorized_server() -> String {
    let app = Router::new()
        .route(
            "/v1/images/edits",
            post(|| async {
                (
                    StatusCode::UNAUTHORIZED,
                    r#"{"error":{"message":"bad key"}}"#,
                )
            }),
        )
        .route(
            "/v1/images/generations",
            post(|| async { (StatusCode::UNAUTHORIZED, "{}") }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/v1")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bad_key_profile_is_disabled_and_work_moves_on() {
    // IT-04：一个配置 Key 错误，被停用，任务全部转到另一个
    let env = env().await;
    let server = MockServer::start(MockConfig::default()).await.unwrap();
    let bad = add_profile(
        &env.store,
        "坏 Key",
        "openai_images",
        "image",
        &unauthorized_server().await,
        8,
    )
    .await;
    add_profile(
        &env.store,
        "好 Key",
        "openai_images",
        "image",
        &server.base_url(),
        8,
    )
    .await;
    let batch_id = seed(&env.store, 5, 2, 3).await;
    let engine = start_engine(&env, Arc::new(TestHandler::new(&env.store)), fast_config()).await;

    let batch = wait_settled(&env.store, &batch_id, Duration::from_secs(10)).await;
    engine.shutdown().await;

    assert_eq!(batch.status, BatchStatus::Completed);
    assert_eq!(batch.succeeded_jobs, 10);
    let profile = env.store.get_profile(&bad).await.unwrap();
    assert!(!profile.enabled);
    assert_eq!(profile.disabled_reason.as_deref(), Some("auth"));
    assert!(env.sink.events().iter().any(
        |e| matches!(e, EngineEvent::ProfileDisabled { profile_id, .. } if *profile_id == bad)
    ));
    // 被 401 拒绝的那次不算尝试次数
    assert!(
        jobs(&env.store, &batch_id)
            .await
            .iter()
            .all(|j| j.attempt == 1)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn batch_pauses_when_no_profile_is_usable() {
    // IT-05：唯一的配置 Key 错误，停用后批次暂停
    let env = env().await;
    add_profile(
        &env.store,
        "坏 Key",
        "openai_images",
        "image",
        &unauthorized_server().await,
        4,
    )
    .await;
    let batch_id = seed(&env.store, 2, 2, 3).await;
    let engine = start_engine(&env, Arc::new(TestHandler::new(&env.store)), fast_config()).await;

    let batch = wait_settled(&env.store, &batch_id, Duration::from_secs(10)).await;
    engine.shutdown().await;

    assert_eq!(batch.status, BatchStatus::Paused);
    assert_eq!(batch.pause_reason.as_deref(), Some("no_available_profile"));
    assert!(env.sink.events().iter().any(
        |e| matches!(e, EngineEvent::BatchPaused { reason, .. } if reason == "no_available_profile")
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn async_jobs_complete_through_polling() {
    // IT-06：异步接口提交后释放名额，轮询拿到结果
    let env = env().await;
    let server = MockServer::start(MockConfig {
        latency_ms: (100, 200),
        ..MockConfig::default()
    })
    .await
    .unwrap();
    add_profile(
        &env.store,
        "异步",
        "async_task",
        "image",
        &server.base_url(),
        2,
    )
    .await;
    let batch_id = seed(&env.store, 4, 3, 3).await;
    let engine = start_engine(&env, Arc::new(TestHandler::new(&env.store)), fast_config()).await;

    let batch = wait_settled(&env.store, &batch_id, Duration::from_secs(10)).await;
    engine.shutdown().await;

    assert_eq!(batch.status, BatchStatus::Completed);
    assert_eq!(batch.succeeded_jobs, 12);
    let stats = server.stats();
    assert_eq!(stats.requests_by_path["/v1/async/images"], 12);
    assert_eq!(stats.duplicate_jobs, 0);
    assert!(
        jobs(&env.store, &batch_id)
            .await
            .iter()
            .all(|j| j.remote_task_id.is_some())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn content_rejection_rewrites_once_then_fails() {
    // IT-07：审核拒绝后改写一次，再被拒就失败
    let env = env().await;
    let server = MockServer::start(MockConfig {
        content_rejected: 1.0,
        ..MockConfig::default()
    })
    .await
    .unwrap();
    add_profile(
        &env.store,
        "生图",
        "openai_images",
        "image",
        &server.base_url(),
        4,
    )
    .await;
    let batch_id = seed(&env.store, 1, 3, 3).await;
    let handler = Arc::new(TestHandler {
        rewrite: true,
        ..TestHandler::new(&env.store)
    });
    let engine = start_engine(&env, handler.clone(), fast_config()).await;

    let batch = wait_settled(&env.store, &batch_id, Duration::from_secs(10)).await;
    engine.shutdown().await;

    assert_eq!(batch.status, BatchStatus::Failed);
    assert_eq!(handler.rewrites.load(Ordering::SeqCst), 3);
    for job in jobs(&env.store, &batch_id).await {
        assert!(job.rewritten);
        assert_eq!(job.instruction.as_deref(), Some("改写后的指令"));
        assert_eq!(job.error_code.as_deref(), Some("content_rejected"));
        assert_eq!(job.attempt, 2);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_stops_pending_work() {
    // IT-08：取消批次后排队中的任务不再发出
    let env = env().await;
    let server = MockServer::start(MockConfig {
        latency_ms: (300, 300),
        ..MockConfig::default()
    })
    .await
    .unwrap();
    add_profile(
        &env.store,
        "生图",
        "openai_images",
        "image",
        &server.base_url(),
        2,
    )
    .await;
    let batch_id = seed(&env.store, 10, 2, 3).await;
    let config = EngineConfig {
        image_concurrency: 2,
        ..fast_config()
    };
    let engine = start_engine(&env, Arc::new(TestHandler::new(&env.store)), config).await;

    tokio::time::sleep(Duration::from_millis(100)).await;
    let cancelled = env
        .store
        .cancel_batch(&batch_id, prism_core::now_ms())
        .await
        .unwrap();
    engine.shutdown().await;

    assert_eq!(cancelled, 18, "并发 2，取消时应该只有 2 个在途");
    let batch = env.store.get_batch(&batch_id).await.unwrap();
    assert_eq!(batch.status, BatchStatus::Cancelled);
    assert_eq!(batch.succeeded_jobs, 2);
    assert_eq!(server.stats().requests_total, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn high_failure_rate_pauses_the_batch() {
    // IT-09：失败率超过 50% 且已结束 20 个以上时自动暂停
    let env = env().await;
    let server = MockServer::start(MockConfig {
        error_500: 1.0,
        ..MockConfig::default()
    })
    .await
    .unwrap();
    add_profile(
        &env.store,
        "生图",
        "openai_images",
        "image",
        &server.base_url(),
        4,
    )
    .await;
    let batch_id = seed(&env.store, 20, 3, 1).await;
    let config = EngineConfig {
        breaker_threshold: 1000,
        ..fast_config()
    };
    let engine = start_engine(&env, Arc::new(TestHandler::new(&env.store)), config).await;

    let batch = wait_settled(&env.store, &batch_id, Duration::from_secs(10)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    engine.shutdown().await;

    assert_eq!(batch.status, BatchStatus::Paused);
    assert_eq!(batch.pause_reason.as_deref(), Some("high_failure_rate"));
    let batch = env.store.get_batch(&batch_id).await.unwrap();
    assert!(
        batch.failed_jobs >= 20 && batch.failed_jobs < 60,
        "{batch:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restart_recovers_interrupted_jobs() {
    // 模拟上次运行时 2 个同步任务发到一半进程退出
    let env = env().await;
    let server = MockServer::start(MockConfig::default()).await.unwrap();
    add_profile(
        &env.store,
        "生图",
        "openai_images",
        "image",
        &server.base_url(),
        4,
    )
    .await;
    let batch_id = seed(&env.store, 2, 2, 3).await;
    let now = prism_core::now_ms();
    for job in env.store.dispatchable_jobs(now, None, 2).await.unwrap() {
        assert!(env.store.claim_job(&job.id, None, now).await.unwrap());
    }

    let engine = start_engine(&env, Arc::new(TestHandler::new(&env.store)), fast_config()).await;
    let batch = wait_settled(&env.store, &batch_id, Duration::from_secs(10)).await;
    engine.shutdown().await;

    assert_eq!(batch.status, BatchStatus::Completed);
    assert_eq!(batch.succeeded_jobs, 4);
    assert_eq!(batch.resubmitted_jobs, 2);
    assert!(env.sink.events().contains(&EngineEvent::Recovered {
        resubmitted: 2,
        resumed_polling: 0
    }));
}
