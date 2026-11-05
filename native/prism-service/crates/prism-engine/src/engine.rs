//! 调度器、工作协程、轮询器、进度推送。设计见 `docs/detail-design.md` 第 2 章。
//!
//! 并发模型：
//! - 调度器（1 个协程）：取可派发任务 → 选配置、占名额 → 抢占任务 → 为每个任务起一个工作协程
//! - 工作协程：构造请求 → 调供应商 → 交处理器入库 / 按错误码处理 → 释放名额 → 唤醒调度器
//! - 轮询器（1 个协程）：定时查轮询中的任务
//! - 进度推送（1 个协程）：每隔一段时间把攒下的状态变化推出去
//!
//! 运行时状态放在一把普通互斥锁里，锁内不做任何 await。

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use prism_core::{BatchStatus, JobStatus, RetryPolicy, UpstreamErrorCode, now_ms};
use prism_provider::{PollOutcome, ProviderError, SubmitOutcome};
use prism_store::{JobRecord, Store, StoreError};
use rand::SeedableRng;
use rand::rngs::StdRng;
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinHandle;

use crate::limits::{backoff_cap, jittered};
use crate::profiles::{
    ProfileLimits, ProfileRuntime, ProfileSnapshot, ProviderHandle, choose_profile,
};
use crate::types::ProgressBuffer;
use crate::{
    EngineConfig, EngineEvent, EventSink, HandlerError, JobChange, JobHandler, JobOutput,
    JobRequest, ProviderFactory, Purpose, lock,
};

/// 引擎启动失败。
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// 停止时等在途任务结束的最长时间，超过就不等了（下次启动会恢复）。
const SHUTDOWN_GRACE: Duration = Duration::from_secs(30);
/// 一轮轮询最多查多少个任务。
const POLL_BATCH: i64 = 100;

/// 任务引擎。`start` 之后在后台运行，`shutdown` 停止。
pub struct Engine {
    inner: Arc<Inner>,
    tasks: Vec<JoinHandle<()>>,
}

struct Inner {
    store: Store,
    handler: Arc<dyn JobHandler>,
    factory: Arc<dyn ProviderFactory>,
    sink: Arc<dyn EventSink>,
    config: EngineConfig,
    wake: Notify,
    idle: Notify,
    stopping: AtomicBool,
    workers: AtomicUsize,
    progress: ProgressBuffer,
    state: Mutex<State>,
}

struct State {
    image: Vec<ProfileRuntime>,
    text: Vec<ProfileRuntime>,
    image_inflight: u32,
    text_inflight: u32,
    rng: StdRng,
    /// 已经推送过"批次结束"的批次，避免重复推送
    finished: HashSet<String>,
    /// 正在轮询的任务，避免同一个任务被两轮轮询同时查
    polling: HashSet<String>,
}

impl State {
    fn profiles(&mut self, purpose: Purpose) -> &mut Vec<ProfileRuntime> {
        match purpose {
            Purpose::Image => &mut self.image,
            Purpose::Text => &mut self.text,
        }
    }

    fn inflight(&mut self, purpose: Purpose) -> &mut u32 {
        match purpose {
            Purpose::Image => &mut self.image_inflight,
            Purpose::Text => &mut self.text_inflight,
        }
    }

    fn profile_mut(&mut self, purpose: Purpose, id: &str) -> Option<&mut ProfileRuntime> {
        self.profiles(purpose).iter_mut().find(|p| p.id == id)
    }
}

/// 调度器给一个任务预留的名额。
struct Reservation {
    purpose: Purpose,
    profile_id: String,
    provider: ProviderHandle,
    /// 这次派发是不是熔断半开时的试探请求，释放时要让出试探名额
    probe: bool,
}

/// 一次执行的结果。
enum Executed {
    /// 拿到结果并已入库
    Completed,
    /// 异步任务已提交，转入轮询
    Submitted,
}

enum Failure {
    Provider(ProviderError),
    Handler(HandlerError),
    Store(StoreError),
}

