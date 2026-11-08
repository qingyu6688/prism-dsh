//! 生图配置、持久化批次及结果查询。引擎和凭据由服务实例持有。

use super::{Context, fail, parse, secrets::Secrets};
use crate::protocol::RpcError;
use prism_catalog::{Catalog, CatalogError};
use prism_core::{ApiError, UpstreamErrorCode, now_ms};
use prism_engine::{
    Engine, EngineConfig, EngineEvent, EventSink, KeyedFactory, ProviderFactory, ProviderHandle,
};
use prism_pipeline::{ImageArgs, ImagePipeline};
use prism_provider::{ApiKey, ProviderError};
use prism_store::{ApiProfileRecord, NewApiProfile, Store, StoreError};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, broadcast};

/// 服务退出前停止引擎；同实例的配置变更和提交串行执行。
pub struct ImageRuntime {
    pub secrets: Arc<Secrets>,
    pub credential_lock: Mutex<()>,
    pub events: broadcast::Sender<Value>,
    pipeline: Arc<ImagePipeline>,
    catalog: Arc<Catalog>,
    engine: Mutex<Option<Engine>>,
}
struct Factory(Arc<Secrets>);
impl ProviderFactory for Factory {
    fn build(&self, profile: &ApiProfileRecord) -> Result<ProviderHandle, ProviderError> {
        let key = self
            .0
            .get(&format!("image/{}", profile.credential_id()))
            .map_err(|e| ProviderError::new(UpstreamErrorCode::Auth, e.message))?
            .ok_or_else(|| {
                ProviderError::new(UpstreamErrorCode::Auth, "生图配置尚未保存 API Key")
            })?;
        KeyedFactory::new()
            .with_key(&profile.id, ApiKey::new(key))
            .build(profile)
    }
}
struct Events(broadcast::Sender<Value>);
impl EventSink for Events {
    fn emit(&self, event: EngineEvent) {
        // 没有订阅者时不保留通知，完整状态始终可从数据库查询。
        let _ = self
            .0
            .send(json!({"jsonrpc":"2.0", "method":"image.changed", "params": event}));
    }
}
impl ImageRuntime {
    pub fn new(store: Store, dir: PathBuf) -> Self {
        let catalog = Arc::new(Catalog::new(store, &dir));
        Self {
            secrets: Arc::new(Secrets::new(&dir)),
            credential_lock: Mutex::new(()),
            events: broadcast::channel(256).0,
            pipeline: Arc::new(ImagePipeline::new(catalog.clone())),
            catalog,
            engine: Mutex::new(None),
        }
    }
    pub async fn start(&self) -> Result<(), RpcError> {
        let mut engine = self.engine.lock().await;
        if engine.is_none() {
            *engine = Some(
                Engine::start(
                    self.catalog.store().clone(),
                    self.pipeline.clone(),
                    Arc::new(Factory(self.secrets.clone())),
                    Arc::new(Events(self.events.clone())),
                    EngineConfig::default(),
                )
                .await
                .map_err(engine_error)?,
            );
        }
        Ok(())
    }
    pub async fn stop(&self) {
        if let Some(engine) = self.engine.lock().await.take() {
            engine.shutdown().await;
        }
    }
}
fn engine_error(error: impl std::fmt::Display) -> RpcError {
    ApiError::invalid_input(error.to_string()).into()
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProfileInput {
    id: Option<String>,
    name: String,
    base_url: String,
    model: String,
    provider_type: String,
    api_key: Option<String>,
    max_concurrency: i64,
    timeout_secs: i64,
}
fn checked_profile(input: &ProfileInput) -> Result<NewApiProfile, RpcError> {
    if input.name.trim().is_empty()
        || input.name.chars().count() > 50
        || input.model.trim().is_empty()
        || input.model.len() > 100
        || !(1..=64).contains(&input.max_concurrency)
        || !(10..=1800).contains(&input.timeout_secs)
        || !matches!(input.provider_type.as_str(), "openai_images" | "async_task")
    {
        return Err(RpcError::invalid_params(
            "请检查名称、模型、接口类型、并发（1～64）和超时（10～1800 秒）",
        ));
    }
    let url = input.base_url.trim().trim_end_matches('/');
    let parsed =
        reqwest::Url::parse(url).map_err(|_| RpcError::invalid_params("接口地址格式不正确"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(RpcError::invalid_params(
            "接口地址必须是 HTTP(S) 地址，不能含凭据、查询参数或片段",
        ));
    }
    Ok(NewApiProfile {
        name: input.name.trim().into(),
        purpose: "image".into(),
        access_type: "relay".into(),
        provider_type: input.provider_type.clone(),
        base_url: url.into(),
        model: input.model.trim().into(),
        model_is_custom: true,
        max_concurrency: input.max_concurrency,
        rpm_limit: None,
        weight: 1,
        timeout_secs: input.timeout_secs,
        transparent_method: "api".into(),
        capabilities: None,
        custom_spec: None,
    })
}
fn profile_view(runtime: &ImageRuntime, p: ApiProfileRecord) -> Result<Value, RpcError> {
    Ok(
        json!({"id":p.id, "name":p.name, "baseUrl":p.base_url, "model":p.model,
        "providerType":p.provider_type, "maxConcurrency":p.max_concurrency, "timeoutSecs":p.timeout_secs,
        "enabled":p.enabled, "disabledReason":p.disabled_reason,
        "hasKey":runtime.secrets.get(&format!("image/{}", p.credential_id()))?.is_some()}),
    )
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Submit {
    owner: String,
    args: ImageArgs,
    #[serde(default)]
    fission: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id {
    id: String,
}

async fn batch_view(ctx: &Context, id: &str) -> Result<Value, RpcError> {
    let b = ctx.store.get_batch(id).await.map_err(fail)?;
    if b.module != "image" {
        return Err(RpcError::invalid_params("该批次不是生图任务"));
    }
    let mut assets = Vec::new();
    for id in ctx
        .images
        .pipeline
        .results(id)
        .await
        .map_err(engine_error)?
    {
        match ctx.images.catalog.get_asset(&id).await {
            Ok(asset) => assets.push(asset),
            // 用户可在素材库删除生成图片；历史批次仍保留完成状态。
            Err(CatalogError::Store(StoreError::NotFound { .. })) => {}
            Err(error) => return Err(fail(error)),
        }
    }
    let errors: Vec<_> = ctx
        .store
        .list_jobs(id)
        .await
        .map_err(fail)?
        .into_iter()
        .filter_map(|j| j.error_message)
        .take(10)
        .collect();
    Ok(
        json!({"id":b.id,"seq":b.seq,"owner":serde_json::from_str::<Value>(&b.params).ok().and_then(|p| p.get("sessionId").cloned()),"status":b.status,"total":b.total_jobs,
        "succeeded":b.succeeded_jobs,"failed":b.failed_jobs,"reason":b.pause_reason,"assets":assets,"errors":errors}),
    )
}

/// 执行生图操作；审批由发起工具的 Host 负责，服务验证参数与资源。
pub async fn dispatch(
    ctx: &Context,
    method: &str,
    name: &str,
    params: Value,
) -> Result<Value, RpcError> {
    let runtime = &ctx.images;
    match name {
        "listProfiles" => {
            let mut result = Vec::new();
            for p in ctx.store.list_profiles(Some("image")).await.map_err(fail)? {
                result.push(profile_view(runtime, p)?);
            }
            Ok(json!(result))
        }
        "saveProfile" => {
            let input: ProfileInput = parse(params)?;
            let profile = checked_profile(&input)?;
            let _guard = runtime.credential_lock.lock().await;
            let id = input.id.clone().unwrap_or_else(prism_core::new_id);
            if input.id.is_some() {
                let old = ctx.store.get_profile(&id).await.map_err(fail)?;
                if old.purpose != "image" {
                    return Err(RpcError::invalid_params("只能修改生图配置"));
                }
            }
            let key_name = format!("image/{id}");
            let previous = runtime.secrets.get(&key_name)?;
            if previous.is_none() && input.api_key.as_deref().is_none_or(|k| k.trim().is_empty()) {
                return Err(RpcError::invalid_params("新配置需要 API Key"));
            }
            if let Some(key) = input.api_key.as_deref().filter(|k| !k.trim().is_empty()) {
                runtime.secrets.set(&key_name, key)?;
            }
            let saved = if input.id.is_some() {
                ctx.store.update_profile(&id, &profile, now_ms()).await
            } else {
                ctx.store
                    .save_profile_connection(&id, &profile, None, now_ms())
                    .await
            };
            if let Err(error) = saved {
                match previous {
                    Some(value) => runtime.secrets.set(&key_name, &value)?,
                    None => runtime.secrets.delete(&key_name)?,
                }
                return Err(fail(error));
            }
            ctx.store
                .set_profile_enabled(&id, true, None, now_ms())
                .await
                .map_err(fail)?;
            if let Some(engine) = runtime.engine.lock().await.as_ref() {
                engine.reload_profiles().await.map_err(engine_error)?;
            }
            profile_view(runtime, ctx.store.get_profile(&id).await.map_err(fail)?)
        }
        "deleteProfile" => {
            let req: Id = parse(params)?;
            let _guard = runtime.credential_lock.lock().await;
            let p = ctx.store.get_profile(&req.id).await.map_err(fail)?;
            if p.purpose != "image" {
                return Err(RpcError::invalid_params("只能删除生图配置"));
            }
            let key_name = format!("image/{}", p.credential_id());
            let previous = runtime.secrets.get(&key_name)?;
            runtime.secrets.delete(&key_name)?;
            if let Err(error) = ctx.store.delete_profile(&req.id).await {
                if let Some(value) = previous {
                    runtime.secrets.set(&key_name, &value)?;
                }
                return Err(fail(error));
            }
            if let Some(engine) = runtime.engine.lock().await.as_ref() {
                engine.reload_profiles().await.map_err(engine_error)?;
            }
            Ok(json!({}))
        }
        "testProfile" => {
            let req: Id = parse(params)?;
            let p = ctx.store.get_profile(&req.id).await.map_err(fail)?;
            let key = runtime
                .secrets
                .get(&format!("image/{}", p.credential_id()))?
                .ok_or_else(|| RpcError::invalid_params("请先保存 API Key"))?;
            let config = prism_provider::ProviderConfig::new(p.base_url, ApiKey::new(key), p.model);
            let models = prism_provider::list_models(&config)
                .await
                .map_err(engine_error)?;
            Ok(json!({"supported":models.supported,"models":models.image_models}))
        }
        "submitBatch" => {
            let req: Submit = parse(params)?;
            if req.owner.trim().is_empty() || req.owner.len() > 256 {
                return Err(RpcError::invalid_params("缺少有效的会话标识"));
            }
            runtime
                .pipeline
                .validate(&req.args, req.fission)
                .await
                .map_err(engine_error)?;
            runtime.start().await?;
            let guard = runtime.engine.lock().await;
            let engine = guard
                .as_ref()
                .ok_or_else(|| RpcError::invalid_params("生图引擎正在关闭"))?;
            engine.reload_profiles().await.map_err(engine_error)?;
            if engine.profile_snapshots().is_empty() {
                return Err(RpcError::invalid_params(
                    "请先在设置中保存可用的生图配置和 API Key",
                ));
            }
            let id = runtime
                .pipeline
                .submit_session(&req.owner, &req.args, req.fission)
                .await
                .map_err(engine_error)?;
            engine.notify();
            batch_view(ctx, &id).await
        }
        "batchStatus" => {
            let req: Id = parse(params)?;
            batch_view(ctx, &req.id).await
        }
        "cancelBatch" => {
            let req: Id = parse(params)?;
            batch_view(ctx, &req.id).await?;
            ctx.store
                .cancel_batch(&req.id, now_ms())
                .await
                .map_err(fail)?;
            batch_view(ctx, &req.id).await
        }
        "listBatches" => {
            let mut result = Vec::new();
            for batch in ctx.store.list_batches(100, 0).await.map_err(fail)? {
                if batch.module == "image" {
                    result.push(batch_view(ctx, &batch.id).await?);
                }
            }
            Ok(json!(result))
        }
        _ => Err(RpcError::method_not_found(method)),
    }
}
