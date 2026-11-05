//! 任务和批次的状态。
//!
//! 状态值会原样写进数据库（`job.status`、`batch.status`），所以序列化名称一旦发布就不能改。

use serde::{Deserialize, Serialize};

/// 任务类型：每张原图一个规划任务，规划完成后派生 K 个生图任务。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Plan,
    Generate,
}

impl JobKind {
    /// 数据库里存的字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Generate => "generate",
        }
    }
}

/// 单个任务的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// 等待派发
    Queued,
    /// 请求已发出（同步供应商）或正在提交（异步供应商）
    Running,
    /// 异步供应商已返回远端任务 ID，等待轮询结果
    Polling,
    /// 失败后等待重试
    RetryWait,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobStatus {
    /// 数据库里存的字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Polling => "polling",
            Self::RetryWait => "retry_wait",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// 是否已经结束，不会再被调度。
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    /// 是否允许从当前状态流转到 `next`。
    ///
    /// 引擎更新状态时先用它校验，数据库层再用带条件的 UPDATE 防并发覆盖。
    pub fn can_transition_to(self, next: JobStatus) -> bool {
        use JobStatus::*;
        match self {
            Queued => matches!(next, Running | Cancelled),
            // Running → Queued：遇到 429 重新排队，或者崩溃恢复时同步请求结果丢失
            Running => matches!(
                next,
                Succeeded | Polling | RetryWait | Failed | Cancelled | Queued
            ),
            // Polling → Queued：轮询超时后重新提交
            Polling => matches!(next, Succeeded | RetryWait | Failed | Cancelled | Queued),
            RetryWait => matches!(next, Running | Cancelled),
            // 用户点"重试"把失败任务重新排队
            Failed => matches!(next, Queued),
            Succeeded | Cancelled => false,
        }
    }
}

/// 批次状态，由任务状态汇总得出。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    Running,
    /// 失败率过高或没有可用配置时自动暂停，用户点"继续"后恢复
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl BatchStatus {
    /// 数据库里存的字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl_db_enum!(JobKind, "任务类型", [Plan, Generate]);
impl_db_enum!(
    JobStatus,
    "任务状态",
    [
        Queued, Running, Polling, RetryWait, Succeeded, Failed, Cancelled
    ]
);
impl_db_enum!(
    BatchStatus,
    "批次状态",
    [Running, Paused, Completed, Failed, Cancelled]
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_every_value() {
        for status in JobStatus::ALL {
            assert_eq!(status.as_str().parse::<JobStatus>().unwrap(), *status);
        }
        for status in BatchStatus::ALL {
            assert_eq!(status.as_str().parse::<BatchStatus>().unwrap(), *status);
        }
        for kind in JobKind::ALL {
            assert_eq!(kind.as_str().parse::<JobKind>().unwrap(), *kind);
        }
        let err = "unknown".parse::<JobStatus>().unwrap_err();
        assert_eq!(err.to_string(), "无法识别的任务状态：unknown");
    }

    const ALL: [JobStatus; 7] = [
        JobStatus::Queued,
        JobStatus::Running,
        JobStatus::Polling,
        JobStatus::RetryWait,
        JobStatus::Succeeded,
        JobStatus::Failed,
        JobStatus::Cancelled,
    ];

    #[test]
    fn normal_paths_are_allowed() {
        assert!(JobStatus::Queued.can_transition_to(JobStatus::Running));
        assert!(JobStatus::Running.can_transition_to(JobStatus::Succeeded));
        assert!(JobStatus::Running.can_transition_to(JobStatus::Polling));
        assert!(JobStatus::Polling.can_transition_to(JobStatus::Succeeded));
        assert!(JobStatus::Running.can_transition_to(JobStatus::RetryWait));
        assert!(JobStatus::RetryWait.can_transition_to(JobStatus::Running));
        assert!(JobStatus::Failed.can_transition_to(JobStatus::Queued));
    }

    #[test]
    fn finished_jobs_never_run_again() {
        for next in ALL {
            assert!(!JobStatus::Succeeded.can_transition_to(next));
            assert!(!JobStatus::Cancelled.can_transition_to(next));
        }
        assert!(!JobStatus::Succeeded.can_transition_to(JobStatus::Running));
    }

    #[test]
    fn queued_cannot_skip_running() {
        assert!(!JobStatus::Queued.can_transition_to(JobStatus::Succeeded));
        assert!(!JobStatus::Queued.can_transition_to(JobStatus::Polling));
    }

    #[test]
    fn terminal_flags() {
        let terminal: Vec<_> = ALL.into_iter().filter(|s| s.is_terminal()).collect();
        assert_eq!(
            terminal,
            vec![
                JobStatus::Succeeded,
                JobStatus::Failed,
                JobStatus::Cancelled
            ]
        );
    }

    #[test]
    fn serde_names_match_database_values() {
        for status in ALL {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, format!("\"{}\"", status.as_str()));
        }
        let json = serde_json::to_string(&BatchStatus::Paused).unwrap();
        assert_eq!(json, "\"paused\"");
        assert_eq!(JobKind::Generate.as_str(), "generate");
    }
}