impl Engine {
    /// 恢复上次未完成的任务，加载配置，启动后台协程。
    pub async fn start(
        store: Store,
        handler: Arc<dyn JobHandler>,
        factory: Arc<dyn ProviderFactory>,
        sink: Arc<dyn EventSink>,
        config: EngineConfig,
    ) -> Result<Self, EngineError> {
        let report = store.recover_after_restart(now_ms()).await?;
        if report.resubmitted > 0 || report.resumed_polling > 0 {
            tracing::info!(
                resubmitted = report.resubmitted,
                resumed_polling = report.resumed_polling,
                "恢复了上次未完成的任务"
            );
            sink.emit(EngineEvent::Recovered {
                resubmitted: report.resubmitted,
                resumed_polling: report.resumed_polling,
            });
        }

        let rng = config
            .seed
            .map_or_else(StdRng::from_os_rng, StdRng::seed_from_u64);
        let inner = Arc::new(Inner {
            store,
            handler,
            factory,
            sink,
            config,
            wake: Notify::new(),
            idle: Notify::new(),
            stopping: AtomicBool::new(false),
            workers: AtomicUsize::new(0),
            progress: ProgressBuffer::default(),
            state: Mutex::new(State {
                image: Vec::new(),
                text: Vec::new(),
                image_inflight: 0,
                text_inflight: 0,
                rng,
                finished: HashSet::new(),
                polling: HashSet::new(),
            }),
        });
        inner.load_profiles().await?;

        let tasks = vec![
            tokio::spawn(Arc::clone(&inner).dispatch_loop()),
            tokio::spawn(Arc::clone(&inner).poll_loop()),
            tokio::spawn(Arc::clone(&inner).progress_loop()),
        ];
        Ok(Self { inner, tasks })
    }

    /// 有新任务入队后调用，让调度器马上处理。
    pub fn notify(&self) {
        self.inner.wake.notify_one();
    }

    /// 配置增删改后调用，重新加载启用的配置。正在跑的任务不受影响。
    pub async fn reload_profiles(&self) -> Result<(), EngineError> {
        self.inner.load_profiles().await?;
        self.inner.wake.notify_one();
        Ok(())
    }

    /// 各配置的运行时状态，设置页实时显示。
    pub fn profile_snapshots(&self) -> Vec<ProfileSnapshot> {
        let now = Instant::now();
        let state = lock(&self.inner.state);
        state
            .text
            .iter()
            .chain(state.image.iter())
            .map(|p| p.snapshot(now))
            .collect()
    }

    /// 停止派发新任务，等在途任务结束（最多 30 秒），推送剩余进度。
    pub async fn shutdown(self) {
        self.inner.stopping.store(true, Ordering::SeqCst);
        self.inner.wake.notify_one();
        let deadline = tokio::time::Instant::now() + SHUTDOWN_GRACE;
        while self.inner.workers.load(Ordering::SeqCst) > 0 {
            let notified = self.inner.idle.notified();
            if self.inner.workers.load(Ordering::SeqCst) == 0 {
                break;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                tracing::warn!("停止时还有任务没结束，下次启动时会恢复");
                break;
            }
        }
        for task in &self.tasks {
            task.abort();
        }
        self.inner.flush_progress();
    }
}

impl Inner {
    // ---------- 配置 ----------

    async fn load_profiles(&self) -> Result<(), StoreError> {
        let records = self.store.list_profiles(None).await?;
        let now = Instant::now();
        let mut image = Vec::new();
        let mut text = Vec::new();
        for record in records.iter().filter(|r| r.enabled) {
            let provider = match self.factory.build(record) {
                Ok(provider) => provider,
                Err(error) => {
                    tracing::warn!(profile = %record.name, "配置无法使用：{error}");
                    continue;
                }
            };
            let limits = ProfileLimits {
                weight: to_u32(record.weight),
                max_concurrency: to_u32(record.max_concurrency),
                rpm_limit: record.rpm_limit.map(to_u32),
            };
            let runtime = ProfileRuntime::new(
                record.id.clone(),
                record.name.clone(),
                limits,
                provider.clone(),
                &self.config,
                now,
            );
            match provider {
                ProviderHandle::Image(_) => image.push(runtime),
                ProviderHandle::Text(_) => text.push(runtime),
            }
        }
        let mut state = lock(&self.state);
        // 重新加载时保留在途数，否则正在跑的任务释放名额时会把计数减成负的
        for runtime in image.iter_mut().chain(text.iter_mut()) {
            if let Some(old) = state
                .image
                .iter()
                .chain(state.text.iter())
                .find(|old| old.id == runtime.id)
            {
                runtime.inflight = old.inflight;
            }
        }
        state.image = image;
        state.text = text;
        Ok(())
    }

