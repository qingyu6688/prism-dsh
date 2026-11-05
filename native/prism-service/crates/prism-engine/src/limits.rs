//! 限流和保护部件：令牌桶、自适应并发（AIMD）、熔断、退避。规则见 `docs/detail-design.md` 2.3～2.7。
//!
//! 都是纯状态机，时间由调用方传入，方便测试。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use rand::Rng;

/// 令牌桶：每分钟最多 `per_minute` 个请求，允许瞬间用完一分钟的额度。
#[derive(Debug, Clone)]
pub struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_per_sec: f64,
    last: Instant,
}

impl TokenBucket {
    pub fn per_minute(per_minute: u32, now: Instant) -> Self {
        let capacity = f64::from(per_minute.max(1));
        Self {
            capacity,
            tokens: capacity,
            refill_per_sec: capacity / 60.0,
            last: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        self.last = now;
    }

    pub fn has_token(&mut self, now: Instant) -> bool {
        self.refill(now);
        self.tokens >= 1.0
    }

    /// 取一个令牌，没有时返回 `false`。
    pub fn try_take(&mut self, now: Instant) -> bool {
        self.refill(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// 还要等多久才有下一个令牌。
    pub fn wait_time(&mut self, now: Instant) -> Duration {
        self.refill(now);
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64((1.0 - self.tokens) / self.refill_per_sec)
        }
    }
}

/// 判断"持续被限流"看最近多少个响应。
const AIMD_WINDOW: usize = 20;
/// 样本少于这个数时不下调，避免刚开始一两个 429 就误判。
const AIMD_MIN_SAMPLES: usize = 10;
/// 最近的响应里 429 至少占这个比例才下调。
const AIMD_CUT_THRESHOLD: f64 = 0.3;
/// 每次下调到原来的 3/4。
const AIMD_CUT_NUMERATOR: u32 = 3;
const AIMD_CUT_DENOMINATOR: u32 = 4;

/// 自适应并发上限。
///
/// - **下调**：最近 20 个响应里 429 占 30% 以上才下调，一次降到 3/4（不低于 1），两次下调至少间隔一段冷却时间
/// - **上调**：每成功若干次加 1（不超过配置上限）；偶尔的 429 不打断计数
///
/// 零星的 429 已经由"按 `Retry-After` 暂停配置"处理，不需要降并发；只有持续被限流才说明并发确实开高了。
/// 最初的版本每个 429 都减半，压测时几十个在途请求同时收到 429，并发瞬间被压到 1，
/// 随机 429 又不断打断恢复，吞吐掉到原来的 1/20（见 `docs/test.md` 6.6）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aimd {
    max: u32,
    current: u32,
    successes: u32,
    increase_after: u32,
    cut_cooldown: Duration,
    last_cut: Option<Instant>,
    /// 最近的响应，`true` 表示 429
    recent: VecDeque<bool>,
}

impl Aimd {
    pub fn new(max: u32, increase_after: u32, cut_cooldown: Duration) -> Self {
        let max = max.max(1);
        Self {
            max,
            current: max,
            successes: 0,
            increase_after: increase_after.max(1),
            cut_cooldown,
            last_cut: None,
            recent: VecDeque::with_capacity(AIMD_WINDOW),
        }
    }

    pub fn limit(&self) -> u32 {
        self.current
    }

    fn record(&mut self, rate_limited: bool) {
        if self.recent.len() == AIMD_WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(rate_limited);
    }

    pub fn on_success(&mut self) {
        self.record(false);
        self.successes += 1;
        if self.successes >= self.increase_after {
            self.successes = 0;
            self.current = (self.current + 1).min(self.max);
        }
    }

    pub fn on_rate_limited(&mut self, now: Instant) {
        self.record(true);
        let samples = self.recent.len();
        let limited = self.recent.iter().filter(|r| **r).count();
        let sustained = samples >= AIMD_MIN_SAMPLES
            && (limited as f64) / (samples as f64) >= AIMD_CUT_THRESHOLD;
        let cooled_down = self
            .last_cut
            .is_none_or(|last| now.saturating_duration_since(last) >= self.cut_cooldown);
        if sustained && cooled_down {
            self.current = (self.current * AIMD_CUT_NUMERATOR / AIMD_CUT_DENOMINATOR).max(1);
            self.last_cut = Some(now);
            // 下调后重新攒证据，避免同一批 429 连续触发
            self.recent.clear();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    Closed,
    Open,
    HalfOpen,
}

/// 熔断：连续失败达到阈值后打开，冷却后放一个请求试探，成功关闭，失败重新打开。
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    threshold: u32,
    cooldown: Duration,
    failures: u32,
    open_until: Option<Instant>,
    probing: bool,
}

impl CircuitBreaker {
    pub fn new(threshold: u32, cooldown: Duration) -> Self {
        Self {
            threshold: threshold.max(1),
            cooldown,
            failures: 0,
            open_until: None,
            probing: false,
        }
    }

