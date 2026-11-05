//! 发请求、检查状态码、解析 JSON 的公共部分。

use std::time::Instant;

use reqwest::RequestBuilder;
use serde_json::Value;

use crate::error::{ProviderError, classify_status, classify_transport};
use crate::{JOB_ID_HEADER, ProviderConfig};

/// 按配置建 HTTP 客户端。每个供应商实例一个，复用连接池。
pub(crate) fn build_client(config: &ProviderConfig) -> Result<reqwest::Client, ProviderError> {
    reqwest::Client::builder()
        .timeout(config.timeout)
        .user_agent(concat!("Prism/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| {
            ProviderError::new(
                prism_core::UpstreamErrorCode::Network,
                format!("初始化网络客户端失败：{error}"),
            )
        })
}

/// 加上鉴权头和（压测时的）任务 ID 头。
pub(crate) fn authorize(
    request: RequestBuilder,
    config: &ProviderConfig,
    job_id: Option<&str>,
) -> RequestBuilder {
    let request = request.bearer_auth(config.api_key.expose());
    match (config.send_job_header, job_id) {
        (true, Some(job_id)) => request.header(JOB_ID_HEADER, job_id),
        _ => request,
    }
}

/// 发送请求并返回 JSON。非 2xx 和网络错误都转成 [`ProviderError`]。
pub(crate) async fn send_json(
    request: RequestBuilder,
    label: &str,
) -> Result<Value, ProviderError> {
    let bytes = send_bytes(request, label).await?;
    serde_json::from_slice(&bytes).map_err(ProviderError::bad_response)
}

/// 发送请求并返回原始响应体。
pub(crate) async fn send_bytes(
    request: RequestBuilder,
    label: &str,
) -> Result<Vec<u8>, ProviderError> {
    let started = Instant::now();
    let response = request
        .send()
        .await
        .map_err(|error| classify_transport(&error))?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| classify_transport(&error))?;
    tracing::debug!(
        target: "prism_provider",
        label,
        status = status.as_u16(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "上游请求完成"
    );
    if !status.is_success() {
        return Err(classify_status(
            status.as_u16(),
            &headers,
            &String::from_utf8_lossy(&bytes),
        ));
    }
    Ok(bytes.to_vec())
}