    // ---------- 调度 ----------

    async fn dispatch_loop(self: Arc<Self>) {
        while !self.stopping.load(Ordering::SeqCst) {
            let wait = self.dispatch_round().await;
            tokio::select! {
                () = self.wake.notified() => {}
                () = tokio::time::sleep(wait) => {}
            }
        }
    }

    /// 派发一轮，返回下次最晚什么时候醒来。
    async fn dispatch_round(self: &Arc<Self>) -> Duration {
        let jobs = match self
            .store
            .dispatchable_jobs(now_ms(), None, self.config.dispatch_batch)
            .await
        {
            Ok(jobs) => jobs,
            Err(error) => {
                tracing::error!("读取待派发任务失败：{error}");
                return self.config.idle_wakeup;
            }
        };

        let mut paused_batches = HashSet::new();
        for job in jobs {
            if self.stopping.load(Ordering::SeqCst) {
                break;
            }
            if paused_batches.contains(&job.batch_id) {
                continue;
            }
            let purpose = self.handler.purpose(&job);
            let reservation = match self.reserve(purpose, job.profile_id.as_deref()) {
                Reserve::Reserved(reservation) => reservation,
                Reserve::Busy => continue,
                Reserve::NoProfile => {
                    self.pause_batch(&job.batch_id, "no_available_profile")
                        .await;
                    paused_batches.insert(job.batch_id.clone());
                    continue;
                }
            };
            match self
                .store
                .claim_job(&job.id, Some(&reservation.profile_id), now_ms())
                .await
            {
                Ok(true) => self.spawn_worker(job, reservation),
                Ok(false) => self.release(&reservation),
                Err(error) => {
                    tracing::error!(job = %job.id, "抢占任务失败：{error}");
                    self.release(&reservation);
                }
            }
        }
        self.next_wakeup().await
    }

    fn reserve(&self, purpose: Purpose, avoid: Option<&str>) -> Reserve {
        let limit = match purpose {
            Purpose::Image => self.config.image_concurrency,
            Purpose::Text => self.config.text_concurrency,
        };
        let now = Instant::now();
        let mut state = lock(&self.state);
        if *state.inflight(purpose) >= limit {
            return Reserve::Busy;
        }
        let State {
            image, text, rng, ..
        } = &mut *state;
        let profiles = match purpose {
            Purpose::Image => image,
            Purpose::Text => text,
        };
        if profiles.iter().all(|p| p.disabled_reason.is_some()) {
            return Reserve::NoProfile;
        }
        let Some(index) = choose_profile(profiles, avoid, now, rng) else {
            return Reserve::Busy;
        };
        let profile = &mut profiles[index];
        let probe = profile.acquire(now);
        let reservation = Reservation {
            purpose,
            profile_id: profile.id.clone(),
            provider: profile.provider.clone(),
            probe,
        };
        *state.inflight(purpose) += 1;
        Reserve::Reserved(reservation)
    }

    fn release(&self, reservation: &Reservation) {
        let mut state = lock(&self.state);
        let inflight = state.inflight(reservation.purpose);
        *inflight = inflight.saturating_sub(1);
        if let Some(profile) = state.profile_mut(reservation.purpose, &reservation.profile_id) {
            profile.release(reservation.probe);
        }
    }

