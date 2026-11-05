use prism_core::{ApiError, ApiErrorCode};

/// 存储层的错误。
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("数据库操作失败：{0}")]
    Database(#[from] sqlx::Error),

    /// 迁移失败，`String` 是具体原因
    #[error("数据库迁移失败：{0}")]
    Migration(String),

    #[error("找不到{entity}：{id}")]
    NotFound { entity: &'static str, id: String },

    /// 状态已经被别人改过，比如任务已被另一个协程抢走或已被取消
    #[error("{entity} {id} 的状态已变化，本次更新没有生效")]
    StateChanged { entity: &'static str, id: String },

    /// 唯一字段重复，比如商品编码已被别的商品用了
    #[error("{entity}的{field}「{value}」已经存在")]
    Duplicate {
        entity: &'static str,
        field: &'static str,
        value: String,
    },

    /// 数据不符合要求，`String` 是给用户看的说明
    #[error("{0}")]
    Invalid(String),
}

impl StoreError {
    pub(crate) fn not_found(entity: &'static str, id: &str) -> Self {
        Self::NotFound {
            entity,
            id: id.to_owned(),
        }
    }

    pub(crate) fn state_changed(entity: &'static str, id: &str) -> Self {
        Self::StateChanged {
            entity,
            id: id.to_owned(),
        }
    }
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NotFound { entity, .. } => {
                ApiError::new(ApiErrorCode::NotFound, format!("找不到这个{entity}"))
            }
            StoreError::Duplicate { .. } | StoreError::Invalid(_) => {
                ApiError::invalid_input(error.to_string())
            }
            // 原始错误只进日志，给用户看的说法要能看懂
            StoreError::Database(_)
            | StoreError::Migration(_)
            | StoreError::StateChanged { .. } => {
                ApiError::new(ApiErrorCode::Storage, "读写本地数据失败，请稍后重试")
            }
        }
    }
}
