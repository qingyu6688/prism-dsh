//! 生图供应商。

use std::fmt;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use prism_core::UpstreamErrorCode;
use reqwest::multipart::{Form, Part};
use serde_json::{Map, Value, json};

use crate::http::{authorize, build_client, send_bytes, send_json};
use crate::{Capabilities, ProviderConfig, ProviderError};

/// 一张参考图。`Debug` 只显示大小，不打印图片数据。
#[derive(Clone, PartialEq, Eq)]
pub struct ImageInput {
    pub bytes: Vec<u8>,
    /// 如 `image/png`
    pub mime: String,
    pub file_name: String,
}

impl fmt::Debug for ImageInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageInput")
            .field("file_name", &self.file_name)
            .field("mime", &self.mime)
            .field("bytes", &format_args!("{} 字节", self.bytes.len()))
            .finish()
    }
}

/// 一次生图请求。有参考图时走编辑接口，没有时走生成接口。
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRequest {
    pub prompt: String,
    pub references: Vec<ImageInput>,
    /// 如 `1024x1024`，`None` 用接口默认值
    pub size: Option<String>,
    /// `low` / `medium` / `high` / `auto`
    pub quality: Option<String>,
    /// 一次出几张，通常为 1
    pub n: u32,
    /// `transparent` / `opaque` / `auto`
    pub background: Option<String>,
    /// `png` / `webp` / `jpeg`
    pub output_format: Option<String>,
    /// 引擎的任务 ID，压测时放进请求头
    pub job_id: Option<String>,
}

impl ImageRequest {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            references: Vec::new(),
            size: None,
            quality: None,
            n: 1,
            background: None,
            output_format: None,
            job_id: None,
        }
    }
}

/// 生成好的一张图。`Debug` 只显示大小。
#[derive(Clone, PartialEq, Eq)]
pub struct GeneratedImage {
    pub bytes: Vec<u8>,
    /// 模型改写后的提示词，接口返回了才有
    pub revised_prompt: Option<String>,
}

impl fmt::Debug for GeneratedImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeneratedImage")
            .field("bytes", &format_args!("{} 字节", self.bytes.len()))
            .field("revised_prompt", &self.revised_prompt)
            .finish()
    }
}

/// 提交结果：同步接口直接给图，异步接口给远端任务 ID。
#[derive(Debug, Clone, PartialEq)]
pub enum SubmitOutcome {
    Done(Vec<GeneratedImage>),
    Pending { remote_task_id: String },
}

/// 轮询结果。
#[derive(Debug, Clone, PartialEq)]
pub enum PollOutcome {
    Pending,
    Done(Vec<GeneratedImage>),
}

/// 生图供应商。同步和异步接口都实现这个 trait，引擎不需要区分。
#[async_trait]
pub trait ImageProvider: Send + Sync {
    /// 提交一次生图。
    async fn submit(&self, request: &ImageRequest) -> Result<SubmitOutcome, ProviderError>;
    /// 查询异步任务。同步供应商不会被调用。
    async fn poll(&self, remote_task_id: &str) -> Result<PollOutcome, ProviderError>;
    /// 声明支持哪些参数。
    fn capabilities(&self) -> &Capabilities;
}

/// OpenAI Images 接口（同步）：有参考图走 `/images/edits`，没有走 `/images/generations`。
#[derive(Debug, Clone)]
pub struct OpenAiImages {
    config: ProviderConfig,
    client: reqwest::Client,
}

impl OpenAiImages {
    pub fn new(config: ProviderConfig) -> Result<Self, ProviderError> {
        let client = build_client(&config)?;
        Ok(Self { config, client })
    }

    async fn generate(&self, request: &ImageRequest) -> Result<Value, ProviderError> {
        let caps = &self.config.capabilities;
        let mut body = Map::new();
        body.insert("model".into(), json!(self.config.model));
        body.insert("prompt".into(), json!(request.prompt));
        if caps.supports_n {
            body.insert("n".into(), json!(request.n));
        }
        for (key, value, supported) in self.optional_fields(request) {
            if supported && let Some(value) = value {
                body.insert(key.into(), json!(value));
            }
        }
        let http = self
            .client
            .post(self.config.endpoint("images/generations"))
            .json(&Value::Object(body));
        send_json(
            authorize(http, &self.config, request.job_id.as_deref()),
            "images/generations",
        )
        .await
    }