    /// 下次醒来的时间：最近的重试到期、最早恢复可用的配置、兜底间隔，取最小。
    async fn next_wakeup(&self) -> Duration {
        let mut wait = self.config.idle_wakeup;
        if let Ok(Some(next)) = self.store.next_retry_at().await {
            let delay = u64::try_from(next - now_ms()).unwrap_or(0);
            wait = wait.min(Duration::from_millis(delay));
        }
        let now = Instant::now();
        let mut state = lock(&self.state);
        let State { image, text, .. } = &mut *state;
        for profile in image.iter_mut().chain(text.iter_mut()) {
            if let Some(available) = profile.available_in(now) {
                wait = wait.min(available);
            }
        }
        wait.max(Duration::from_millis(5))
    }

    // ---------- 执行 ----------

    fn spawn_worker(self: &Arc<Self>, mut job: JobRecord, reservation: Reservation) {
        job.attempt += 1;
        job.status = JobStatus::Running;
        job.profile_id = Some(reservation.profile_id.clone());
        self.workers.fetch_add(1, Ordering::SeqCst);
        let inner = Arc::clone(self);
        tokio::spawn(async move {
            inner.push_progress(&job, JobStatus::Running, None);
            match inner.execute(&job, &reservation.provider).await {
                Ok(Executed::Completed) => {
                    inner.on_profile_success(&reservation);
                    inner.push_progress(&job, JobStatus::Succeeded, None);
                    inner.check_finished(&job.batch_id).await;
                }
                Ok(Executed::Submitted) => {
                    inner.on_profile_success(&reservation);
                    inner.push_progress(&job, JobStatus::Polling, None);
                }
                Err(failure) => {
                    inner
                        .handle_failure(&job, failure, Some(&reservation))
                        .await;
                }
            }
            inner.release(&reservation);
            inner.wake.notify_one();
            if inner.workers.fetch_sub(1, Ordering::SeqCst) == 1 {
                inner.idle.notify_waiters();
            }
        });
    }

    async fn execute(
        &self,
        job: &JobRecord,
        provider: &ProviderHandle,
    ) -> Result<Executed, Failure> {
        let request = self
            .handler
            .build_request(job)
            .await
            .map_err(Failure::Handler)?;
        match (request, provider) {
            (JobRequest::Image(mut request), ProviderHandle::Image(provider)) => {
                request.job_id = Some(job.id.clone());
                match provider.submit(&request).await.map_err(Failure::Provider)? {
                    SubmitOutcome::Done(images) => {
                        self.handler
                            .on_output(job, JobOutput::Images(images))
                            .await
                            .map_err(Failure::Handler)?;
                        Ok(Executed::Completed)
                    }
                    SubmitOutcome::Pending { remote_task_id } => {
                        self.store
                            .mark_polling(&job.id, &remote_task_id, now_ms())
                            .await
                            .map_err(Failure::Store)?;
                        Ok(Executed::Submitted)
                    }
                }
            }
            (JobRequest::Text(mut request), ProviderHandle::Text(provider)) => {
                request.job_id = Some(job.id.clone());
                let response = provider.chat(&request).await.map_err(Failure::Provider)?;
                self.handler
                    .on_output(job, JobOutput::Text(response))
                    .await
                    .map_err(Failure::Handler)?;
                Ok(Executed::Completed)
            }
            _ => Err(Failure::Handler(HandlerError::fatal(
                "任务类型和配置用途不匹配",
            ))),
        }
    }

    fn on_profile_success(&self, reservation: &Reservation) {
        let mut state = lock(&self.state);
        if let Some(profile) = state.profile_mut(reservation.purpose, &reservation.profile_id) {
            profile.on_success();
        }
    }

    /// 按错误类型处理失败，对应详细设计 2.7。
    async fn handle_failure(
        &self,
        job: &JobRecord,
        failure: Failure,
        reservation: Option<&Reservation>,
    ) {
        let purpose = self.handler.purpose(job);
        let profile_id = reservation
            .map(|r| r.profile_id.clone())
            .or_else(|| job.profile_id.clone());
        let result = match failure {
            Failure::Store(error) => {
                // 存储出错时任务状态不确定，留给下次启动的恢复逻辑
                tracing::error!(job = %job.id, "写入任务状态失败：{error}");
                Ok(())
            }
            Failure::Handler(error) if error.retryable => {
                self.retry_or_fail(job, UpstreamErrorCode::BadResponse, &error.message, None, 2)
                    .await
            }
            Failure::Handler(error) => {
                self.fail(job, UpstreamErrorCode::BadRequest, &error.message)
                    .await
            }
            Failure::Provider(error) => {
                self.handle_provider_error(job, purpose, profile_id.as_deref(), &error)
                    .await
            }
        };
        if let Err(error) = result {
            tracing::error!(job = %job.id, "处理任务失败时出错：{error}");
        }
    }

