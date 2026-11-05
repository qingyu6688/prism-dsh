//! 默认值和上限，对应 `docs/detail-design.md` 第 8 章。
//!
//! 设置页里能改的项，这里是出厂默认；不能改的项（如单批次上限）是硬限制。

use crate::error::ApiError;

/// 生图配置的默认模型。
pub const DEFAULT_IMAGE_MODEL: &str = "gpt-image-2.5-sunburst";

/// 每张原图默认裂变几张。
pub const DEFAULT_VARIANTS_PER_SOURCE: u32 = 4;
/// 每张原图最多裂变几张。
pub const MAX_VARIANTS_PER_SOURCE: u32 = 16;
/// 一次最多上传几张原图。
pub const MAX_SOURCES_PER_BATCH: u32 = 50;
/// 单张原图大小上限（20 MB）。
pub const MAX_SOURCE_BYTES: u64 = 20 * 1024 * 1024;
/// 单批次生图任务数上限，防止误操作一次烧掉大量额度。
pub const MAX_JOBS_PER_BATCH: u32 = 500;

/// 生图全局并发。
pub const DEFAULT_GLOBAL_IMAGE_CONCURRENCY: u32 = 10;
/// 看图规划全局并发。
pub const DEFAULT_PLAN_CONCURRENCY: u32 = 4;
/// 单个任务最多尝试次数（429 重新排队不计入）。
pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;
/// 同步请求超时。4K 图生成慢，不能设太短。
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 600;
/// 异步任务轮询间隔。
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;
/// 异步任务从提交到出结果的总超时。
pub const ASYNC_TOTAL_TIMEOUT_SECS: u64 = 30 * 60;
/// Agent 发起的生图数超过这个值时先让用户确认。
pub const AGENT_CONFIRM_THRESHOLD: u32 = 40;
/// 进度事件合并推送的间隔。
pub const PROGRESS_PUSH_INTERVAL_MS: u64 = 200;

/// 校验一次裂变的规模，返回会产生的生图任务数。
pub fn validate_batch_size(sources: u32, variants_per_source: u32) -> Result<u32, ApiError> {
    if sources == 0 {
        return Err(ApiError::invalid_input("至少上传 1 张图片"));
    }
    if sources > MAX_SOURCES_PER_BATCH {
        return Err(ApiError::invalid_input(format!(
            "一次最多上传 {MAX_SOURCES_PER_BATCH} 张图片"
        )));
    }
    if variants_per_source == 0 || variants_per_source > MAX_VARIANTS_PER_SOURCE {
        return Err(ApiError::invalid_input(format!(
            "每张图可以裂变 1 到 {MAX_VARIANTS_PER_SOURCE} 张"
        )));
    }
    let jobs = sources * variants_per_source;
    if jobs > MAX_JOBS_PER_BATCH {
        return Err(ApiError::new(
            crate::error::ApiErrorCode::LimitExceeded,
            format!("这次会生成 {jobs} 张，超过单批次上限 {MAX_JOBS_PER_BATCH} 张，请分几批提交"),
        ));
    }
    Ok(jobs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ApiErrorCode;

    #[test]
    fn five_sources_default_count_is_twenty_jobs() {
        assert_eq!(
            validate_batch_size(5, DEFAULT_VARIANTS_PER_SOURCE).unwrap(),
            20
        );
    }

    #[test]
    fn rejects_out_of_range_input() {
        assert_eq!(
            validate_batch_size(0, 4).unwrap_err().code,
            ApiErrorCode::InvalidInput
        );
        assert_eq!(
            validate_batch_size(51, 4).unwrap_err().code,
            ApiErrorCode::InvalidInput
        );
        assert_eq!(
            validate_batch_size(1, 17).unwrap_err().code,
            ApiErrorCode::InvalidInput
        );
        assert_eq!(
            validate_batch_size(1, 0).unwrap_err().code,
            ApiErrorCode::InvalidInput
        );
    }

    #[test]
    fn rejects_batches_over_job_limit() {
        // 50 × 16 = 800，超过 500
        let err = validate_batch_size(50, 16).unwrap_err();
        assert_eq!(err.code, ApiErrorCode::LimitExceeded);
        assert!(validate_batch_size(50, 10).is_ok());
    }
}