    async fn edit(&self, request: &ImageRequest) -> Result<Value, ProviderError> {
        let caps = &self.config.capabilities;
        if request.references.len() > caps.max_reference_images {
            return Err(ProviderError::new(
                UpstreamErrorCode::BadRequest,
                format!(
                    "参考图有 {} 张，这个接口最多支持 {} 张",
                    request.references.len(),
                    caps.max_reference_images
                ),
            ));
        }
        let mut form = Form::new()
            .text("model", self.config.model.clone())
            .text("prompt", request.prompt.clone());
        if caps.supports_n {
            form = form.text("n", request.n.to_string());
        }
        for (key, value, supported) in self.optional_fields(request) {
            if supported && let Some(value) = value {
                form = form.text(key, value.to_owned());
            }
        }
        for image in &request.references {
            let part = Part::bytes(image.bytes.clone())
                .file_name(image.file_name.clone())
                .mime_str(&image.mime)
                .map_err(|_| {
                    ProviderError::new(
                        UpstreamErrorCode::BadRequest,
                        format!("参考图类型 {} 不正确", image.mime),
                    )
                })?;
            form = form.part("image[]", part);
        }
        let http = self
            .client
            .post(self.config.endpoint("images/edits"))
            .multipart(form);
        send_json(
            authorize(http, &self.config, request.job_id.as_deref()),
            "images/edits",
        )
        .await
    }

    /// 可选参数和是否支持，不支持的不发（UT-P05）。
    fn optional_fields<'a>(
        &self,
        request: &'a ImageRequest,
    ) -> [(&'static str, Option<&'a str>, bool); 4] {
        let caps = &self.config.capabilities;
        [
            ("size", request.size.as_deref(), true),
            ("quality", request.quality.as_deref(), caps.supports_quality),
            (
                "background",
                request.background.as_deref(),
                caps.supports_background,
            ),
            (
                "output_format",
                request.output_format.as_deref(),
                caps.supports_output_format,
            ),
        ]
    }

    /// 解析 `data[]`：每项是 `b64_json` 或 `url`，`url` 会下载下来。
    async fn parse_images(&self, body: &Value) -> Result<Vec<GeneratedImage>, ProviderError> {
        let items = body
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::bad_response("缺少 data 字段"))?;
        if items.is_empty() {
            return Err(ProviderError::bad_response("data 为空，没有返回图片"));
        }
        let mut images = Vec::with_capacity(items.len());
        for item in items {
            let revised_prompt = item
                .get("revised_prompt")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let bytes = if let Some(b64) = item.get("b64_json").and_then(Value::as_str) {
                BASE64
                    .decode(b64.trim())
                    .map_err(|_| ProviderError::bad_response("b64_json 不是合法的 base64"))?
            } else if let Some(url) = item.get("url").and_then(Value::as_str) {
                send_bytes(self.client.get(url), "image download").await?
            } else {
                return Err(ProviderError::bad_response(
                    "图片既没有 b64_json 也没有 url",
                ));
            };
            images.push(GeneratedImage {
                bytes,
                revised_prompt,
            });
        }
        Ok(images)
    }
}

#[async_trait]
impl ImageProvider for OpenAiImages {
    async fn submit(&self, request: &ImageRequest) -> Result<SubmitOutcome, ProviderError> {
        let body = if request.references.is_empty() {
            self.generate(request).await?
        } else {
            self.edit(request).await?
        };
        Ok(SubmitOutcome::Done(self.parse_images(&body).await?))
    }

    async fn poll(&self, _remote_task_id: &str) -> Result<PollOutcome, ProviderError> {
        Err(ProviderError::new(
            UpstreamErrorCode::BadRequest,
            "同步接口不支持轮询",
        ))
    }

    fn capabilities(&self) -> &Capabilities {
        &self.config.capabilities
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_hides_image_bytes() {
        let input = ImageInput {
            bytes: vec![7; 2048],
            mime: "image/png".into(),
            file_name: "ref.png".into(),
        };
        let debug = format!("{input:?}");
        assert!(debug.contains("2048 字节"));
        assert!(!debug.contains("7, 7"));
    }
}
