//! 调度引擎：持久化队列、全局和单配置并发控制、令牌桶、自适应并发、重试和熔断、
//! 异步轮询、崩溃恢复、进度事件。
//!
//! 引擎只认"任务"，不知道"裂变"是什么：请求怎么构造、结果怎么入库由 [`JobHandler`] 决定，
//! 配置怎么变成供应商由 [`ProviderFactory`] 决定。设计见 `docs/detail-design.md` 第 2 章。

mod config;
mod engine;
mod factory;
pub mod limits;
mod profiles;
mod types;

pub use config::EngineConfig;
pub use engine::{Engine, EngineError};
pub use factory::KeyedFactory;
pub use profiles::{ProfileSnapshot, ProviderHandle};
pub use types::{
    EngineEvent, EventSink, HandlerError, JobChange, JobHandler, JobOutput, JobRequest,
    ProviderFactory, Purpose, Rewrite,
};

/// 锁被毒化（持锁的协程 panic）时继续用里面的数据，不让一个任务的崩溃拖垮整个引擎。
pub(crate) fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
