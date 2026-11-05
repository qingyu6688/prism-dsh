//! 一个 API 配置在适配层需要的信息。

use std::fmt;
use std::time::Duration;

/// API Key。`Debug` 输出只显示首尾几位，不会被意外打进日志。
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// 取出原文，只在拼请求头时用。
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// 打码后的形式，比如 `sk-…f3a9`，可以显示在界面和日志里。
    pub fn masked(&self) -> String {
        let chars: Vec<char> = self.0.chars().collect();
        if chars.len() <= 8 {
            return "***".into();
        }
        let head: String = chars[..3].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}…{tail}")
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ApiKey({})", self.masked())
    }
}

/// 上游支持哪些参数。不支持的参数不发，避免中转站因为不认识字段直接报错。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    /// 一次请求出多张（`n`）
    pub supports_n: bool,
    /// 透明背景（`background`）
    pub supports_background: bool,
    /// 输出格式（`output_format`）
    pub supports_output_format: bool,
    /// 质量档（`quality`）
    pub supports_quality: bool,
    /// JSON 输出（`response_format: json_object`）
    pub supports_json_mode: bool,
    /// 一次最多带几张参考图
    pub max_reference_images: usize,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            supports_n: true,
            supports_background: true,
            supports_output_format: true,
            supports_quality: true,
            supports_json_mode: true,
            max_reference_images: 16,
        }
    }
}

/// 适配层用的配置，由上层从 `api_profile` 表和凭据管理器组装。
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// 接口地址，包含版本路径，比如 `https://api.openai.com/v1`
    pub base_url: String,
    pub api_key: ApiKey,
    pub model: String,
    pub timeout: Duration,
    /// 是否发送 `X-Prism-Job-Id` 请求头，只在压测时开启
    pub send_job_header: bool,
    pub capabilities: Capabilities,
}

impl ProviderConfig {
    pub fn new(base_url: impl Into<String>, api_key: ApiKey, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key,
            model: model.into(),
            timeout: Duration::from_secs(prism_core::defaults::DEFAULT_REQUEST_TIMEOUT_SECS),
            send_job_header: false,
            capabilities: Capabilities::default(),
        }
    }

    /// 拼接口地址：`base_url` 末尾的 `/` 有没有都可以。
    pub fn endpoint(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_never_contains_the_key() {
        let key = ApiKey::new("sk-proj-1234567890abcdef");
        let config =
            ProviderConfig::new("https://api.openai.com/v1", key, "gpt-image-2.5-sunburst");
        let debug = format!("{config:?}");
        assert!(!debug.contains("1234567890abcdef"));
        assert!(debug.contains("sk-…cdef"));
    }

    #[test]
    fn short_keys_are_fully_masked() {
        assert_eq!(ApiKey::new("abc").masked(), "***");
    }

    #[test]
    fn endpoint_handles_trailing_slashes() {
        let mut config = ProviderConfig::new("https://x.com/v1/", ApiKey::new("k"), "m");
        assert_eq!(config.endpoint("/models"), "https://x.com/v1/models");
        config.base_url = "https://x.com/v1".into();
        assert_eq!(
            config.endpoint("images/edits"),
            "https://x.com/v1/images/edits"
        );
    }
}
