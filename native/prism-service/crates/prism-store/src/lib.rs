//! 数据存储：SQLite 建表迁移、各表读写、任务抢占和状态更新、崩溃恢复。
//!
//! 只负责读写，不含业务规则。表结构见 `docs/database.md`，迁移文件在仓库根目录的 `migrations/`。
//!
//! SQL 用运行时查询（`sqlx::query`），不用编译期检查的宏：这样贡献者改 SQL 不需要装 sqlx-cli
//! 或准备数据库，SQL 的正确性由 `tests/` 里的测试保证。
//!
//! 所有写操作都由调用方传入当前时间 `now`（Unix 毫秒），方便测试。

mod agent;
mod batches;
mod catalog;
mod db;
mod error;
mod jobs;
mod mappings;
mod migrate;
mod notifications;
mod profiles;
mod records;
mod search;
mod settings;

pub use agent::{ConversationRecord, MessageRecord, NewPlan, NewStep, PlanRecord, StepRecord};
pub use catalog::{
    AssetQuery, AssetRecord, CopyRecord, NewAsset, NewCopy, NewProduct, NewShop, NewSku,
    ProductQuery, ProductRecord, ProductSummaryRecord, ShopRecord, SkuRecord,
};
pub use db::Store;
pub use error::StoreError;
pub use mappings::{ImportMappingRecord, NewImportMapping};
pub use notifications::NotificationRecord;
pub use records::{
    ApiProfileRecord, BatchRecord, JobRecord, NewApiProfile, NewBatch, NewImage, NewJob,
    NewSourceImage, RecoveryReport, SourceImageRecord,
};

/// 存储层的返回类型。
pub type Result<T> = std::result::Result<T, StoreError>;
