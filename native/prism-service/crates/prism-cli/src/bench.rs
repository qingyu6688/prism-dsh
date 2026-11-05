//! `prism-cli bench`：压测调度引擎，对应 `docs/test.md` 第 4 章的 PT-01～PT-06。
//!
//! 只压引擎和存储，不写图片文件。需要先启动模拟服务（`cargo run -p mock-image-api -- ...`），
//! 任务带 `X-Prism-Job-Id` 请求头，结束后从模拟服务的 `/stats` 读重复生成数。

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use async_trait::async_trait;
use clap::{Args, ValueEnum};
use prism_core::{BatchStatus, JobKind, now_ms};
use prism_engine::{
    Engine, EngineConfig, EngineEvent, EventSink, HandlerError, JobHandler, JobOutput, JobRequest,
    KeyedFactory, Purpose,
};
use prism_provider::{ApiKey, ImageInput, ImageRequest};
use prism_store::{JobRecord, NewApiProfile, NewBatch, NewImage, NewJob, NewSourceImage, Store};
use serde_json::{Value, json};

/// 进度打印间隔。
const PROGRESS_EVERY: Duration = Duration::from_secs(5);
/// 每次插入多少个任务，避免单个事务太大。
const INSERT_CHUNK: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ProviderKind {
    /// 同步接口（OpenAI Images）
    Sync,
    /// 异步接口（提交 + 轮询）
    Async,
}

#[derive(Debug, Args)]
pub struct BenchArgs {
    /// 接口地址，默认是本机的模拟服务
    #[arg(long, default_value = "http://127.0.0.1:18080/v1")]
    base_url: String,
    /// 压测用的数据目录，里面放数据库；不存在会自动创建
    #[arg(long, default_value = ".data/bench")]
    data_dir: PathBuf,
    /// 接口类型
    #[arg(long, value_enum, default_value_t = ProviderKind::Sync)]
    provider: ProviderKind,
    /// 每个批次几张原图
    #[arg(long, default_value_t = 250)]
    sources: i64,
    /// 每张原图几个生图任务
    #[arg(long, default_value_t = 4)]
    variants: i64,
    /// 连续跑几个批次
    #[arg(long, default_value_t = 1)]
    batches: u32,
    /// 生图全局并发，也是配置的并发上限
    #[arg(long, default_value_t = 50)]
    concurrency: u32,
    /// 每个任务最多尝试几次
    #[arg(long, default_value_t = 3)]
    max_attempts: i64,
    /// 单个请求的超时（秒），要比模拟服务的挂起时间短，才能测到超时处理
    #[arg(long, default_value_t = 30)]
    request_timeout_secs: i64,
    /// 完成到这个比例时直接退出进程（模拟崩溃），比如 0.5
    #[arg(long)]
    crash_at: Option<f64>,
    /// 不建新批次，接着跑数据目录里没完成的任务（崩溃后重启用）
    #[arg(long)]
    resume: bool,
    /// 把结果另存为 JSON
    #[arg(long)]
    report: Option<PathBuf>,
}

/// 压测用的处理器：发一张很小的参考图，成功后只写数据库记录，不写图片文件。
struct BenchHandler {
    store: Store,
}

#[async_trait]
impl JobHandler for BenchHandler {
    fn purpose(&self, _job: &JobRecord) -> Purpose {
        Purpose::Image
    }

    async fn build_request(&self, job: &JobRecord) -> Result<JobRequest, HandlerError> {
        let mut request = ImageRequest::new(job.instruction.clone().unwrap_or_default());
        request.references = vec![ImageInput {
            bytes: vec![0; 16],
            mime: "image/png".into(),
            file_name: "ref.png".into(),
        }];
        Ok(JobRequest::Image(request))
    }

    async fn on_output(&self, job: &JobRecord, output: JobOutput) -> Result<(), HandlerError> {
        let JobOutput::Images(images) = output else {
            return Err(HandlerError::fatal("压测只处理生图任务"));
        };
        let image = NewImage {
            variant_index: job.variant_index,
            ref_no: None,
            file_path: format!("bench/{}.png", job.id),
            thumb_path: None,
            format: "png".into(),
            width: 64,
            height: 64,
            sha256: images
                .first()
                .map(|i| i.bytes.len())
                .unwrap_or(0)
                .to_string(),
            phash: None,
            dist_to_source: None,
            flags: None,
            actual_params: None,
        };
        self.store
            .succeed_job(&job.id, &[image], now_ms())
            .await
            .map(|_| ())
            .map_err(|e| HandlerError::fatal(e.to_string()))
    }
}

