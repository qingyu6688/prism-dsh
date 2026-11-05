//! 引擎和外部的约定：任务处理器、供应商工厂、事件。

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use prism_core::{BatchStatus, JobStatus};
use prism_provider::{ChatRequest, ChatResponse, GeneratedImage, ImageRequest, ProviderError};
use prism_store::{ApiProfileRecord, JobRecord};
use serde::Serialize;

use crate::profiles::ProviderHandle;

/// 任务要用哪类配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Purpose {
    Image,
    Text,
}

/// 处理器为一个任务构造的请求。
#[derive(Debug, Clone)]
pub enum JobRequest {
    Image(ImageRequest),
    Text(ChatRequest),
}

/// 上游返回的结果，交给处理器入库。
#[derive(Debug, Clone)]
pub enum JobOutput {
    Images(Vec<GeneratedImage>),
    Text(ChatResponse),
}

/// 处理器出错。`retryable` 为 true 时按"再试一次"处理（比如模型返回的 JSON 格式不对）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct HandlerError {
    pub message: String,
    pub retryable: bool,
}

impl HandlerError {
    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }

    pub fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }
}

/// 审核拒绝后的改写结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    pub instruction: String,
    pub request_params: Option<String>,
}

/// 任务处理器：引擎不知道任务的业务含义，由处理器负责"发什么请求"和"结果怎么入库"。
///
/// `on_output` 必须用存储层的 `succeed_job` / `succeed_plan_job` 把任务标记为成功。
#[async_trait]
pub trait JobHandler: Send + Sync {
    fn purpose(&self, job: &JobRecord) -> Purpose;
    async fn build_request(&self, job: &JobRecord) -> Result<JobRequest, HandlerError>;
    async fn on_output(&self, job: &JobRecord, output: JobOutput) -> Result<(), HandlerError>;
    /// 审核拒绝后改写指令。返回 `None` 表示不改写，任务直接失败。
    async fn rewrite_after_rejection(
        &self,
        _job: &JobRecord,
    ) -> Result<Option<Rewrite>, HandlerError> {
        Ok(None)
    }
}

/// 把数据库里的 API 配置变成可调用的供应商。由应用层实现（要从凭据管理器取 Key）。
pub trait ProviderFactory: Send + Sync {
    fn build(&self, profile: &ApiProfileRecord) -> Result<ProviderHandle, ProviderError>;
}

/// 一个任务的状态变化。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobChange {
    pub job_id: String,
    pub status: JobStatus,
    pub error_code: Option<String>,
}

/// 引擎推给界面的事件。进度事件按批次合并后定时推送，其他事件立即推送。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EngineEvent {
    #[serde(rename_all = "camelCase")]
    Progress {
        batch_id: String,
        changes: Vec<JobChange>,
    },
    #[serde(rename_all = "camelCase")]
    BatchFinished {
        batch_id: String,
        status: BatchStatus,
    },
    #[serde(rename_all = "camelCase")]
    BatchPaused { batch_id: String, reason: String },
    #[serde(rename_all = "camelCase")]
    ProfileDisabled {
        profile_id: String,
        reason: String,
        message: String,
    },
    #[serde(rename_all = "camelCase")]
    Recovered {
        resubmitted: u64,
        resumed_polling: u64,
    },
}

/// 事件出口，应用层用 Tauri 事件实现，测试里收集到列表。
pub trait EventSink: Send + Sync {
    fn emit(&self, event: EngineEvent);
}

/// 进度缓冲：把一段时间内的变化按批次攒起来，一次推送。
#[derive(Debug, Default)]
pub(crate) struct ProgressBuffer {
    pending: Mutex<HashMap<String, Vec<JobChange>>>,
}

impl ProgressBuffer {
    pub fn push(&self, batch_id: &str, change: JobChange) {
        crate::lock(&self.pending)
            .entry(batch_id.to_owned())
            .or_default()
            .push(change);
    }

    pub fn drain(&self) -> Vec<EngineEvent> {
        crate::lock(&self.pending)
            .drain()
            .map(|(batch_id, changes)| EngineEvent::Progress { batch_id, changes })
            .collect()
    }
}
