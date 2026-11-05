//! 按配置里的接口类型创建供应商。Key 由调用方提供（应用层从凭据管理器取，命令行从参数取）。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use prism_core::UpstreamErrorCode;
use prism_provider::{
    ApiKey, AsyncTaskProvider, AsyncTaskSpec, OpenAiChat, OpenAiImages, ProviderConfig,
    ProviderError,
};
use prism_store::ApiProfileRecord;

use crate::ProviderFactory;
use crate::profiles::ProviderHandle;

/// 用现成的 Key 创建供应商：先按配置 ID 找 Key，找不到用默认 Key。
#[derive(Debug, Clone, Default)]
pub struct KeyedFactory {
    keys: HashMap<String, ApiKey>,
    default_key: Option<ApiKey>,
    send_job_header: bool,
    async_spec: AsyncTaskSpec,
}

impl KeyedFactory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_key(mut self, profile_id: impl Into<String>, key: ApiKey) -> Self {
        self.keys.insert(profile_id.into(), key);
        self
    }

    pub fn with_default_key(mut self, key: ApiKey) -> Self {
        self.default_key = Some(key);
        self
    }

    /// 压测时开启，让模拟服务按任务统计重复生成。
    pub fn send_job_header(mut self, enabled: bool) -> Self {
        self.send_job_header = enabled;
        self
    }

    pub fn async_spec(mut self, spec: AsyncTaskSpec) -> Self {
        self.async_spec = spec;
        self
    }
}

impl ProviderFactory for KeyedFactory {
    fn build(&self, profile: &ApiProfileRecord) -> Result<ProviderHandle, ProviderError> {
        let key = self
            .keys
            .get(&profile.id)
            .or(self.default_key.as_ref())
            .cloned()
            .ok_or_else(|| {
                ProviderError::new(
                    UpstreamErrorCode::Auth,
                    format!("配置「{}」还没有填 API Key", profile.name),
                )
            })?;
        let mut config = ProviderConfig::new(profile.base_url.clone(), key, profile.model.clone());
        config.timeout = Duration::from_secs(u64::try_from(profile.timeout_secs).unwrap_or(600));
        config.send_job_header = self.send_job_header;

        match profile.provider_type.as_str() {
            "openai_images" => Ok(ProviderHandle::Image(Arc::new(OpenAiImages::new(config)?))),
            "async_task" => Ok(ProviderHandle::Image(Arc::new(AsyncTaskProvider::new(
                config,
                self.async_spec.clone(),
            )?))),
            "openai_chat" => Ok(ProviderHandle::Text(Arc::new(OpenAiChat::new(config)?))),
            other => Err(ProviderError::new(
                UpstreamErrorCode::BadRequest,
                format!("暂不支持接口类型 {other}"),
            )),
        }
    }
}
