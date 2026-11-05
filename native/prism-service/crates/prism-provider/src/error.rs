//! 把 HTTP 状态码、错误响应体和网络错误归类成 [`UpstreamErrorCode`]，对应 `docs/detail-design.md` 2.7。

use std::time::Duration;

use prism_core::UpstreamErrorCode;
use reqwest::header::HeaderMap;
use serde_json::Value;

/// 上游返回的错误信息最多保留多少字，避免把整页 HTML 错误页塞进数据库。
const MAX_UPSTREAM_MESSAGE_CHARS: usize = 200;

/// 调用上游失败。`message` 是给用户看的中文说明。
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct ProviderError {
    pub code: UpstreamErrorCode,
    pub message: String,
    /// HTTP 状态码，网络错误时为空
    pub status: Option<u16>,
    /// 上游建议的等待时间（`Retry-After` 或限额重置时间）
    pub retry_after: Option<Duration>,
}

impl ProviderError {
    pub fn new(code: UpstreamErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            status: None,
            retry_after: None,
        }
    }

    pub(crate) fn bad_response(detail: impl std::fmt::Display) -> Self {
        Self::new(
            UpstreamErrorCode::BadResponse,
            format!("接口返回的内容无法解析：{detail}"),
        )
    }
}

/// 按状态码和响应体归类。
pub(crate) fn classify_status(status: u16, headers: &HeaderMap, body: &str) -> ProviderError {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let error_field = |name: &str| {
        parsed
            .as_ref()
            .and_then(|v| v.pointer(&format!("/error/{name}")))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let error_code = error_field("code");
    let error_type = error_field("type");
    let upstream_message = shorten(&error_field("message"));
    let detail = if upstream_message.is_empty() {
        String::new()
    } else {
        format!("（上游说明：{upstream_message}）")
    };

    let (code, message) = match status {
        429 if is_quota_error(&error_code, &error_type) => (
            UpstreamErrorCode::QuotaExhausted,
            format!("接口额度或余额不足，请检查账户{detail}"),
        ),
        429 => (
            UpstreamErrorCode::RateLimited,
            format!("请求太频繁，被接口限流{detail}"),
        ),
        402 => (
            UpstreamErrorCode::QuotaExhausted,
            format!("接口余额不足，请充值后再试{detail}"),
        ),
        401 | 403 => (
            UpstreamErrorCode::Auth,
            format!("API Key 无效或没有权限（HTTP {status}）{detail}"),
        ),
        400 if is_content_rejection(&error_code, &error_type, &upstream_message) => (
            UpstreamErrorCode::ContentRejected,
            format!("内容被接口的安全审核拒绝{detail}"),
        ),
        408 | 504 => (
            UpstreamErrorCode::Timeout,
            format!("接口处理超时（HTTP {status}）"),
        ),
        500..=599 => (
            UpstreamErrorCode::UpstreamError,
            format!("接口服务出错（HTTP {status}）{detail}"),
        ),
        _ => (
            UpstreamErrorCode::BadRequest,
            format!("请求被接口拒绝（HTTP {status}）{detail}"),
        ),
    };

    ProviderError {
        code,
        message,
        status: Some(status),
        retry_after: retry_after(headers),
    }
}

/// 网络层错误：超时、连不上、读响应失败。
pub(crate) fn classify_transport(error: &reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::new(UpstreamErrorCode::Timeout, "请求超时，接口长时间没有响应")
    } else if error.is_connect() {
        ProviderError::new(
            UpstreamErrorCode::Network,
            "连不上接口地址，请检查网络和 Base URL",
        )
    } else if error.is_decode() || error.is_body() {
        ProviderError::bad_response("读取响应内容失败")
    } else {
        ProviderError::new(UpstreamErrorCode::Network, "网络请求失败，请检查网络")
    }
}

fn is_quota_error(code: &str, kind: &str) -> bool {
    matches!(code, "insufficient_quota" | "billing_hard_limit_reached")
        || kind == "insufficient_quota"
}

fn is_content_rejection(code: &str, kind: &str, message: &str) -> bool {
    matches!(
        code,
        "moderation_blocked" | "content_policy_violation" | "content_filter"
    ) || (kind == "image_generation_user_error" && message.contains("safety"))
        || message.contains("safety system")
}

