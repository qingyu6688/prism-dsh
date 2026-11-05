//! 方法分发，方法表见 `docs/prism/service-api.md` 第 3 节。
//!
//! 各分组的方法都是薄薄一层：解析参数、调业务 crate、把结果序列化，和原 Tauri 版的命令一一对应。

mod catalog;
mod rules;
mod search;

use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use prism_catalog::Catalog;
use prism_core::{ApiError, ApiErrorCode};
use prism_store::Store;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::protocol::RpcError;

pub use rules::RULES_DIR;

/// 收到后在响应发出去之后开始退出。
pub const SHUTDOWN: &str = "service.shutdown";

/// 各方法共享的状态。
pub struct Context {
    store: Store,
    data_dir: PathBuf,
    catalog: Catalog,
    rules: rules::Rules,
    shutdown: watch::Sender<bool>,
}

impl std::fmt::Debug for Context {
    // Catalog 没有实现 Debug，打印数据目录就够定位了
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("data_dir", &self.data_dir)
            .finish_non_exhaustive()
    }
}

impl Context {
    /// 建好商品库、加载数据目录 `rules/` 下的用户规则。
    pub fn new(store: Store, data_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            catalog: Catalog::new(store.clone(), &data_dir),
            rules: rules::Rules::load(data_dir.join(RULES_DIR)),
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

/// 执行一个方法。
pub async fn dispatch(ctx: &Context, method: &str, params: Value) -> Result<Value, RpcError> {
    match method.split_once('.') {
        Some(("service", name)) => service(ctx, method, name).await,
        Some(("catalog", name)) => catalog::dispatch(ctx, method, name, params).await,
        Some(("rules", name)) => rules::dispatch(ctx, method, name, params),
        Some(("search", name)) => search::dispatch(ctx, method, name, params).await,
        _ => Err(RpcError::method_not_found(method)),
    }
}

/// `service.*` 不需要参数，传了也忽略，方便以后加可选参数。
async fn service(ctx: &Context, method: &str, name: &str) -> Result<Value, RpcError> {
    match name {
        "ping" => Ok(json!({ "pong": true })),
        "version" => {
            let schema_version = ctx.store.schema_version().await.map_err(fail)?;
            Ok(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "dataDir": ctx.data_dir.display().to_string(),
                "schemaVersion": schema_version,
            }))
        }
        "shutdown" => Ok(json!({})),
        _ => Err(RpcError::method_not_found(method)),
    }
}

/// 把参数对象解析成方法的参数类型；出错时说明是哪个字段。
pub(crate) fn parse<T: DeserializeOwned>(params: Value) -> Result<T, RpcError> {
    serde_path_to_error::deserialize(params).map_err(|error| {
        let path = error.path().to_string();
        let inner = error.into_inner();
        if path == "." {
            RpcError::invalid_params(format!("参数不对：{inner}"))
        } else {
            RpcError::invalid_params(format!("参数 {path} 不对：{inner}"))
        }
    })
}

/// 把返回值序列化成 JSON。
pub(crate) fn reply<T: Serialize>(value: T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|error| {
        tracing::error!(%error, "序列化返回值失败");
        RpcError::from(ApiError::new(ApiErrorCode::Internal, "整理返回结果失败"))
    })
}

/// 业务错误：原始错误写日志，返回给用户看的说明（错误类型自己的 `Into<ApiError>`）。
pub(crate) fn fail<E: Into<ApiError> + Display>(error: E) -> RpcError {
    tracing::warn!(%error, "请求失败");
    RpcError::from(error.into())
}

/// 没有返回值的方法统一回 `{}`。
pub(crate) fn done() -> Value {
    json!({})
}
