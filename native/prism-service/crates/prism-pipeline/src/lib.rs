//! 对话作图：普通 AI 生图与显式图裂变共用持久化引擎，结果进入素材库。

use std::sync::Arc;

use async_trait::async_trait;
use prism_catalog::{Catalog, ImportOptions};
use prism_core::{JobKind, now_ms};
use prism_engine::{HandlerError, JobHandler, JobOutput, JobRequest, Purpose};
use prism_provider::{ImageInput, ImageRequest};
use prism_store::{JobRecord, NewBatch, NewJob};
use serde::{Deserialize, Serialize};

/// 模型提交的作图参数。生图默认一张，图裂变的数量由工具补成四张。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageArgs {
    pub prompt: String,
    #[serde(default = "one")]
    pub count: u32,
    #[serde(default)]
    pub reference_ids: Vec<String>,
    pub product_id: Option<String>,
    pub size: Option<String>,
}

fn one() -> u32 {
    1
}

impl ImageArgs {
    /// 所有工具入口先校验，避免空指令、过大批次或没有原图的裂变发往上游。
    pub fn validate(&self, fission: bool) -> Result<(), String> {
        if self.prompt.trim().is_empty() || self.prompt.chars().count() > 4000 {
            return Err("生图指令需要 1～4000 字".into());
        }
        if !(1..=16).contains(&self.count) || self.reference_ids.len() > 6 {
            return Err("每次生成 1～16 张，最多引用 6 张图片".into());
        }
        if fission && self.reference_ids.is_empty() {
            return Err("图裂变需要原图，请添加图片附件或引用素材".into());
        }
        if let Some(size) = &self.size {
            let valid = size.split_once('x').is_some_and(|(w, h)| {
                [w, h].iter().all(|s| {
                    s.parse::<u32>()
                        .is_ok_and(|v| (256..=4096).contains(&v) && v % 16 == 0)
                })
            });
            if !valid {
                return Err("尺寸应为 256～4096 范围内的 16 倍数，例如 1024x1024".into());
            }
        }
        Ok(())
    }
}

/// 图像任务处理器，同时提供创建批次与读取结果的业务接口。
pub struct ImagePipeline {
    catalog: Arc<Catalog>,
}

impl ImagePipeline {
    /// 与应用的商品库共用存储和素材目录。
    pub fn new(catalog: Arc<Catalog>) -> Self {
        Self { catalog }
    }

    /// 校验素材和关联商品存在，不接受任意本机路径。
    pub async fn validate(&self, args: &ImageArgs, fission: bool) -> Result<(), String> {
        args.validate(fission)?;
        if let Some(id) = &args.product_id {
            self.catalog
                .product_detail(id)
                .await
                .map_err(|e| e.to_string())?;
        }
        for id in &args.reference_ids {
            let asset = self
                .catalog
                .get_asset(id)
                .await
                .map_err(|e| e.to_string())?;
            if asset.kind != "image" {
                return Err("参考素材必须是图片".into());
            }
        }
        Ok(())
    }

    /// 建一个批次，每张图一个任务。普通生图不会自动扩展为裂变。
    pub async fn submit(
        &self,
        conversation_id: &str,
        args: &ImageArgs,
        fission: bool,
    ) -> Result<String, String> {
        self.validate(args, fission).await?;
        let store = self.catalog.store();
        let now = now_ms();
        let params = serde_json::to_string(args).map_err(|e| e.to_string())?;
        let batch = store
            .create_batch(
                &NewBatch {
                    module: "image".into(),
                    product_id: args.product_id.clone(),
                    conversation_id: Some(conversation_id.into()),
                    mode: if fission { "similar" } else { "generate" }.into(),
                    variants_per_source: i64::from(args.count),
                    params: params.clone(),
                },
                now,
            )
            .await
            .map_err(|e| e.to_string())?;
        let jobs: Vec<NewJob> = (1..=args.count).map(|index| NewJob {
            batch_id: batch.id.clone(), source_id: None, kind: JobKind::Generate,
            task_type: if fission { "image.fission" } else { "image.generate" }.into(),
            product_id: args.product_id.clone(), variant_index: i64::from(index), axis: None,
            instruction: Some(if fission {
                format!("{}\n以参考图为原图，保留主体和商品细节，生成第 {index} 个具有不同构图或场景的变体。", args.prompt)
            } else { args.prompt.clone() }),
            priority: 1, max_attempts: 3, request_params: Some(params.clone()),
        }).collect();
        if let Err(error) = store.insert_jobs(&jobs, now).await {
            store
                .cancel_batch(&batch.id, now_ms())
                .await
                .map_err(|e| e.to_string())?;
            return Err(error.to_string());
        }
        Ok(batch.id)
    }

