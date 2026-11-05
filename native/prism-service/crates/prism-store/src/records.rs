//! 表记录。字段和 `docs/database.md` 一一对应；JSON 字段保持字符串，由上层解析。

use prism_core::{BatchStatus, JobKind, JobStatus};

/// `api_profile` 表的一行。Key 不在这里，存在系统凭据管理器里。
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ApiProfileRecord {
    pub id: String,
    pub name: String,
    pub purpose: String,
    pub access_type: String,
    pub provider_type: String,
    pub base_url: String,
    pub model: String,
    pub model_is_custom: bool,
    pub models_cache: Option<String>,
    pub models_fetched_at: Option<i64>,
    pub max_concurrency: i64,
    pub rpm_limit: Option<i64>,
    pub weight: i64,
    pub timeout_secs: i64,
    pub transparent_method: String,
    pub capabilities: Option<String>,
    pub custom_spec: Option<String>,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub credential_owner_id: Option<String>,
}

impl ApiProfileRecord {
    /// 共享配置只在主配置下保存一次 Key。
    pub fn credential_id(&self) -> &str {
        self.credential_owner_id.as_deref().unwrap_or(&self.id)
    }
}

/// 新建或整体更新一个 API 配置时需要的字段。
#[derive(Debug, Clone, PartialEq)]
pub struct NewApiProfile {
    pub name: String,
    pub purpose: String,
    pub access_type: String,
    pub provider_type: String,
    pub base_url: String,
    pub model: String,
    pub model_is_custom: bool,
    pub max_concurrency: i64,
    pub rpm_limit: Option<i64>,
    pub weight: i64,
    pub timeout_secs: i64,
    pub transparent_method: String,
    pub capabilities: Option<String>,
    pub custom_spec: Option<String>,
}

/// `batch` 表的一行。
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct BatchRecord {
    pub id: String,
    pub seq: i64,
    pub conversation_id: Option<String>,
    pub mode: String,
    pub variants_per_source: i64,
    pub params: String,
    #[sqlx(try_from = "String")]
    pub status: BatchStatus,
    pub pause_reason: Option<String>,
    pub total_jobs: i64,
    pub succeeded_jobs: i64,
    pub failed_jobs: i64,
    pub resubmitted_jobs: i64,
    pub finished_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    /// 所属模块：`image` / `listing` / `analytics` / `research`
    pub module: String,
    pub product_id: Option<String>,
}

/// 新建批次需要的字段。序号由数据库分配。
#[derive(Debug, Clone, PartialEq)]
pub struct NewBatch {
    /// 所属模块：`image` / `listing` / `analytics` / `research`
    pub module: String,
    pub product_id: Option<String>,
    pub conversation_id: Option<String>,
    pub mode: String,
    pub variants_per_source: i64,
    /// JSON：强度、锁定项、补充要求、输出参数等
    pub params: String,
}

/// `source_image` 表的一行。
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct SourceImageRecord {
    pub id: String,
    pub batch_id: String,
    pub seq: i64,
    pub ref_no: Option<String>,
    pub parent_image_id: Option<String>,
    pub file_path: String,
    pub sha256: String,
    pub width: i64,
    pub height: i64,
    pub phash: Option<String>,
    pub analysis: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 新增原图需要的字段。
#[derive(Debug, Clone, PartialEq)]
pub struct NewSourceImage {
    pub batch_id: String,
    pub seq: i64,
    pub ref_no: Option<String>,
    pub parent_image_id: Option<String>,
    pub file_path: String,
    pub sha256: String,
    pub width: i64,
    pub height: i64,
    pub phash: Option<String>,
}

/// `job` 表的一行。
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct JobRecord {
    pub id: String,
    pub batch_id: String,
    pub batch_seq: i64,
    /// 生图类任务的原图；文案等任务没有原图
    pub source_id: Option<String>,
    pub source_seq: i64,
    #[sqlx(try_from = "String")]
    pub kind: JobKind,
    /// 具体做什么，如 `image.fission`、`listing.copy`；引擎不看它，交给处理器分派
    pub task_type: String,
    pub product_id: Option<String>,
    pub variant_index: i64,
    pub axis: Option<String>,
    pub instruction: Option<String>,
    pub rewritten: bool,
    pub priority: i64,
    #[sqlx(try_from = "String")]
    pub status: JobStatus,
    pub attempt: i64,
    pub max_attempts: i64,
    pub next_run_at: Option<i64>,
    pub profile_id: Option<String>,
    pub remote_task_id: Option<String>,
    pub request_params: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 新增任务需要的字段。`batch_seq`、`source_seq` 由存储层从批次和原图上读，不用调用方传。
#[derive(Debug, Clone, PartialEq)]
pub struct NewJob {
    pub batch_id: String,
    /// 生图类任务的原图；文案等任务为空
    pub source_id: Option<String>,
    pub kind: JobKind,
    pub task_type: String,
    pub product_id: Option<String>,
    pub variant_index: i64,
    pub axis: Option<String>,
    pub instruction: Option<String>,
    pub priority: i64,
    pub max_attempts: i64,
    pub request_params: Option<String>,
}

/// 生图任务成功后写入的一张结果图。
#[derive(Debug, Clone, PartialEq)]
pub struct NewImage {
    pub variant_index: i64,
    pub ref_no: Option<String>,
    pub file_path: String,
    pub thumb_path: Option<String>,
    pub format: String,
    pub width: i64,
    pub height: i64,
    pub sha256: String,
    pub phash: Option<String>,
    pub dist_to_source: Option<i64>,
    pub flags: Option<String>,
    pub actual_params: Option<String>,
}

/// 启动恢复的结果，界面上会提示用户。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    /// 同步请求途中被中断、需要重新提交的任务数（这部分费用已经花掉了）
    pub resubmitted: u64,
    /// 异步任务已拿到远端任务 ID、改为继续轮询的任务数
    pub resumed_polling: u64,
}