    pub fn state(&self, now: Instant) -> BreakerState {
        match self.open_until {
            None => BreakerState::Closed,
            Some(until) if now < until => BreakerState::Open,
            Some(_) => BreakerState::HalfOpen,
        }
    }

    /// 现在能不能发请求。半开状态下只允许一个试探请求在途。
    pub fn allows(&self, now: Instant) -> bool {
        match self.state(now) {
            BreakerState::Closed => true,
            BreakerState::Open => false,
            BreakerState::HalfOpen => !self.probing,
        }
    }

    /// 真正发出请求时调用，半开状态下占住试探名额。返回这次是不是试探请求。
    pub fn on_dispatch(&mut self, now: Instant) -> bool {
        let probe = self.state(now) == BreakerState::HalfOpen;
        if probe {
            self.probing = true;
        }
        probe
    }

    /// 试探请求结束了，但既没成功也没失败（限流、没抢到任务、本地出错）：
    /// 让出试探名额，下一个请求接着试探。不让出的话名额一直被占，配置再也派不出任务。
    pub fn end_probe(&mut self) {
        self.probing = false;
    }

    pub fn on_success(&mut self) {
        self.failures = 0;
        self.open_until = None;
        self.probing = false;
    }

    pub fn on_failure(&mut self, now: Instant) {
        self.probing = false;
        if self.open_until.is_some() {
            // 半开试探失败，重新打开
            self.open_until = Some(now + self.cooldown);
            return;
        }
        self.failures += 1;
        if self.failures >= self.threshold {
            self.open_until = Some(now + self.cooldown);
        }
    }

