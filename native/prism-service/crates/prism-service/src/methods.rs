//! 方法分发。D2 只有 `service.*`，业务方法在 D3、D4 按 `docs/prism/service-api.md` 第 3 节加。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use prism_core::{ApiError, ApiErrorCode};
use prism_store::Store;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::protocol::RpcError;

/// 收到后在响应发出去之后开始退出。
pub const SHUTDOWN: &str = "service.shutdown";

/// 各方法共享的状态。
#[derive(Debug)]
pub struct Context {
    store: Store,
    data_dir: PathBuf,
    shutdown: watch::Sender<bool>,
}

impl Context {
    pub fn new(store: Store, data_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            store,
            data_dir,
            shutdown: watch::Sender::new(false),
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 让服务循环停止接收新请求并退出。
    pub fn request_shutdown(&self) {
        self.shutdown.send_replace(true);
    }

    pub(crate) fn shutdown_signal(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }
}

/// 执行一个方法。`service.*` 不需要参数，传了也忽略，方便以后加可选参数。
pub async fn dispatch(ctx: &Context, method: &str, _params: Value) -> Result<Value, RpcError> {
    match method {
        "service.ping" => Ok(json!({ "pong": true })),
        "service.version" => version(ctx).await,
        SHUTDOWN => Ok(json!({})),
        _ => Err(RpcError::method_not_found(method)),
    }
}

async fn version(ctx: &Context) -> Result<Value, RpcError> {
    let schema_version = ctx.store.schema_version().await.map_err(|e| {
        tracing::error!(error = %e, "读取数据库迁移版本失败");
        ApiError::new(ApiErrorCode::Storage, "读取数据库失败")
    })?;
    Ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "dataDir": ctx.data_dir.display().to_string(),
        "schemaVersion": schema_version,
    }))
}
