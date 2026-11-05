//! 每个 API 配置的运行时状态（在途数、自适应上限、令牌桶、熔断、暂停）和选配置的规则（详细设计 2.4）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use prism_provider::{ImageProvider, TextProvider};
use rand::Rng;
use serde::Serialize;

use crate::EngineConfig;
use crate::limits::{Aimd, BreakerState, CircuitBreaker, TokenBucket};

/// 配置的调用对象：生图配置持有生图供应商，文本配置持有文本供应商。
#[derive(Clone)]
pub enum ProviderHandle {
    Image(Arc<dyn ImageProvider>),
    Text(Arc<dyn TextProvider>),
}

/// 配置里和限流有关的数字。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileLimits {
    pub weight: u32,
    pub max_concurrency: u32,
    /// 每分钟请求上限，`None` 不限
    pub rpm_limit: Option<u32>,
}

/// 一个配置的运行时状态。
pub struct ProfileRuntime {
    pub id: String,
    pub name: String,
    pub weight: u32,
    pub provider: ProviderHandle,
    pub inflight: u32,
    pub aimd: Aimd,
    pub bucket: Option<TokenBucket>,
    pub breaker: CircuitBreaker,
    /// 遇到 429 后暂停到这个时间
    pub paused_until: Option<Instant>,
    /// Key 无效、余额不足等需要用户处理的情况，停用后不再派发
    pub disabled_reason: Option<String>,
}

/// 设置页显示用的快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSnapshot {
    pub profile_id: String,
    pub name: String,
    pub inflight: u32,
    pub current_limit: u32,
    pub breaker: &'static str,
    pub paused: bool,
    pub disabled_reason: Option<String>,
}

impl ProfileRuntime {
    pub fn new(
        id: String,
        name: String,
        limits: ProfileLimits,
        provider: ProviderHandle,
        config: &EngineConfig,
        now: Instant,
    ) -> Self {
        Self {
            id,
            name,
            weight: limits.weight.max(1),
            provider,
            inflight: 0,
            aimd: Aimd::new(
                limits.max_concurrency,
                config.success_streak_to_grow,
                config.rate_limit_cut_cooldown,
            ),
            bucket: limits
                .rpm_limit
                .map(|rpm| TokenBucket::per_minute(rpm, now)),
            breaker: CircuitBreaker::new(config.breaker_threshold, config.breaker_cooldown),
            paused_until: None,
            disabled_reason: None,
        }
    }

    /// 现在能不能再派一个任务给它。
    pub fn can_accept(&mut self, now: Instant) -> bool {
        self.disabled_reason.is_none()
            && self.paused_until.is_none_or(|until| until <= now)
            && self.breaker.allows(now)
            && self.inflight < self.aimd.limit()
            && self
                .bucket
                .as_mut()
                .is_none_or(|bucket| bucket.has_token(now))
    }

    pub fn free_slots(&self) -> u32 {
        self.aimd.limit().saturating_sub(self.inflight)
    }

    /// 派发任务时占位：在途数加 1、取令牌、半开时占住试探名额。返回这次是不是熔断试探。
    pub fn acquire(&mut self, now: Instant) -> bool {
        self.inflight += 1;
        if let Some(bucket) = self.bucket.as_mut() {
            bucket.try_take(now);
        }
        self.breaker.on_dispatch(now)
    }

    /// 任务结束（不管结果如何）时调用。`probe` 是 [`Self::acquire`] 的返回值：
    /// 试探请求的结果已经报告过的话，这里让出名额没有副作用；没报告的话（429、没抢到任务等）
    /// 必须在这里让出，否则熔断会一直卡在半开。
    pub fn release(&mut self, probe: bool) {
        self.inflight = self.inflight.saturating_sub(1);
        if probe {
            self.breaker.end_probe();
        }
    }

    pub fn on_success(&mut self) {
        self.aimd.on_success();
        self.breaker.on_success();
    }

    pub fn on_rate_limited(&mut self, wait: Duration, now: Instant) {
        self.aimd.on_rate_limited(now);
        let until = now + wait;
        self.paused_until = Some(
            self.paused_until
                .map_or(until, |current| current.max(until)),
        );
    }

    /// 什么时候可能重新可用，调度器据此设置醒来时间。
    pub fn available_in(&mut self, now: Instant) -> Option<Duration> {
        if self.disabled_reason.is_some() {
            return None;
        }
        let mut waits = Vec::new();
        if let Some(until) = self.paused_until.filter(|until| *until > now) {
            waits.push(until - now);
        }
        if let Some(wait) = self.breaker.reopens_in(now) {
            waits.push(wait);
        }
        if let Some(bucket) = self.bucket.as_mut() {
            let wait = bucket.wait_time(now);
            if !wait.is_zero() {
                waits.push(wait);
            }
        }
        waits.into_iter().max()
    }

    pub fn snapshot(&self, now: Instant) -> ProfileSnapshot {
        ProfileSnapshot {
            profile_id: self.id.clone(),
            name: self.name.clone(),
            inflight: self.inflight,
            current_limit: self.aimd.limit(),
            breaker: match self.breaker.state(now) {
                BreakerState::Closed => "closed",
                BreakerState::Open => "open",
                BreakerState::HalfOpen => "half_open",
            },
            paused: self.paused_until.is_some_and(|until| until > now),
            disabled_reason: self.disabled_reason.clone(),
        }
    }
}