    /// 还要多久才进入半开状态，调度器据此决定什么时候醒来。
    pub fn reopens_in(&self, now: Instant) -> Option<Duration> {
        self.open_until
            .filter(|until| *until > now)
            .map(|until| until - now)
    }
}

/// 第 `attempt` 次失败后的退避上限：`min(max, base × 2^(attempt-1))`。
pub fn backoff_cap(attempt: u32, base: Duration, max: Duration) -> Duration {
    let exponent = attempt.saturating_sub(1).min(20);
    base.saturating_mul(1 << exponent).min(max)
}

/// 完全抖动：在 0 到上限之间随机取，避免大量任务同时重试。
pub fn jittered(cap: Duration, rng: &mut impl Rng) -> Duration {
    if cap.is_zero() {
        return Duration::ZERO;
    }
    let millis = u64::try_from(cap.as_millis()).unwrap_or(u64::MAX);
    Duration::from_millis(rng.random_range(0..=millis))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    #[test]
    fn backoff_doubles_until_capped() {
        // UT-E03
        let base = Duration::from_secs(2);
        let max = Duration::from_secs(60);
        let caps: Vec<u64> = (1..=6)
            .map(|a| backoff_cap(a, base, max).as_secs())
            .collect();
        assert_eq!(caps, vec![2, 4, 8, 16, 32, 60]);
        assert_eq!(backoff_cap(100, base, max), max);
    }

    #[test]
    fn jitter_stays_within_cap() {
        let mut rng = StdRng::seed_from_u64(1);
        let cap = Duration::from_secs(8);
        for _ in 0..1000 {
            assert!(jittered(cap, &mut rng) <= cap);
        }
        assert_eq!(jittered(Duration::ZERO, &mut rng), Duration::ZERO);
    }

    #[test]
    fn token_bucket_limits_bursts_and_refills() {
        // UT-E05：每分钟 60 次，瞬间请求 100 次，前 60 次放行
        let start = Instant::now();
        let mut bucket = TokenBucket::per_minute(60, start);
        let allowed = (0..100).filter(|_| bucket.try_take(start)).count();
        assert_eq!(allowed, 60);
        assert!(bucket.wait_time(start) > Duration::ZERO);
        // 1 秒后补回 1 个
        assert!(bucket.try_take(start + Duration::from_secs(1)));
        assert!(!bucket.try_take(start + Duration::from_secs(1)));
    }

    const COOLDOWN: Duration = Duration::from_secs(10);

    #[test]
    fn occasional_rate_limits_do_not_lower_concurrency() {
        // 10% 的随机 429：不该下调（压测里发现的问题，见 docs/test.md 6.6）
        let now = Instant::now();
        let mut aimd = Aimd::new(8, 10, COOLDOWN);
        for round in 0..20 {
            for _ in 0..9 {
                aimd.on_success();
            }
            aimd.on_rate_limited(now + Duration::from_secs(round * 11));
        }
        assert_eq!(aimd.limit(), 8);
    }

    #[test]
    fn simultaneous_rate_limits_cut_only_once() {
        // UT-E06：几十个在途请求同时收到 429，只下调一次
        let now = Instant::now();
        let mut aimd = Aimd::new(8, 10, COOLDOWN);
        for _ in 0..50 {
            aimd.on_rate_limited(now);
        }
        assert_eq!(aimd.limit(), 6);
    }

    #[test]
    fn sustained_rate_limits_keep_cutting_after_cooldown_but_never_to_zero() {
        // UT-E06：持续被限流时每个冷却期降到 3/4，最低为 1
        let start = Instant::now();
        let mut aimd = Aimd::new(8, 10, COOLDOWN);
        let mut limits = Vec::new();
        for round in 0..8 {
            let at = start + COOLDOWN * round;
            for _ in 0..AIMD_MIN_SAMPLES {
                aimd.on_rate_limited(at);
            }
            limits.push(aimd.limit());
        }
        assert_eq!(limits, vec![6, 4, 3, 2, 1, 1, 1, 1]);
    }

    #[test]
    fn grows_back_after_successes_even_with_occasional_429s() {
        // UT-E07：每成功 10 次加 1，偶尔的 429 不打断计数，不超过上限
        let now = Instant::now();
        let mut aimd = Aimd::new(8, 10, COOLDOWN);
        // 正好攒够 10 个 429，下调一次并清空记录
        for _ in 0..AIMD_MIN_SAMPLES {
            aimd.on_rate_limited(now);
        }
        assert_eq!(aimd.limit(), 6);
        for _ in 0..5 {
            aimd.on_success();
        }
        aimd.on_rate_limited(now + COOLDOWN * 2);
        for _ in 0..5 {
            aimd.on_success();
        }
        assert_eq!(aimd.limit(), 7);
        for _ in 0..100 {
            aimd.on_success();
        }
        assert_eq!(aimd.limit(), 8);
    }

    #[test]
    fn breaker_opens_then_probes_then_closes() {
        // UT-E08
        let start = Instant::now();
        let cooldown = Duration::from_secs(60);
        let mut breaker = CircuitBreaker::new(5, cooldown);
        for _ in 0..4 {
            breaker.on_failure(start);
        }
        assert!(breaker.allows(start));
        breaker.on_failure(start);
        assert_eq!(breaker.state(start), BreakerState::Open);
        assert!(!breaker.allows(start + Duration::from_secs(59)));
        assert_eq!(breaker.reopens_in(start), Some(cooldown));

        let later = start + cooldown;
        assert_eq!(breaker.state(later), BreakerState::HalfOpen);
        assert!(breaker.allows(later));
        breaker.on_dispatch(later);
        assert!(!breaker.allows(later), "半开时只放一个试探");
        breaker.on_success();
        assert_eq!(breaker.state(later), BreakerState::Closed);
    }

    #[test]
    fn probe_without_verdict_frees_the_slot() {
        // 回归：PT-03 里试探请求遇到 429，熔断一直停在半开、试探名额被占，引擎再也不派任务
        let start = Instant::now();
        let cooldown = Duration::from_secs(60);
        let mut breaker = CircuitBreaker::new(1, cooldown);
        breaker.on_failure(start);
        let probe_time = start + cooldown;
        assert!(breaker.on_dispatch(probe_time));
        assert!(!breaker.allows(probe_time));
        breaker.end_probe();
        assert_eq!(breaker.state(probe_time), BreakerState::HalfOpen);
        assert!(breaker.allows(probe_time));
        // 关闭状态下发出的不是试探请求
        breaker.on_success();
        assert!(!breaker.on_dispatch(probe_time));
    }

    #[test]
    fn failed_probe_reopens_breaker() {
        let start = Instant::now();
        let cooldown = Duration::from_secs(60);
        let mut breaker = CircuitBreaker::new(1, cooldown);
        breaker.on_failure(start);
        let probe_time = start + cooldown;
        breaker.on_dispatch(probe_time);
        breaker.on_failure(probe_time);
        assert_eq!(breaker.state(probe_time), BreakerState::Open);
        assert_eq!(breaker.reopens_in(probe_time), Some(cooldown));
    }
}
