//! 异步任务型供应商：提交后拿到远端任务 ID，再轮询结果。sub2api 一类的接口都是这种形式。
//!
//! 请求和响应的字段位置由 [`AsyncTaskSpec`] 描述，默认值对应模拟服务的格式。
//! 自定义 JSON 供应商（`docs/api.md` 4.3）以后也会转换成这个结构。

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use prism_core::UpstreamErrorCode;
use serde_json::{Map, Value, json};

use crate::http::{authorize, build_client, send_bytes, send_json};
use crate::{
    Capabilities, GeneratedImage, ImageProvider, ImageRequest, PollOutcome, ProviderConfig,
    ProviderError, SubmitOutcome,
};

/// 异步接口的字段位置。指针用 JSON Pointer 写法（`/data/0/task_id`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsyncTaskSpec {
    /// 提交地址，相对 Base URL
    pub submit_path: String,
    /// 提交响应里任务 ID 的位置
    pub task_id_pointer: String,
    /// 查询地址，`{task_id}` 会被替换
    pub poll_path: String,
    /// 查询响应里状态的位置
    pub status_pointer: String,
    pub success_values: Vec<String>,
    pub failure_values: Vec<String>,
    /// 失败时错误说明的位置
    pub error_pointer: String,
    /// 结果图片数组的位置，每项含 `b64_json` 或 `url`
    pub images_pointer: String,
}

impl Default for AsyncTaskSpec {
    fn default() -> Self {
        Self {
            submit_path: "async/images".into(),
            task_id_pointer: "/data/0/task_id".into(),
            poll_path: "async/tasks/{task_id}".into(),
            status_pointer: "/data/status".into(),
            success_values: vec!["completed".into(), "succeeded".into()],
            failure_values: vec!["failed".into(), "cancelled".into()],
            error_pointer: "/data/error/message".into(),
            images_pointer: "/data/result/images".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AsyncTaskProvider {
    config: ProviderConfig,
    spec: AsyncTaskSpec,
    client: reqwest::Client,
}

impl AsyncTaskProvider {
    pub fn new(config: ProviderConfig, spec: AsyncTaskSpec) -> Result<Self, ProviderError> {
        let client = build_client(&config)?;
        Ok(Self {
            config,
            spec,
            client,
        })
    }

    fn submit_body(&self, request: &ImageRequest) -> Value {
        let caps = &self.config.capabilities;
        let mut body = Map::new();
        body.insert("model".into(), json!(self.config.model));
        body.insert("prompt".into(), json!(request.prompt));
        if caps.supports_n {
            body.insert("n".into(), json!(request.n));
        }
        if let Some(size) = &request.size {
            body.insert("size".into(), json!(size));
        }
        if caps.supports_quality
            && let Some(quality) = &request.quality
        {
            body.insert("quality".into(), json!(quality));
        }
        if caps.supports_background
            && let Some(background) = &request.background
        {
            body.insert("background".into(), json!(background));
        }
        if caps.supports_output_format
            && let Some(format) = &request.output_format
        {
            body.insert("output_format".into(), json!(format));
        }
        if !request.references.is_empty() {
            let urls: Vec<Value> = request
                .references
                .iter()
                .map(|image| {
                    json!(format!(
                        "data:{};base64,{}",
                        image.mime,
                        BASE64.encode(&image.bytes)
                    ))
                })
                .collect();
            body.insert("image_urls".into(), Value::Array(urls));
        }
        Value::Object(body)
    }

    async fn read_images(&self, items: &[Value]) -> Result<Vec<GeneratedImage>, ProviderError> {
        if items.is_empty() {
            return Err(ProviderError::bad_response("任务完成了但没有返回图片"));
        }
        let mut images = Vec::with_capacity(items.len());
        for item in items {
            let bytes = if let Some(b64) = item.get("b64_json").and_then(Value::as_str) {
                BASE64
                    .decode(b64.trim())
                    .map_err(|_| ProviderError::bad_response("b64_json 不是合法的 base64"))?
            } else if let Some(url) = item.get("url").and_then(Value::as_str) {
                send_bytes(self.client.get(url), "async image download").await?
            } else {
                return Err(ProviderError::bad_response(
                    "图片既没有 b64_json 也没有 url",
                ));
            };
            images.push(GeneratedImage {
                bytes,
                revised_prompt: item
                    .get("revised_prompt")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
        Ok(images)
    }
}

#[async_trait]
impl ImageProvider for AsyncTaskProvider {
    async fn submit(&self, request: &ImageRequest) -> Result<SubmitOutcome, ProviderError> {
        let http = self
            .client
            .post(self.config.endpoint(&self.spec.submit_path))
            .json(&self.submit_body(request));
        let body = send_json(
            authorize(http, &self.config, request.job_id.as_deref()),
            "async submit",
        )
        .await?;
        let task_id = body
            .pointer(&self.spec.task_id_pointer)
            .and_then(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .or_else(|| v.as_u64().map(|n| n.to_string()))
            })
            .ok_or_else(|| ProviderError::bad_response("提交响应里找不到任务 ID"))?;
        Ok(SubmitOutcome::Pending {
            remote_task_id: task_id,
        })
    }

    async fn poll(&self, remote_task_id: &str) -> Result<PollOutcome, ProviderError> {
        let path = self.spec.poll_path.replace("{task_id}", remote_task_id);
        let http = self.client.get(self.config.endpoint(&path));
        let body = send_json(authorize(http, &self.config, None), "async poll").await?;
        let status = body
            .pointer(&self.spec.status_pointer)
            .and_then(Value::as_str)
            .unwrap_or_default();
        if self.spec.success_values.iter().any(|v| v == status) {
            let items = body
                .pointer(&self.spec.images_pointer)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            return Ok(PollOutcome::Done(self.read_images(&items).await?));
        }
        if self.spec.failure_values.iter().any(|v| v == status) {
            let reason = body
                .pointer(&self.spec.error_pointer)
                .and_then(Value::as_str)
                .unwrap_or("没有给出原因");
            return Err(ProviderError::new(
                UpstreamErrorCode::UpstreamError,
                format!("远端任务失败：{reason}"),
            ));
        }
        Ok(PollOutcome::Pending)
    }

    fn capabilities(&self) -> &Capabilities {
        &self.config.capabilities
    }
}