    /// 按任务顺序汇总结果，内容去重后的素材仍能出现在对应计划里。
    pub async fn results(&self, batch_id: &str) -> Result<Vec<String>, String> {
        let store = self.catalog.store();
        let mut ids = Vec::new();
        for job in store.list_jobs(batch_id).await.map_err(|e| e.to_string())? {
            if let Some(value) = store
                .get_setting(&result_key(&job.id))
                .await
                .map_err(|e| e.to_string())?
            {
                let items: Vec<String> = serde_json::from_str(&value).map_err(|e| e.to_string())?;
                for id in items {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
        }
        Ok(ids)
    }
}

fn result_key(job_id: &str) -> String {
    format!("image.job_results.{job_id}")
}
fn failure(error: impl std::fmt::Display) -> HandlerError {
    HandlerError::fatal(error.to_string())
}

#[async_trait]
impl JobHandler for ImagePipeline {
    fn purpose(&self, _: &JobRecord) -> Purpose {
        Purpose::Image
    }

    async fn build_request(&self, job: &JobRecord) -> Result<JobRequest, HandlerError> {
        let args: ImageArgs =
            serde_json::from_str(job.request_params.as_deref().unwrap_or("{}")).map_err(failure)?;
        let mut request = ImageRequest::new(job.instruction.clone().unwrap_or(args.prompt));
        request.size = args.size;
        for id in args.reference_ids {
            let asset = self.catalog.get_asset(&id).await.map_err(failure)?;
            let path = self
                .catalog
                .asset_path(&asset)
                .ok_or_else(|| failure("参考图没有文件"))?;
            request.references.push(ImageInput {
                bytes: tokio::fs::read(path).await.map_err(failure)?,
                mime: asset.mime.unwrap_or_else(|| "image/png".into()),
                file_name: format!("{id}.png"),
            });
        }
        Ok(JobRequest::Image(request))
    }

    async fn on_output(&self, job: &JobRecord, output: JobOutput) -> Result<(), HandlerError> {
        let JobOutput::Images(images) = output else {
            return Err(failure("生图任务收到非图片结果"));
        };
        if images.is_empty() {
            return Err(HandlerError::retryable("生图接口没有返回图片"));
        }
        let mut ids = Vec::new();
        for image in images {
            let result = self
                .catalog
                .import_image_bytes(
                    &image.bytes,
                    &ImportOptions {
                        product_id: job.product_id.clone(),
                        source: "generated".into(),
                        title: Some(format!("AI 生图 · {}", job.variant_index)),
                        origin_batch_id: Some(job.batch_id.clone()),
                        origin_job_id: Some(job.id.clone()),
                        ..Default::default()
                    },
                )
                .await
                .map_err(failure)?;
            ids.push(result.asset.id);
        }
        let store = self.catalog.store();
        store
            .set_setting(
                &result_key(&job.id),
                &serde_json::to_string(&ids).map_err(failure)?,
                now_ms(),
            )
            .await
            .map_err(failure)?;
        store
            .succeed_job(&job.id, &[], now_ms())
            .await
            .map_err(failure)?;
        Ok(())
    }
}
