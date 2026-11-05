use std::time::Duration;

use prism_core::defaults;

/// 引擎参数。默认值见 `docs/detail-design.md` 第 8 章；测试里会把时间调短。
#[derive(Debug, Clone, PartialEq)]
pub struct EngineConfig {
    /// 生图全局并发
    pub image_concurrency: u32,
    /// 文本（规划、Agent）全局并发
    pub text_concurrency: u32,
    /// 退避基数和上限：第 n 次失败最多等 `min(max, base × 2^(n-1))`
    pub base_backoff: Duration,
    pub max_backoff: Duration,
    /// 429 没带 `Retry-After` 时暂停这个配置多久
    pub default_rate_limit_wait: Duration,
    /// 每成功多少次并发上限加 1
    pub success_streak_to_grow: u32,
    /// 持续被限流时，两次下调并发上限之间至少隔多久
    pub rate_limit_cut_cooldown: Duration,
    /// 连续多少次上游故障后熔断，熔断多久
    pub breaker_threshold: u32,
    pub breaker_cooldown: Duration,
    /// 异步任务的轮询间隔、总超时、同时轮询的数量
    pub poll_interval: Duration,
    pub async_total_timeout: Duration,
    pub poll_concurrency: usize,
    /// 批次已结束的生图任务达到这个数、且失败率超过比例时自动暂停批次
    pub failure_pause_min_jobs: i64,
    pub failure_pause_ratio: f64,
    /// 没有事情可做时最多睡多久再检查一次（兜底，正常靠通知唤醒）
    pub idle_wakeup: Duration,
    /// 进度事件合并推送的间隔
    pub progress_interval: Duration,
    /// 调度器每轮最多取多少个待派发任务
    pub dispatch_batch: i64,
    /// 随机数种子（选配置、退避抖动），测试时固定
    pub seed: Option<u64>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            image_concurrency: defaults::DEFAULT_GLOBAL_IMAGE_CONCURRENCY,
            text_concurrency: defaults::DEFAULT_PLAN_CONCURRENCY,
            base_backoff: Duration::from_secs(2),
            max_backoff: Duration::from_secs(60),
            default_rate_limit_wait: Duration::from_secs(10),
            success_streak_to_grow: 10,
            rate_limit_cut_cooldown: Duration::from_secs(10),
            breaker_threshold: 5,
            breaker_cooldown: Duration::from_secs(60),
            poll_interval: Duration::from_secs(defaults::DEFAULT_POLL_INTERVAL_SECS),
            async_total_timeout: Duration::from_secs(defaults::ASYNC_TOTAL_TIMEOUT_SECS),
            poll_concurrency: 8,
            failure_pause_min_jobs: 20,
            failure_pause_ratio: 0.5,
            idle_wakeup: Duration::from_secs(2),
            progress_interval: Duration::from_millis(defaults::PROGRESS_PUSH_INTERVAL_MS),
            dispatch_batch: 256,
            seed: None,
        }
    }
}