/// 选配置：排除不可用的；能避开 `avoid`（上次失败的配置）就避开；
/// 剩下的按 `权重 × 空闲名额` 加权随机。返回下标。
pub fn choose_profile(
    profiles: &mut [ProfileRuntime],
    avoid: Option<&str>,
    now: Instant,
    rng: &mut impl Rng,
) -> Option<usize> {
    let available: Vec<usize> = (0..profiles.len())
        .filter(|&i| profiles[i].can_accept(now))
        .collect();
    let preferred: Vec<usize> = available
        .iter()
        .copied()
        .filter(|&i| Some(profiles[i].id.as_str()) != avoid)
        .collect();
    let candidates = if preferred.is_empty() {
        available
    } else {
        preferred
    };
    let weights: Vec<u64> = candidates
        .iter()
        .map(|&i| u64::from(profiles[i].weight) * u64::from(profiles[i].free_slots()))
        .collect();
    let total: u64 = weights.iter().sum();
    if total == 0 {
        return None;
    }
    let mut pick = rng.random_range(0..total);
    for (index, weight) in candidates.iter().zip(weights) {
        if pick < weight {
            return Some(*index);
        }
        pick -= weight;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use prism_provider::{ChatRequest, ChatResponse, ProviderError};
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    struct NoopText;

    #[async_trait]
    impl TextProvider for NoopText {
        async fn chat(&self, _request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
            Ok(ChatResponse {
                content: None,
                tool_calls: Vec::new(),
                finish_reason: None,
            })
        }
    }

    fn runtime(id: &str, max: u32, now: Instant) -> ProfileRuntime {
        ProfileRuntime::new(
            id.into(),
            id.into(),
            ProfileLimits {
                weight: 1,
                max_concurrency: max,
                rpm_limit: None,
            },
            ProviderHandle::Text(Arc::new(NoopText)),
            &EngineConfig::default(),
            now,
        )
    }

    #[test]
    fn picks_the_only_available_profile() {
        // UT-E10：一个熔断、一个满载、一个空闲，选空闲的
        let now = Instant::now();
        let mut profiles = vec![
            runtime("open", 4, now),
            runtime("full", 1, now),
            runtime("idle", 4, now),
        ];
        for _ in 0..5 {
            profiles[0].breaker.on_failure(now);
        }
        profiles[1].acquire(now);
        let mut rng = StdRng::seed_from_u64(7);
        for _ in 0..20 {
            assert_eq!(choose_profile(&mut profiles, None, now, &mut rng), Some(2));
        }
    }

    #[test]
    fn avoids_last_failed_profile_when_possible() {
        let now = Instant::now();
        let mut profiles = vec![runtime("a", 4, now), runtime("b", 4, now)];
        let mut rng = StdRng::seed_from_u64(7);
        for _ in 0..20 {
            assert_eq!(
                choose_profile(&mut profiles, Some("a"), now, &mut rng),
                Some(1)
            );
        }
        // 只剩它一个时还是会选它
        profiles[1].disabled_reason = Some("auth".into());
        assert_eq!(
            choose_profile(&mut profiles, Some("a"), now, &mut rng),
            Some(0)
        );
    }

    #[test]
    fn rate_limited_probe_does_not_block_the_profile() {
        // 回归 PT-03：熔断半开时派出的试探请求遇到 429，释放后配置还能接着派发
        let start = Instant::now();
        let mut profile = runtime("a", 8, start);
        for _ in 0..5 {
            profile.breaker.on_failure(start);
        }
        let half_open = start + EngineConfig::default().breaker_cooldown;
        let probe = profile.acquire(half_open);
        assert!(probe);
        assert!(!profile.can_accept(half_open));
        profile.on_rate_limited(Duration::from_secs(1), half_open);
        profile.release(probe);
        assert!(profile.can_accept(half_open + Duration::from_secs(1)));
    }

    #[test]
    fn rate_limit_pauses_the_profile() {
        let now = Instant::now();
        let mut profile = runtime("a", 8, now);
        profile.on_rate_limited(Duration::from_secs(3), now);
        assert!(!profile.can_accept(now));
        // 单个 429 只暂停，不下调并发
        assert_eq!(profile.aimd.limit(), 8);
        assert_eq!(profile.available_in(now), Some(Duration::from_secs(3)));
        assert!(profile.can_accept(now + Duration::from_secs(3)));
    }

    #[test]
    fn weights_spread_load_by_free_slots() {
        let now = Instant::now();
        let mut profiles = vec![runtime("big", 8, now), runtime("small", 2, now)];
        let mut rng = StdRng::seed_from_u64(3);
        let big = (0..1000)
            .filter(|_| choose_profile(&mut profiles, None, now, &mut rng) == Some(0))
            .count();
        // 期望 800 左右
        assert!((700..=900).contains(&big), "big = {big}");
    }
}
