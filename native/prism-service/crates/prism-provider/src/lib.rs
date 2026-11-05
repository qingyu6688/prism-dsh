//! 供应商适配：把统一的生图 / 文本请求翻译成上游接口，解析响应，把错误归类成
//! [`prism_core::UpstreamErrorCode`]。
//!
//! 不重试、不限流，这些由 `prism-engine` 负责。设计见 `docs/detail-design.md` 第 3 章。
//!
//! 日志里只记接口地址、状态码和耗时，不记 Key、图片数据和提示词全文。

mod async_task;
mod chat;
mod config;
mod error;
mod http;
mod image;
mod models;

pub use async_task::{AsyncTaskProvider, AsyncTaskSpec};
pub use chat::{
    ChatMessage, ChatRequest, ChatResponse, ContentPart, OpenAiChat, Role, TextProvider, ToolCall,
    ToolDefinition,
};
pub use config::{ApiKey, Capabilities, ProviderConfig};
pub use error::ProviderError;
pub use image::{
    GeneratedImage, ImageInput, ImageProvider, ImageRequest, OpenAiImages, PollOutcome,
    SubmitOutcome,
};
pub use models::{ModelList, is_image_model, list_models, sort_models};

/// 请求头：压测时让模拟服务按任务统计是否重复生成。只在配置开启时发送。
pub const JOB_ID_HEADER: &str = "X-Prism-Job-Id";