/// 统计引擎推出来的事件：各错误码出现了多少次（含重试过程中的）。
#[derive(Default)]
struct BenchSink {
    error_codes: Mutex<BTreeMap<String, u64>>,
    paused: Mutex<Vec<String>>,
}

impl EventSink for BenchSink {
    fn emit(&self, event: EngineEvent) {
        match event {
            EngineEvent::Progress { changes, .. } => {
                let mut codes = self.error_codes.lock().unwrap_or_else(|e| e.into_inner());
                for code in changes.iter().filter_map(|c| c.error_code.as_ref()) {
                    *codes.entry(code.clone()).or_default() += 1;
                }
            }
            EngineEvent::BatchPaused { batch_id, reason } => {
                println!("批次 {batch_id} 已暂停：{reason}");
                self.paused
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(reason);
            }
            EngineEvent::ProfileDisabled { message, .. } => println!("配置被停用：{message}"),
            EngineEvent::Recovered {
                resubmitted,
                resumed_polling,
            } => println!(
                "启动恢复：重新提交 {resubmitted} 个同步任务，继续轮询 {resumed_polling} 个异步任务"
            ),
            EngineEvent::BatchFinished { .. } => {}
        }
    }
}

pub async fn run(args: BenchArgs) -> anyhow::Result<()> {
    std::fs::create_dir_all(&args.data_dir)
        .with_context(|| format!("创建数据目录 {} 失败", args.data_dir.display()))?;
    let db_path = args.data_dir.join("prism.db");
    if !args.resume && db_path.exists() {
        bail!(
            "{} 里已经有压测数据。接着跑请加 --resume，重新开始请换一个 --data-dir 或删掉这个目录",
            args.data_dir.display()
        );
    }
    let store = Store::open(&db_path).await.context("打开数据库失败")?;

    let batch_ids = if args.resume {
        let batches = store.list_batches(1000, 0).await?;
        if batches.is_empty() {
            bail!("数据目录里没有批次，不能 --resume");
        }
        batches.into_iter().map(|b| b.id).collect()
    } else {
        ensure_profile(&store, &args).await?;
        let mut ids = Vec::new();
        for _ in 0..args.batches {
            ids.push(seed_batch(&store, &args).await?);
        }
        ids
    };

    let total: i64 = args.sources * args.variants * i64::from(args.batches);
    println!(
        "开始压测：{} 个批次，{} 个生图任务，并发 {}，接口 {:?}，{}",
        batch_ids.len(),
        if args.resume {
            "沿用已有".to_owned()
        } else {
            total.to_string()
        },
        args.concurrency,
        args.provider,
        args.base_url
    );

    let sink = Arc::new(BenchSink::default());
    let factory = KeyedFactory::new()
        .with_default_key(ApiKey::new("sk-bench"))
        .send_job_header(true);
    let config = EngineConfig {
        image_concurrency: args.concurrency,
        poll_interval: Duration::from_secs(1),
        ..EngineConfig::default()
    };
    // 接着跑（--resume）时，之前那次已经成功的不算进这次的吞吐
    let succeeded_before = succeeded_count(&store, &batch_ids).await?;
    let started = Instant::now();
    let engine = Engine::start(
        store.clone(),
        Arc::new(BenchHandler {
            store: store.clone(),
        }),
        Arc::new(factory),
        sink.clone(),
        config,
    )
    .await?;

    wait_until_settled(&store, &batch_ids, started, args.crash_at).await?;
    let elapsed = started.elapsed();
    engine.shutdown().await;

    let run = RunStats {
        elapsed,
        succeeded_before,
    };
    let report = build_report(&store, &batch_ids, &args, &sink, run).await?;
    print_report(&report);
    if let Some(path) = &args.report {
        std::fs::write(path, serde_json::to_string_pretty(&report)?)
            .with_context(|| format!("写报告 {} 失败", path.display()))?;
    }
    if report["verdict"]["passed"] != json!(true) {
        bail!("压测未通过，详见上面的判定");
    }
    Ok(())
}