/// 读等待时间：优先 `retry-after-ms`，其次 `retry-after`（秒），最后 `x-ratelimit-reset-requests`（如 `6m0s`）。
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if let Some(ms) = header("retry-after-ms").and_then(|v| v.trim().parse::<f64>().ok()) {
        return duration_from_secs(ms / 1000.0);
    }
    if let Some(secs) = header("retry-after").and_then(|v| v.trim().parse::<f64>().ok()) {
        return duration_from_secs(secs);
    }
    header("x-ratelimit-reset-requests").and_then(parse_go_duration)
}

fn duration_from_secs(secs: f64) -> Option<Duration> {
    (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs))
}

/// 解析 `1s`、`20ms`、`6m0s`、`1h2m3.5s` 这类时长写法。
fn parse_go_duration(text: &str) -> Option<Duration> {
    let mut total = 0.0;
    let mut number = String::new();
    let mut chars = text.trim().chars().peekable();
    let mut parsed_any = false;
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            number.push(c);
            continue;
        }
        let value: f64 = number.parse().ok()?;
        number.clear();
        let unit_secs = match c {
            'h' => 3600.0,
            'm' if chars.peek() == Some(&'s') => {
                chars.next();
                0.001
            }
            'm' => 60.0,
            's' => 1.0,
            _ => return None,
        };
        total += value * unit_secs;
        parsed_any = true;
    }
    if !number.is_empty() || !parsed_any {
        return None;
    }
    duration_from_secs(total)
}

fn shorten(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX_UPSTREAM_MESSAGE_CHARS {
        return trimmed.to_owned();
    }
    let mut short: String = trimmed.chars().take(MAX_UPSTREAM_MESSAGE_CHARS).collect();
    short.push('…');
    short
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    fn classify(status: u16, body: &str) -> UpstreamErrorCode {
        classify_status(status, &HeaderMap::new(), body).code
    }

    #[test]
    fn maps_status_codes() {
        // UT-P01
        assert_eq!(classify(429, "{}"), UpstreamErrorCode::RateLimited);
        assert_eq!(classify(500, ""), UpstreamErrorCode::UpstreamError);
        assert_eq!(classify(502, "<html>"), UpstreamErrorCode::UpstreamError);
        assert_eq!(classify(400, "{}"), UpstreamErrorCode::BadRequest);
        assert_eq!(classify(401, "{}"), UpstreamErrorCode::Auth);
        assert_eq!(classify(403, "{}"), UpstreamErrorCode::Auth);
        assert_eq!(classify(504, ""), UpstreamErrorCode::Timeout);
        assert_eq!(classify(402, "{}"), UpstreamErrorCode::QuotaExhausted);
    }

    #[test]
    fn detects_content_rejection() {
        // UT-P02
        let moderation = r#"{"error":{"code":"moderation_blocked","message":"rejected"}}"#;
        assert_eq!(
            classify(400, moderation),
            UpstreamErrorCode::ContentRejected
        );
        let safety = r#"{"error":{"message":"Your request was rejected as a result of our safety system."}}"#;
        assert_eq!(classify(400, safety), UpstreamErrorCode::ContentRejected);
    }

    #[test]
    fn distinguishes_quota_from_rate_limit() {
        let quota = r#"{"error":{"code":"insufficient_quota","message":"You exceeded your current quota"}}"#;
        assert_eq!(classify(429, quota), UpstreamErrorCode::QuotaExhausted);
    }

    #[test]
    fn reads_retry_after_headers() {
        let error = classify_status(429, &headers(&[("retry-after", "7")]), "{}");
        assert_eq!(error.retry_after, Some(Duration::from_secs(7)));
        let error = classify_status(429, &headers(&[("retry-after-ms", "1500")]), "{}");
        assert_eq!(error.retry_after, Some(Duration::from_millis(1500)));
        let error = classify_status(
            429,
            &headers(&[("x-ratelimit-reset-requests", "6m0s")]),
            "{}",
        );
        assert_eq!(error.retry_after, Some(Duration::from_secs(360)));
    }

    #[test]
    fn parses_go_style_durations() {
        assert_eq!(parse_go_duration("20ms"), Some(Duration::from_millis(20)));
        assert_eq!(parse_go_duration("1h2m3s"), Some(Duration::from_secs(3723)));
        assert_eq!(parse_go_duration("1.5s"), Some(Duration::from_millis(1500)));
        assert_eq!(parse_go_duration("soon"), None);
        assert_eq!(parse_go_duration("12"), None);
    }

    #[test]
    fn keeps_upstream_message_short() {
        let long = "x".repeat(1000);
        let body = format!(r#"{{"error":{{"message":"{long}"}}}}"#);
        let error = classify_status(500, &HeaderMap::new(), &body);
        assert!(error.message.chars().count() < 260);
    }
}