    async fn handle_provider_error(
        &self,
        job: &JobRecord,
        purpose: Purpose,
        profile_id: Option<&str>,
        error: &ProviderError,
    ) -> Result<(), StoreError> {
        let code = error.code;
        if code.counts_toward_circuit_breaker() {
            self.with_profile(purpose, profile_id, |profile, now| {
                profile.breaker.on_failure(now)
            });
        }
        match code.retry_policy() {
            RetryPolicy::Requeue => {
                let wait = error
                    .retry_after
                    .unwrap_or(self.config.default_rate_limit_wait);
                self.with_profile(purpose, profile_id, |profile, now| {
                    profile.on_rate_limited(wait, now);
                });
                self.store.requeue_job(&job.id, true, now_ms()).await?;
                self.push_progress(job, JobStatus::Queued, Some(code));
                Ok(())
            }
            RetryPolicy::Retry => {
                let max = to_u32(job.max_attempts);
                self.retry_or_fail(job, code, &error.message, error.retry_after, max)
                    .await
            }
            RetryPolicy::RetryOnce => {
                self.retry_or_fail(job, code, &error.message, error.retry_after, 2)
                    .await
            }
            RetryPolicy::RewriteOnce => self.rewrite_or_fail(job, code, &error.message).await,
            RetryPolicy::Fail => self.fail(job, code, &error.message).await,
            RetryPolicy::DisableProfile | RetryPolicy::PauseProfile => {
                if let Some(profile_id) = profile_id {
                    self.disable_profile(purpose, profile_id, code, &error.message)
                        .await;
                }
                // 换别的配置再试，这次不算尝试次数
                self.store.requeue_job(&job.id, true, now_ms()).await?;
                self.push_progress(job, JobStatus::Queued, Some(code));
                Ok(())
            }
        }
    }

    async fn retry_or_fail(
        &self,
        job: &JobRecord,
        code: UpstreamErrorCode,
        message: &str,
        retry_after: Option<Duration>,
        max_attempts: u32,
    ) -> Result<(), StoreError> {
        let attempt = to_u32(job.attempt);
        if attempt >= max_attempts.min(to_u32(job.max_attempts)).max(1) {
            return self.fail(job, code, message).await;
        }
        let backoff = {
            let mut state = lock(&self.state);
            let cap = backoff_cap(attempt, self.config.base_backoff, self.config.max_backoff);
            jittered(cap, &mut state.rng)
        };
        let delay = retry_after.map_or(backoff, |wait| wait.max(backoff));
        let next = now_ms() + i64::try_from(delay.as_millis()).unwrap_or(i64::MAX / 2);
        self.store
            .schedule_retry(&job.id, next, code.as_str(), message, now_ms())
            .await?;
        self.push_progress(job, JobStatus::RetryWait, Some(code));
        Ok(())
    }

