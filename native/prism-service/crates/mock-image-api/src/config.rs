//! 模拟服务的配置：延迟、限额、各类错误的注入比例。

use std::time::Duration;

/// 模拟服务的行为配置。所有比例都是 0～1 之间的小数。
#[derive(Debug, Clone, PartialEq)]
pub struct MockConfig {
    /// 成功响应前的延迟区间（毫秒），每次在区间内随机取
    pub latency_ms: (u64, u64),
    /// 每分钟允许的请求数，超过后返回 429；`None` 不限
    pub rate_limit_per_min: Option<u32>,
    /// 随机返回 429 的比例（和每分钟限额无关，模拟上游临时限流）
    pub error_429: f64,
    /// 随机返回 500 的比例
    pub error_500: f64,
    /// 随机挂起不响应的比例，用来测试客户端超时
    pub timeout: f64,
    /// 随机返回无法解析的响应体的比例
    pub bad_json: f64,
    /// 随机返回内容审核拒绝的比例
    pub content_rejected: f64,
    /// 模拟超时时挂起多久
    pub hang: Duration,
    /// 返回图片的边长（像素）。模拟服务不需要真实尺寸，小图省 CPU
    pub image_size: u32,
    /// 随机数种子，相同种子下错误注入的顺序可复现
    pub seed: u64,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            latency_ms: (0, 0),
            rate_limit_per_min: None,
            error_429: 0.0,
            error_500: 0.0,
            timeout: 0.0,
            bad_json: 0.0,
            content_rejected: 0.0,
            hang: Duration::from_secs(120),
            image_size: 64,
            seed: 42,
        }
    }
}

impl MockConfig {
    /// 检查比例是否在 0～1 之间、总和不超过 1，延迟区间是否合法。
    pub fn validate(&self) -> Result<(), String> {
        let rates = [
            ("error_429", self.error_429),
            ("error_500", self.error_500),
            ("timeout", self.timeout),
            ("bad_json", self.bad_json),
            ("content_rejected", self.content_rejected),
        ];
        for (name, rate) in rates {
            if !(0.0..=1.0).contains(&rate) {
                return Err(format!("{name} 必须在 0 到 1 之间，现在是 {rate}"));
            }
        }
        let total: f64 = rates.iter().map(|(_, rate)| rate).sum();
        if total > 1.0 {
            return Err(format!("各类错误比例加起来不能超过 1，现在是 {total}"));
        }
        if self.latency_ms.0 > self.latency_ms.1 {
            return Err("延迟区间的最小值不能大于最大值".into());
        }
        if self.image_size == 0 {
            return Err("图片边长必须大于 0".into());
        }
        Ok(())
    }
}

/// 解析 "2000-8000" 或 "500" 形式的延迟区间。
pub fn parse_latency(text: &str) -> Result<(u64, u64), String> {
    let parse = |part: &str| {
        part.trim()
            .parse::<u64>()
            .map_err(|_| format!("延迟要写成毫秒数，比如 2000-8000，现在是 {text}"))
    };
    match text.split_once('-') {
        Some((min, max)) => Ok((parse(min)?, parse(max)?)),
        None => {
            let value = parse(text)?;
            Ok((value, value))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        assert!(MockConfig::default().validate().is_ok());
    }

    #[test]
    fn rejects_rates_that_add_up_over_one() {
        let config = MockConfig {
            error_429: 0.6,
            error_500: 0.5,
            ..MockConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn parses_latency_ranges() {
        assert_eq!(parse_latency("2000-8000").unwrap(), (2000, 8000));
        assert_eq!(parse_latency("300").unwrap(), (300, 300));
        assert!(parse_latency("abc").is_err());
    }
}