async fn ensure_profile(store: &Store, args: &BenchArgs) -> anyhow::Result<()> {
    let provider_type = match args.provider {
        ProviderKind::Sync => "openai_images",
        ProviderKind::Async => "async_task",
    };
    let profile = NewApiProfile {
        name: "压测".into(),
        purpose: "image".into(),
        access_type: "relay".into(),
        provider_type: provider_type.into(),
        base_url: args.base_url.clone(),
        model: prism_core::defaults::DEFAULT_IMAGE_MODEL.into(),
        model_is_custom: false,
        max_concurrency: i64::from(args.concurrency),
        rpm_limit: None,
        weight: 1,
        timeout_secs: args.request_timeout_secs,
        transparent_method: "api".into(),
        capabilities: None,
        custom_spec: None,
    };
    store.create_profile(&profile, now_ms()).await?;
    Ok(())
}

async fn seed_batch(store: &Store, args: &BenchArgs) -> anyhow::Result<String> {
    let now = now_ms();
    let batch = store
        .create_batch(
            &NewBatch {
                module: "image".into(),
                product_id: None,
                conversation_id: None,
                mode: "similar".into(),
                variants_per_source: args.variants,
                params: "{}".into(),
            },
            now,
        )
        .await?;
    let mut jobs = Vec::new();
    for seq in 1..=args.sources {
        let source = store
            .insert_source(
                &NewSourceImage {
                    batch_id: batch.id.clone(),
                    seq,
                    ref_no: Some(format!("S{seq}")),
                    parent_image_id: None,
                    file_path: format!("bench/source-{seq}.png"),
                    sha256: format!("bench-{seq}"),
                    width: 64,
                    height: 64,
                    phash: None,
                },
                now,
            )
            .await?;
        for variant in 1..=args.variants {
            jobs.push(NewJob {
                batch_id: batch.id.clone(),
                source_id: Some(source.id.clone()),
                kind: JobKind::Generate,
                task_type: "bench.generate".into(),
                product_id: None,
                variant_index: variant,
                axis: None,
                instruction: Some(format!("压测指令 S{seq}-{variant}")),
                priority: 0,
                max_attempts: args.max_attempts,
                request_params: None,
            });
        }
    }
    for chunk in jobs.chunks(INSERT_CHUNK) {
        store.insert_jobs(chunk, now).await?;
    }
    Ok(batch.id)
}

