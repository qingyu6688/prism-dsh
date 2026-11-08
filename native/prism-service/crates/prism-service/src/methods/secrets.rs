//! 系统凭据管理器读写。SQLite 只保存凭据名称索引；秘密不写数据库或日志。

use super::{Context, fail, parse};
use crate::protocol::RpcError;
use prism_core::{ApiError, ApiErrorCode, now_ms};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

/// 每个业务数据目录有独立的系统凭据命名空间。
pub struct Secrets {
    service: String,
}
impl Secrets {
    pub fn new(dir: &Path) -> Self {
        let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        Self {
            service: format!(
                "prism-{:x}",
                Sha256::digest(canonical.to_string_lossy().as_bytes())
            ),
        }
    }
    fn entry(&self, key: &str) -> Result<keyring::Entry, RpcError> {
        keyring::Entry::new(&self.service, key).map_err(secret_error)
    }
    pub fn get(&self, key: &str) -> Result<Option<String>, RpcError> {
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(secret_error(error)),
        }
    }
    pub fn set(&self, key: &str, value: &str) -> Result<(), RpcError> {
        self.entry(key)?.set_password(value).map_err(secret_error)
    }
    pub fn delete(&self, key: &str) -> Result<(), RpcError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(secret_error(error)),
        }
    }
}
fn secret_error(_: keyring::Error) -> RpcError {
    ApiError::new(
        ApiErrorCode::Keyring,
        "系统凭据读写失败，请检查凭据管理器和凭据长度",
    )
    .into()
}
#[derive(Deserialize)]
struct Request {
    key: String,
    value: Option<String>,
}

/// 仅供 Host 凭据插件使用，不注册为 Remote 或模型工具。
pub async fn dispatch(
    ctx: &Context,
    method: &str,
    name: &str,
    params: Value,
) -> Result<Value, RpcError> {
    let _guard = ctx.images.credential_lock.lock().await;
    let mut keys: Vec<String> = match ctx
        .store
        .get_setting("credentials.index")
        .await
        .map_err(fail)?
    {
        Some(value) => {
            serde_json::from_str(&value).map_err(|_| RpcError::invalid_params("凭据索引损坏"))?
        }
        None => Vec::new(),
    };
    if name == "list" {
        return Ok(json!(keys));
    }
    let req: Request = parse(params)?;
    if req.key.is_empty() || req.key.len() > 256 {
        return Err(RpcError::invalid_params("凭据名称长度不正确"));
    }
    match name {
        "get" => Ok(json!(ctx.images.secrets.get(&req.key)?)),
        "set" => {
            let value = req
                .value
                .ok_or_else(|| RpcError::invalid_params("缺少凭据内容"))?;
            if !keys.contains(&req.key) {
                keys.push(req.key.clone());
                ctx.store
                    .set_setting("credentials.index", &json!(keys).to_string(), now_ms())
                    .await
                    .map_err(fail)?;
            }
            ctx.images.secrets.set(&req.key, &value)?;
            Ok(json!({}))
        }
        "delete" => {
            ctx.images.secrets.delete(&req.key)?;
            keys.retain(|key| key != &req.key);
            ctx.store
                .set_setting("credentials.index", &json!(keys).to_string(), now_ms())
                .await
                .map_err(fail)?;
            Ok(json!({}))
        }
        _ => Err(RpcError::method_not_found(method)),
    }
}
