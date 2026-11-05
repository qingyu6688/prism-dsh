//! `batch` 和 `source_image` 表。

use prism_core::new_id;
use sqlx::SqliteConnection;

use crate::{BatchRecord, NewBatch, NewSourceImage, Result, SourceImageRecord, Store, StoreError};

/// 图裂变批次的模式。其他模块的子模式由各自的模块定义，这里只要求非空。
const IMAGE_FISSION_MODES: &[&str] = &["similar", "theme", "palette", "text"];

impl Store {
    /// 新建批次，状态为 `running`。序号在同一条语句里分配，并发创建也不会重复。
    pub async fn create_batch(&self, batch: &NewBatch, now: i64) -> Result<BatchRecord> {
        check_mode(batch)?;
        let id = new_id();
        sqlx::query(
            "INSERT INTO batch (id, seq, module, product_id, conversation_id, mode,
                                variants_per_source, params, status, created_at, updated_at)
             SELECT ?, COALESCE(MAX(seq), 0) + 1, ?, ?, ?, ?, ?, ?, 'running', ?, ? FROM batch",
        )
        .bind(&id)
        .bind(&batch.module)
        .bind(&batch.product_id)
        .bind(&batch.conversation_id)
        .bind(&batch.mode)
        .bind(batch.variants_per_source)
        .bind(&batch.params)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.get_batch(&id).await
    }

    pub async fn get_batch(&self, id: &str) -> Result<BatchRecord> {
        sqlx::query_as::<_, BatchRecord>("SELECT * FROM batch WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::not_found("批次", id))
    }

    /// 按创建顺序倒序列出批次。
    pub async fn list_batches(&self, limit: i64, offset: i64) -> Result<Vec<BatchRecord>> {
        let rows = sqlx::query_as::<_, BatchRecord>(
            "SELECT * FROM batch ORDER BY seq DESC LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 暂停运行中的批次（失败率过高、没有可用配置时由引擎调用，也可以由用户手动暂停）。
    /// 暂停期间这个批次的任务不会被派发，已发出的请求照常完成。
    pub async fn pause_batch(&self, id: &str, reason: &str, now: i64) -> Result<()> {
        let result = sqlx::query(
            "UPDATE batch SET status = 'paused', pause_reason = ?, updated_at = ?
             WHERE id = ? AND status = 'running'",
        )
        .bind(reason)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::state_changed("批次", id));
        }
        Ok(())
    }

    /// 继续已暂停的批次。
    pub async fn resume_batch(&self, id: &str, now: i64) -> Result<()> {
        let result = sqlx::query(
            "UPDATE batch SET status = 'running', pause_reason = NULL, updated_at = ?
             WHERE id = ? AND status = 'paused'",
        )
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::state_changed("批次", id));
        }
        Ok(())
    }

    /// 新增一张原图。
    pub async fn insert_source(
        &self,
        source: &NewSourceImage,
        now: i64,
    ) -> Result<SourceImageRecord> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO source_image (id, batch_id, seq, ref_no, parent_image_id, file_path,
                                       sha256, width, height, phash, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&source.batch_id)
        .bind(source.seq)
        .bind(&source.ref_no)
        .bind(&source.parent_image_id)
        .bind(&source.file_path)
        .bind(&source.sha256)
        .bind(source.width)
        .bind(source.height)
        .bind(&source.phash)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.get_source(&id).await
    }

    pub async fn get_source(&self, id: &str) -> Result<SourceImageRecord> {
        sqlx::query_as::<_, SourceImageRecord>("SELECT * FROM source_image WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::not_found("原图", id))
    }

    pub async fn list_sources(&self, batch_id: &str) -> Result<Vec<SourceImageRecord>> {
        let rows = sqlx::query_as::<_, SourceImageRecord>(
            "SELECT * FROM source_image WHERE batch_id = ? ORDER BY seq",
        )
        .bind(batch_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

/// 批次里所有任务都结束后，把批次标记为完成（至少一个生图成功）或失败。
/// 只处理运行中和暂停中的批次，已取消的批次保持取消。
pub(crate) async fn refresh_batch_status(
    conn: &mut SqliteConnection,
    batch_id: &str,
    now: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE batch
         SET status = CASE WHEN succeeded_jobs > 0 THEN 'completed' ELSE 'failed' END,
             pause_reason = NULL, finished_at = ?1, updated_at = ?1
         WHERE id = ?2 AND status IN ('running', 'paused')
           AND NOT EXISTS (
               SELECT 1 FROM job
               WHERE job.batch_id = ?2 AND job.status NOT IN ('succeeded', 'failed', 'cancelled')
           )",
    )
    .bind(now)
    .bind(batch_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// 迁移 0002 去掉了 `batch.mode` 的 CHECK（各模块子模式不同），图裂变的模式改在这里校验。
fn check_mode(batch: &NewBatch) -> Result<()> {
    if batch.mode.trim().is_empty() {
        return Err(StoreError::Invalid("批次模式不能为空".into()));
    }
    if batch.module == "image"
        && batch.mode != "generate"
        && !IMAGE_FISSION_MODES.contains(&batch.mode.as_str())
    {
        return Err(StoreError::Invalid(format!(
            "图裂变批次不支持模式「{}」，可选：{}",
            batch.mode,
            IMAGE_FISSION_MODES.join(" / ")
        )));
    }
    Ok(())
}
