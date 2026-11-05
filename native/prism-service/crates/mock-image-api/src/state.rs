//! 共享状态：错误注入、每分钟限额、统计、异步任务。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::Serialize;

use crate::config::MockConfig;

const RATE_WINDOW: Duration = Duration::from_secs(60);

/// 一次请求的处理结果，由错误注入决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    RateLimited { retry_after_secs: u64 },
    ServerError,
    Hang,
    BadJson,
    ContentRejected,
}

/// `/stats` 返回的统计。
#[derive(Debug, Clone, Default, Serialize)]
pub struct Stats {
    pub requests_total: u64,
    pub requests_by_path: HashMap<String, u64>,
    pub responses_by_status: HashMap<u16, u64>,
    pub images_generated: u64,
    /// 每个任务 ID（请求头 `X-Prism-Job-Id`）成功生成的次数
    pub job_successes: HashMap<String, u32>,
    /// 成功生成超过一次的任务数，压测时必须为 0（异步模式）或等于崩溃时的在途数（同步模式）
    pub duplicate_jobs: u64,
}

/// 异步任务：提交时定好完成时间，轮询时到点就返回结果。
#[derive(Debug, Clone)]
pub struct AsyncTask {
    pub ready_at: Instant,
    pub images: u32,
    pub prompt: String,
    pub job_id: Option<String>,
    pub counted: bool,
}

#[derive(Debug)]
struct RateWindow {
    started: Instant,
    count: u32,
}

#[derive(Debug)]
pub struct AppState {
    pub config: MockConfig,
    rng: Mutex<StdRng>,
    window: Mutex<RateWindow>,
    stats: Mutex<Stats>,
    pub tasks: Mutex<HashMap<String, AsyncTask>>,
}

impl AppState {
    pub fn new(config: MockConfig) -> Self {
        let rng = StdRng::seed_from_u64(config.seed);
        Self {
            config,
            rng: Mutex::new(rng),
            window: Mutex::new(RateWindow {
                started: Instant::now(),
                count: 0,
            }),
            stats: Mutex::new(Stats::default()),
            tasks: Mutex::new(HashMap::new()),
        }
    }

    /// 决定这次请求成功还是注入哪种错误。先看每分钟限额，再按比例掷骰子。
    pub fn decide(&self) -> Outcome {
        if let Some(limit) = self.config.rate_limit_per_min {
            let mut window = lock(&self.window);
            let elapsed = window.started.elapsed();
            if elapsed >= RATE_WINDOW {
                window.started = Instant::now();
                window.count = 0;
            } else if window.count >= limit {
                let remaining = RATE_WINDOW.saturating_sub(elapsed).as_secs().max(1);
                return Outcome::RateLimited {
                    retry_after_secs: remaining,
                };
            }
            window.count += 1;
        }

        let roll: f64 = lock(&self.rng).random();
        let config = &self.config;
        let thresholds = [
            (
                config.error_429,
                Outcome::RateLimited {
                    retry_after_secs: 1,
                },
            ),
            (config.error_500, Outcome::ServerError),
            (config.timeout, Outcome::Hang),
            (config.bad_json, Outcome::BadJson),
            (config.content_rejected, Outcome::ContentRejected),
        ];
        let mut cumulative = 0.0;
        for (rate, outcome) in thresholds {
            cumulative += rate;
            if roll < cumulative {
                return outcome;
            }
        }
        Outcome::Success
    }

    /// 本次成功响应前的延迟。
    pub fn latency(&self) -> Duration {
        let (min, max) = self.config.latency_ms;
        if max == 0 {
            return Duration::ZERO;
        }
        Duration::from_millis(lock(&self.rng).random_range(min..=max))
    }

    /// 生成图片颜色用的随机数。
    pub fn next_seed(&self) -> u64 {
        lock(&self.rng).random()
    }

    pub fn record_request(&self, path: &str) {
        let mut stats = lock(&self.stats);
        stats.requests_total += 1;
        *stats.requests_by_path.entry(path.to_owned()).or_default() += 1;
    }

    pub fn record_status(&self, status: u16) {
        *lock(&self.stats)
            .responses_by_status
            .entry(status)
            .or_default() += 1;
    }

    /// 记一次成功生成。同一个任务 ID 第二次成功时计入重复数。
    pub fn record_generation(&self, job_id: Option<&str>, images: u32) {
        let mut stats = lock(&self.stats);
        stats.images_generated += u64::from(images);
        if let Some(job_id) = job_id {
            let count = stats.job_successes.entry(job_id.to_owned()).or_default();
            *count += 1;
            if *count == 2 {
                stats.duplicate_jobs += 1;
            }
        }
    }

    pub fn stats(&self) -> Stats {
        lock(&self.stats).clone()
    }

    pub fn reset_stats(&self) {
        *lock(&self.stats) = Stats::default();
    }
}

/// 锁被毒化（持锁的线程 panic）时继续使用里面的数据。模拟服务只做统计，数据不一致的后果可以接受。
pub fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_gives_same_outcomes() {
        let config = MockConfig {
            error_500: 0.3,
            error_429: 0.2,
            ..MockConfig::default()
        };
        let a = AppState::new(config.clone());
        let b = AppState::new(config);
        let run = |state: &AppState| (0..50).map(|_| state.decide()).collect::<Vec<_>>();
        assert_eq!(run(&a), run(&b));
    }

    #[test]
    fn error_rates_are_roughly_respected() {
        let state = AppState::new(MockConfig {
            error_500: 0.2,
            ..MockConfig::default()
        });
        let errors = (0..5000)
            .filter(|_| state.decide() == Outcome::ServerError)
            .count();
        // 期望 1000，允许正负 15%
        assert!((850..=1150).contains(&errors), "errors = {errors}");
    }

    #[test]
    fn per_minute_limit_kicks_in() {
        let state = AppState::new(MockConfig {
            rate_limit_per_min: Some(3),
            ..MockConfig::default()
        });
        let outcomes: Vec<_> = (0..5).map(|_| state.decide()).collect();
        assert_eq!(&outcomes[..3], &[Outcome::Success; 3]);
        assert!(
            matches!(outcomes[3], Outcome::RateLimited { retry_after_secs } if retry_after_secs >= 1)
        );
    }

    #[test]
    fn duplicate_successes_are_counted_once_per_job() {
        let state = AppState::new(MockConfig::default());
        state.record_generation(Some("job-1"), 1);
        state.record_generation(Some("job-1"), 1);
        state.record_generation(Some("job-1"), 1);
        state.record_generation(Some("job-2"), 1);
        let stats = state.stats();
        assert_eq!(stats.duplicate_jobs, 1);
        assert_eq!(stats.images_generated, 4);
        assert_eq!(stats.job_successes["job-1"], 3);
    }
}
