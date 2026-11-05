//! 错误码。
//!
//! 分两类：
//! - [`UpstreamErrorCode`]：调用上游接口失败时，供应商层归类出的错误，引擎只看它决定怎么重试
//! - [`ApiError`]：Tauri 命令返回给前端的统一错误对象

use serde::{Deserialize, Serialize};

/// 上游接口错误的分类，存进 `job.error_code`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamErrorCode {
    /// 429
    RateLimited,
    /// 5xx
    UpstreamError,
    Timeout,
    /// 连接失败、断网
    Network,
    /// 内容审核拒绝
    ContentRejected,
    /// 其他 400，通常是参数问题
    BadRequest,
    /// 401 / 403
    Auth,
    /// 余额不足
    QuotaExhausted,
    /// 响应格式解析失败
    BadResponse,
}

/// 引擎遇到某类错误时的处理方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPolicy {
    /// 降速后重新排队，不计入尝试次数
    Requeue,
    /// 按退避时间重试，计入尝试次数
    Retry,
    /// 只再试一次
    RetryOnce,
    /// 让编排层改写指令后再试一次
    RewriteOnce,
    /// 直接失败
    Fail,
    /// 停用当前配置，任务转给其他配置
    DisableProfile,
    /// 暂停当前配置，等用户处理
    PauseProfile,
}

impl UpstreamErrorCode {
    /// 数据库里存的字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RateLimited => "rate_limited",
            Self::UpstreamError => "upstream_error",
            Self::Timeout => "timeout",
            Self::Network => "network",
            Self::ContentRejected => "content_rejected",
            Self::BadRequest => "bad_request",
            Self::Auth => "auth",
            Self::QuotaExhausted => "quota_exhausted",
            Self::BadResponse => "bad_response",
        }
    }

    /// 对应 `docs/detail-design.md` 2.7 的处理表。
    pub fn retry_policy(self) -> RetryPolicy {
        match self {
            Self::RateLimited => RetryPolicy::Requeue,
            Self::UpstreamError | Self::Timeout | Self::Network => RetryPolicy::Retry,
            Self::BadResponse => RetryPolicy::RetryOnce,
            Self::ContentRejected => RetryPolicy::RewriteOnce,
            Self::BadRequest => RetryPolicy::Fail,
            Self::Auth => RetryPolicy::DisableProfile,
            Self::QuotaExhausted => RetryPolicy::PauseProfile,
        }
    }

    /// 是否计入熔断的连续失败次数。限流和业务性错误不算上游"坏了"。
    pub fn counts_toward_circuit_breaker(self) -> bool {
        matches!(self, Self::UpstreamError | Self::Timeout | Self::Network)
    }
}

/// 返回给前端的错误类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    InvalidInput,
    NotFound,
    NoProfile,
    LimitExceeded,
    Storage,
    Keyring,
    Upstream,
    Internal,
}

/// Tauri 命令的统一错误对象，序列化后形如 `{"code":"invalid_input","message":"…","detail":null}`。
///
/// `message` 直接展示给用户，要写成能看懂的话；原始错误只进日志，不放进 `detail`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct ApiError {
    pub code: ApiErrorCode,
    pub message: String,
    pub detail: Option<serde_json::Value>,
}

impl ApiError {
    pub fn new(code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::InvalidInput, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::Internal, message)
    }
}

impl_db_enum!(
    UpstreamErrorCode,
    "错误码",
    [
        RateLimited,
        UpstreamError,
        Timeout,
        Network,
        ContentRejected,
        BadRequest,
        Auth,
        QuotaExhausted,
        BadResponse,
    ]
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_codes_parse_back() {
        for code in UpstreamErrorCode::ALL {
            assert_eq!(code.as_str().parse::<UpstreamErrorCode>().unwrap(), *code);
        }
    }

    #[test]
    fn rate_limit_does_not_consume_attempts() {
        assert_eq!(
            UpstreamErrorCode::RateLimited.retry_policy(),
            RetryPolicy::Requeue
        );
        assert!(!UpstreamErrorCode::RateLimited.counts_toward_circuit_breaker());
    }

    #[test]
    fn auth_errors_disable_profile() {
        assert_eq!(
            UpstreamErrorCode::Auth.retry_policy(),
            RetryPolicy::DisableProfile
        );
    }

    #[test]
    fn server_side_failures_trip_breaker() {
        for code in [
            UpstreamErrorCode::UpstreamError,
            UpstreamErrorCode::Timeout,
            UpstreamErrorCode::Network,
        ] {
            assert_eq!(code.retry_policy(), RetryPolicy::Retry);
            assert!(code.counts_toward_circuit_breaker());
        }
    }

    #[test]
    fn api_error_serializes_to_agreed_shape() {
        let err = ApiError::invalid_input("一次最多上传 50 张图片");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["code"], "invalid_input");
        assert_eq!(json["message"], "一次最多上传 50 张图片");
        assert!(json["detail"].is_null());
        assert_eq!(err.to_string(), "一次最多上传 50 张图片");
    }

    #[test]
    fn upstream_code_names_match_database_values() {
        let json = serde_json::to_string(&UpstreamErrorCode::ContentRejected).unwrap();
        assert_eq!(
            json,
            format!("\"{}\"", UpstreamErrorCode::ContentRejected.as_str())
        );
    }
}