/// 等所有批次离开"运行中"，每 5 秒打印一次进度；设置了 `crash_at` 时到点直接退出进程。
async fn wait_until_settled(
    store: &Store,
    batch_ids: &[String],
    started: Instant,
    crash_at: Option<f64>,
) -> anyhow::Result<()> {
    let mut last_print = Instant::now();
    loop {
        let mut total = 0;
        let mut succeeded = 0;
        let mut failed = 0;
        let mut running = false;
        for id in batch_ids {
            let batch = store.get_batch(id).await?;
            total += batch.total_jobs;
            succeeded += batch.succeeded_jobs;
            failed += batch.failed_jobs;
            running |= batch.status == BatchStatus::Running;
        }
        let done = succeeded + failed;
        let fraction = if total > 0 {
            done as f64 / total as f64
        } else {
            0.0
        };

        if let Some(threshold) = crash_at
            && fraction >= threshold
        {
            println!("已完成 {done}/{total}，按 --crash-at 模拟崩溃，直接退出进程");
            // 不做任何收尾：在途请求和未写库的状态都丢掉，和真实崩溃一样
            std::process::exit(3);
        }
        if !running {
            return Ok(());
        }
        if last_print.elapsed() >= PROGRESS_EVERY {
            let minutes = started.elapsed().as_secs_f64() / 60.0;
            println!(
                "进度 {done}/{total}（成功 {succeeded}，失败 {failed}），{:.0} 张/分钟",
                succeeded as f64 / minutes.max(1e-9)
            );
            last_print = Instant::now();
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// 这次运行的用时，以及开始前已经成功的任务数。
struct RunStats {
    elapsed: Duration,
    succeeded_before: i64,
}

async fn succeeded_count(store: &Store, batch_ids: &[String]) -> anyhow::Result<i64> {
    let mut succeeded = 0;
    for id in batch_ids {
        succeeded += store.get_batch(id).await?.succeeded_jobs;
    }
    Ok(succeeded)
}

async fn build_report(
    store: &Store,
    batch_ids: &[String],
    args: &BenchArgs,
    sink: &BenchSink,
    run: RunStats,
) -> anyhow::Result<Value> {
    let elapsed = run.elapsed;
    let mut total = 0;
    let mut succeeded = 0;
    let mut failed = 0;
    let mut resubmitted = 0;
    let mut unfinished = 0;
    let mut attempts: BTreeMap<i64, u64> = BTreeMap::new();
    let mut statuses: HashMap<&'static str, u64> = HashMap::new();
    for id in batch_ids {
        let batch = store.get_batch(id).await?;
        total += batch.total_jobs;
        succeeded += batch.succeeded_jobs;
        failed += batch.failed_jobs;
        resubmitted += batch.resubmitted_jobs;
        for job in store.list_jobs(id).await? {
            *attempts.entry(job.attempt).or_default() += 1;
            *statuses.entry(job.status.as_str()).or_default() += 1;
            if !job.status.is_terminal() {
                unfinished += 1;
            }
        }
    }

    let mock_stats = fetch_mock_stats(&args.base_url).await;
    let duplicates = mock_stats
        .as_ref()
        .and_then(|s| s["duplicate_jobs"].as_i64())
        .unwrap_or(-1);
    // 同步接口：崩溃时在途的请求结果拿不回来，重启后重新提交，重复数不能超过重新提交数；
    // 异步接口：任务 ID 已经落库，重启后接着轮询，不能有任何重复
    let allowed_duplicates = match args.provider {
        ProviderKind::Sync => resubmitted,
        ProviderKind::Async => 0,
    };
    let paused = sink
        .paused
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let passed = unfinished == 0 && duplicates >= 0 && duplicates <= allowed_duplicates;
    let minutes = elapsed.as_secs_f64() / 60.0;

    Ok(json!({
        "provider": format!("{:?}", args.provider),
        "concurrency": args.concurrency,
        "elapsedSeconds": (elapsed.as_secs_f64() * 10.0).round() / 10.0,
        "jobs": { "total": total, "succeeded": succeeded, "failed": failed, "unfinished": unfinished },
        "statuses": statuses,
        "succeededThisRun": succeeded - run.succeeded_before,
        "throughputPerMinute": ((succeeded - run.succeeded_before) as f64 / minutes.max(1e-9)).round(),
        "attempts": attempts,
        "errorCodesSeen": *sink.error_codes.lock().unwrap_or_else(|e| e.into_inner()),
        "resubmittedAfterCrash": resubmitted,
        "pausedReasons": paused,
        "mock": mock_stats.map(|s| json!({
            "requestsTotal": s["requests_total"],
            "responsesByStatus": s["responses_by_status"],
            "imagesGenerated": s["images_generated"],
            "duplicateJobs": s["duplicate_jobs"],
        })),
        "verdict": {
            "passed": passed,
            "unfinishedJobs": unfinished,
            "duplicateJobs": duplicates,
            "allowedDuplicates": allowed_duplicates,
        }
    }))
}

/// 读模拟服务的统计；接的不是模拟服务时返回 `None`。
async fn fetch_mock_stats(base_url: &str) -> Option<Value> {
    let root = base_url.trim_end_matches('/').trim_end_matches("/v1");
    let response = reqwest::get(format!("{root}/stats")).await.ok()?;
    response.json().await.ok()
}

fn print_report(report: &Value) {
    let jobs = &report["jobs"];
    let verdict = &report["verdict"];
    println!();
    println!("========== 压测结果 ==========");
    println!("用时         {} 秒", report["elapsedSeconds"]);
    println!(
        "任务         共 {}，成功 {}，失败 {}，未结束 {}",
        jobs["total"], jobs["succeeded"], jobs["failed"], jobs["unfinished"]
    );
    println!(
        "成功吞吐     {} 张/分钟（本次运行成功 {} 个）",
        report["throughputPerMinute"], report["succeededThisRun"]
    );
    println!("尝试次数分布 {}", report["attempts"]);
    println!("出现的错误码 {}", report["errorCodesSeen"]);
    println!("崩溃后重新提交 {}", report["resubmittedAfterCrash"]);
    if let Some(mock) = report["mock"].as_object() {
        println!(
            "模拟服务     请求 {}，按状态码 {}，生成 {} 张，重复生成的任务 {}",
            mock["requestsTotal"],
            mock["responsesByStatus"],
            mock["imagesGenerated"],
            mock["duplicateJobs"]
        );
    }
    println!(
        "判定         {}（未结束 {}，重复生成 {}，允许 {}）",
        if verdict["passed"] == json!(true) {
            "通过"
        } else {
            "未通过"
        },
        verdict["unfinishedJobs"],
        verdict["duplicateJobs"],
        verdict["allowedDuplicates"]
    );
}