    async fn rewrite_or_fail(
        &self,
        job: &JobRecord,
        code: UpstreamErrorCode,
        message: &str,
    ) -> Result<(), StoreError> {
        if !job.rewritten {
            match self.handler.rewrite_after_rejection(job).await {
                Ok(Some(rewrite)) => {
                    self.store
                        .rewrite_and_requeue(
                            &job.id,
                            &rewrite.instruction,
                            rewrite.request_params.as_deref(),
                            now_ms(),
                        )
                        .await?;
                    self.push_progress(job, JobStatus::Queued, Some(code));
                    return Ok(());
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(job = %job.id, "改写指令失败：{error}"),
            }
        }
        self.fail(job, code, message).await
    }

    async fn fail(
        &self,
        job: &JobRecord,
        code: UpstreamErrorCode,
        message: &str,
    ) -> Result<(), StoreError> {
        self.store
            .fail_job(&job.id, code.as_str(), message, now_ms())
            .await?;
        self.push_progress(job, JobStatus::Failed, Some(code));
        self.check_failure_rate(&job.batch_id).await;
        self.check_finished(&job.batch_id).await;
        Ok(())
    }

    async fn disable_profile(
        &self,
        purpose: Purpose,
        profile_id: &str,
        code: UpstreamErrorCode,
        message: &str,
    ) {
        let newly_disabled = {
            let mut state = lock(&self.state);
            match state.profile_mut(purpose, profile_id) {
                Some(profile) if profile.disabled_reason.is_none() => {
                    profile.disabled_reason = Some(code.as_str().to_owned());
                    true
                }
                _ => false,
            }
        };
        if !newly_disabled {
            return;
        }
        if let Err(error) = self
            .store
            .set_profile_enabled(profile_id, false, Some(code.as_str()), now_ms())
            .await
        {
            tracing::error!(profile = profile_id, "停用配置失败：{error}");
        }
        self.sink.emit(EngineEvent::ProfileDisabled {
            profile_id: profile_id.to_owned(),
            reason: code.as_str().to_owned(),
            message: message.to_owned(),
        });
    }

    fn with_profile(
        &self,
        purpose: Purpose,
        profile_id: Option<&str>,
        f: impl FnOnce(&mut ProfileRuntime, Instant),
    ) {
        let Some(profile_id) = profile_id else { return };
        let mut state = lock(&self.state);
        if let Some(profile) = state.profile_mut(purpose, profile_id) {
            f(profile, Instant::now());
        }
    }

    // ---------- 批次 ----------

    async fn pause_batch(&self, batch_id: &str, reason: &str) {
        match self.store.pause_batch(batch_id, reason, now_ms()).await {
            Ok(()) => self.sink.emit(EngineEvent::BatchPaused {
                batch_id: batch_id.to_owned(),
                reason: reason.to_owned(),
            }),
            // 已经暂停或结束了，不用重复处理
            Err(StoreError::StateChanged { .. }) => {}
            Err(error) => tracing::error!(batch = batch_id, "暂停批次失败：{error}"),
        }
    }

    /// 失败率过高时暂停批次（详细设计 2.11）：通常是配置或网络出了问题，继续跑只会浪费额度。
    async fn check_failure_rate(&self, batch_id: &str) {
        let Ok(batch) = self.store.get_batch(batch_id).await else {
            return;
        };
        let done = batch.succeeded_jobs + batch.failed_jobs;
        if batch.status == BatchStatus::Running
            && done >= self.config.failure_pause_min_jobs
            && (batch.failed_jobs as f64) / (done as f64) > self.config.failure_pause_ratio
        {
            self.pause_batch(batch_id, "high_failure_rate").await;
        }
    }

    async fn check_finished(&self, batch_id: &str) {
        let Ok(batch) = self.store.get_batch(batch_id).await else {
            return;
        };
        let finished = matches!(
            batch.status,
            BatchStatus::Completed | BatchStatus::Failed | BatchStatus::Cancelled
        );
        if finished && lock(&self.state).finished.insert(batch_id.to_owned()) {
            // 先把攒着的进度推出去，界面收到"结束"时进度已经是最新的
            self.flush_progress();
            self.sink.emit(EngineEvent::BatchFinished {
                batch_id: batch_id.to_owned(),
                status: batch.status,
            });
        }
    }

    // ---------- 轮询 ----------

    async fn poll_loop(self: Arc<Self>) {
        let semaphore = Arc::new(Semaphore::new(self.config.poll_concurrency.max(1)));
        while !self.stopping.load(Ordering::SeqCst) {
            tokio::time::sleep(self.config.poll_interval).await;
            let jobs = match self.store.polling_jobs(POLL_BATCH).await {
                Ok(jobs) => jobs,
                Err(error) => {
                    tracing::error!("读取轮询中的任务失败：{error}");
                    continue;
                }
            };
            for job in jobs {
                if !lock(&self.state).polling.insert(job.id.clone()) {
                    continue;
                }
                let Ok(permit) = Arc::clone(&semaphore).acquire_owned().await else {
                    break;
                };
                let inner = Arc::clone(&self);
                self.workers.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    inner.poll_one(&job).await;
                    lock(&inner.state).polling.remove(&job.id);
                    drop(permit);
                    inner.wake.notify_one();
                    if inner.workers.fetch_sub(1, Ordering::SeqCst) == 1 {
                        inner.idle.notify_waiters();
                    }
                });
            }
        }
    }

    async fn poll_one(&self, job: &JobRecord) {
        let purpose = Purpose::Image;
        let provider = job.profile_id.as_deref().and_then(|id| {
            let mut state = lock(&self.state);
            state.profile_mut(purpose, id).map(|p| p.provider.clone())
        });
        let (Some(ProviderHandle::Image(provider)), Some(remote_task_id)) =
            (provider, job.remote_task_id.as_deref())
        else {
            // 配置被删了或者数据不完整，只能重新提交
            if let Err(error) = self.store.requeue_job(&job.id, false, now_ms()).await {
                tracing::error!(job = %job.id, "轮询任务找不到配置，放回队列失败：{error}");
            }
            return;
        };

        let started = job.started_at.unwrap_or(job.updated_at);
        let timeout_ms =
            i64::try_from(self.config.async_total_timeout.as_millis()).unwrap_or(i64::MAX);
        if now_ms() - started > timeout_ms {
            let error =
                ProviderError::new(UpstreamErrorCode::Timeout, "异步任务超过总等待时间还没完成");
            self.handle_failure(job, Failure::Provider(error), None)
                .await;
            return;
        }

        match provider.poll(remote_task_id).await {
            Ok(PollOutcome::Pending) => {}
            Ok(PollOutcome::Done(images)) => {
                match self.handler.on_output(job, JobOutput::Images(images)).await {
                    Ok(()) => {
                        self.push_progress(job, JobStatus::Succeeded, None);
                        self.check_finished(&job.batch_id).await;
                    }
                    Err(error) => {
                        self.handle_failure(job, Failure::Handler(error), None)
                            .await
                    }
                }
            }
            Err(error) if poll_error_is_final(&error) => {
                self.handle_failure(job, Failure::Provider(error), None)
                    .await;
            }
            Err(error) => {
                // 查询本身临时出错（限流、5xx、网络抖动）：远端任务还在，下一轮接着查，
                // 不能重新提交，否则同一张图要付两次钱
                tracing::debug!(job = %job.id, "轮询暂时失败，稍后再查：{error}");
            }
        }
    }

    // ---------- 进度 ----------

    fn push_progress(&self, job: &JobRecord, status: JobStatus, code: Option<UpstreamErrorCode>) {
        self.progress.push(
            &job.batch_id,
            JobChange {
                job_id: job.id.clone(),
                status,
                error_code: code.map(|c| c.as_str().to_owned()),
            },
        );
    }

    fn flush_progress(&self) {
        for event in self.progress.drain() {
            self.sink.emit(event);
        }
    }

    async fn progress_loop(self: Arc<Self>) {
        let mut interval = tokio::time::interval(self.config.progress_interval);
        while !self.stopping.load(Ordering::SeqCst) {
            interval.tick().await;
            self.flush_progress();
        }
    }
}

enum Reserve {
    Reserved(Reservation),
    Busy,
    NoProfile,
}

/// 轮询出错时，只有这几种情况需要按失败处理：
/// - 远端明确告诉我们任务失败了（供应商构造的错误，没有 HTTP 状态码）
/// - 404：远端找不到这个任务，只能重新提交
/// - Key 无效、余额不足：要停用配置
fn poll_error_is_final(error: &ProviderError) -> bool {
    let remote_task_failed =
        error.status.is_none() && error.code == UpstreamErrorCode::UpstreamError;
    remote_task_failed
        || error.status == Some(404)
        || matches!(
            error.code,
            UpstreamErrorCode::Auth | UpstreamErrorCode::QuotaExhausted
        )
}

fn to_u32(value: i64) -> u32 {
    u32::try_from(value.max(0)).unwrap_or(u32::MAX)
}
