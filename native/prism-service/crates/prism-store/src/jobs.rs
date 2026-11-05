//! `job` 表：任务的派发、抢占、状态流转和崩溃恢复。规则见 `docs/detail-design.md` 第 1、2 章。
//!
//! 状态更新都带条件（`WHERE id = ? AND status IN (...)`），影响 0 行说明状态已被别人改过，
//! 返回 [`StoreError::StateChanged`]，不会覆盖别人的结果。

use prism_core::{JobKind, new_id};
use sqlx::SqliteConnection;

use crate::batches::refresh_batch_status;
use crate::{JobRecord, NewImage, NewJob, RecoveryReport, Result, Store, StoreError};

/// 可以被派发的任务所属批次必须在运行中；排序规则见详细设计 2.2：
/// 交互优先 → 规划先于生图 → 按变体序号轮转 → 先提交的批次先跑 → 批次内按原图顺序。
const DISPATCH_SQL: &str = "
    SELECT j.* FROM job j JOIN batch b ON b.id = j.batch_id
    WHERE b.status = 'running'
      AND (j.status = 'queued' OR (j.status = 'retry_wait' AND j.next_run_at <= ?1))
      AND (?2 IS NULL OR j.kind = ?2)
    ORDER BY j.priority DESC,
             CASE j.kind WHEN 'plan' THEN 0 ELSE 1 END,
             j.variant_index, j.batch_seq, j.source_seq, j.created_at
    LIMIT ?3";

impl Store {
    /// 批量新增任务。生图任务会累加到批次的 `total_jobs`。
    pub async fn insert_jobs(&self, jobs: &[NewJob], now: i64) -> Result<Vec<String>> {
        let mut tx = self.begin_write().await?;
        let ids = insert_job_rows(&mut tx, jobs, now).await?;
        tx.commit().await?;
        Ok(ids)
    }

    pub async fn get_job(&self, id: &str) -> Result<JobRecord> {
        fetch_job(&mut *self.pool.acquire().await?, id).await
    }

    /// 某个批次的全部任务，按原图、变体顺序排列。
    pub async fn list_jobs(&self, batch_id: &str) -> Result<Vec<JobRecord>> {
        let rows = sqlx::query_as::<_, JobRecord>(
            "SELECT * FROM job WHERE batch_id = ? ORDER BY source_seq, kind DESC, variant_index",
        )
        .bind(batch_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 取一批可以派发的任务。`kind` 为 `None` 时不限类型。
    pub async fn dispatchable_jobs(
        &self,
        now: i64,
        kind: Option<JobKind>,
        limit: i64,
    ) -> Result<Vec<JobRecord>> {
        let rows = sqlx::query_as::<_, JobRecord>(DISPATCH_SQL)
            .bind(now)
            .bind(kind.map(JobKind::as_str))
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    /// 抢占任务：排队或到期待重试的任务改为运行中，尝试次数加 1。
    /// 返回 `false` 表示已被别的协程抢走或状态已变，调用方跳过即可。
    pub async fn claim_job(&self, id: &str, profile_id: Option<&str>, now: i64) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE job SET status = 'running', attempt = attempt + 1, profile_id = ?,
                            next_run_at = NULL, started_at = ?, updated_at = ?
             WHERE id = ? AND status IN ('queued', 'retry_wait')",
        )
        .bind(profile_id)
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// 异步供应商提交成功：记下远端任务 ID，改为轮询中。提交后要立刻调用，崩溃恢复靠它。
    pub async fn mark_polling(&self, id: &str, remote_task_id: &str, now: i64) -> Result<()> {
        let result = sqlx::query(
            "UPDATE job SET status = 'polling', remote_task_id = ?, updated_at = ?
             WHERE id = ? AND status = 'running'",
        )
        .bind(remote_task_id)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        expect_one(result.rows_affected(), id)
    }

    /// 放回队列。遇到 429 时 `refund_attempt = true`，退回这次的尝试次数，限流不算失败。
    pub async fn requeue_job(&self, id: &str, refund_attempt: bool, now: i64) -> Result<()> {
        let result = sqlx::query(
            "UPDATE job SET status = 'queued', remote_task_id = NULL, updated_at = ?,
                            attempt = CASE WHEN ? THEN MAX(attempt - 1, 0) ELSE attempt END
             WHERE id = ? AND status IN ('running', 'polling')",
        )
        .bind(now)
        .bind(refund_attempt)
        .bind(id)
        .execute(&self.pool)
        .await?;
        expect_one(result.rows_affected(), id)
    }

    /// 失败后等待重试，到 `next_run_at` 时重新进入派发。
    pub async fn schedule_retry(
        &self,
        id: &str,
        next_run_at: i64,
        error_code: &str,
        error_message: &str,
        now: i64,
    ) -> Result<()> {
        let result = sqlx::query(
            "UPDATE job SET status = 'retry_wait', next_run_at = ?, error_code = ?, error_message = ?,
                            remote_task_id = NULL, updated_at = ?
             WHERE id = ? AND status IN ('running', 'polling')",
        )
        .bind(next_run_at)
        .bind(error_code)
        .bind(error_message)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        expect_one(result.rows_affected(), id)
    }

    /// 生图任务成功：写结果图、累加批次成功数、必要时把批次标记为完成，全部在一个事务里。
    /// 返回新结果图的 ID。任务已被取消时返回 `StateChanged`，结果不入库。
    pub async fn succeed_job(
        &self,
        id: &str,
        images: &[NewImage],
        now: i64,
    ) -> Result<Vec<String>> {
        let mut tx = self.begin_write().await?;
        let job = fetch_job(&mut tx, id).await?;
        finish_job(&mut tx, id, "succeeded", None, None, now).await?;

        // image 表只给有原图的生图任务用；其他模块的产出写素材库（P4 统一）
        let source_id = match (&job.source_id, images.is_empty()) {
            (_, true) => None,
            (Some(source_id), false) => Some(source_id.clone()),
            (None, false) => {
                return Err(StoreError::Invalid(
                    "没有原图的任务不能写入裂变结果图".into(),
                ));
            }
        };
        let mut image_ids = Vec::with_capacity(images.len());
        for image in images {
            let image_id = new_id();
            sqlx::query(
                "INSERT INTO image (id, job_id, batch_id, source_id, variant_index, ref_no, file_path,
                                    thumb_path, format, width, height, sha256, phash, dist_to_source,
                                    flags, actual_params, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&image_id)
            .bind(id)
            .bind(&job.batch_id)
            .bind(&source_id)
            .bind(image.variant_index)
            .bind(&image.ref_no)
            .bind(&image.file_path)
            .bind(&image.thumb_path)
            .bind(&image.format)
            .bind(image.width)
            .bind(image.height)
            .bind(&image.sha256)
            .bind(&image.phash)
            .bind(image.dist_to_source)
            .bind(&image.flags)
            .bind(&image.actual_params)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            image_ids.push(image_id);
        }

        if job.kind == JobKind::Generate {
            bump_batch_counter(&mut tx, &job.batch_id, "succeeded_jobs", now).await?;
        }
        refresh_batch_status(&mut tx, &job.batch_id, now).await?;
        tx.commit().await?;
        Ok(image_ids)
    }

    /// 规划任务成功：保存原图描述，派生生图任务，一个事务完成。返回新生图任务的 ID。
    pub async fn succeed_plan_job(
        &self,
        id: &str,
        analysis_json: &str,
        generate_jobs: &[NewJob],
        now: i64,
    ) -> Result<Vec<String>> {
        let mut tx = self.begin_write().await?;
        let job = fetch_job(&mut tx, id).await?;
        finish_job(&mut tx, id, "succeeded", None, None, now).await?;
        if let Some(source_id) = &job.source_id {
            sqlx::query("UPDATE source_image SET analysis = ?, updated_at = ? WHERE id = ?")
                .bind(analysis_json)
                .bind(now)
                .bind(source_id)
                .execute(&mut *tx)
                .await?;
        }
        let ids = insert_job_rows(&mut tx, generate_jobs, now).await?;
        // 规划派生的任务为空时（比如规划结果全部无效），批次可能已经没有未结束的任务了
        refresh_batch_status(&mut tx, &job.batch_id, now).await?;
        tx.commit().await?;
        Ok(ids)
    }

    /// 任务最终失败：记录错误、累加批次失败数、必要时结束批次。
    pub async fn fail_job(
        &self,
        id: &str,
        error_code: &str,
        error_message: &str,
        now: i64,
    ) -> Result<()> {
        let mut tx = self.begin_write().await?;
        let job = fetch_job(&mut tx, id).await?;
        finish_job(
            &mut tx,
            id,
            "failed",
            Some(error_code),
            Some(error_message),
            now,
        )
        .await?;
        if job.kind == JobKind::Generate {
            bump_batch_counter(&mut tx, &job.batch_id, "failed_jobs", now).await?;
        }
        refresh_batch_status(&mut tx, &job.batch_id, now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// 取消整个批次：排队中和等待重试的任务直接取消，在途的任务让它跑完。返回取消的任务数。
    pub async fn cancel_batch(&self, batch_id: &str, now: i64) -> Result<u64> {
        let mut tx = self.begin_write().await?;
        let cancelled = sqlx::query(
            "UPDATE job SET status = 'cancelled', finished_at = ?1, updated_at = ?1
             WHERE batch_id = ?2 AND status IN ('queued', 'retry_wait')",
        )
        .bind(now)
        .bind(batch_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let updated = sqlx::query(
            "UPDATE batch SET status = 'cancelled', pause_reason = NULL, finished_at = ?1, updated_at = ?1
             WHERE id = ?2 AND status IN ('running', 'paused')",
        )
        .bind(now)
        .bind(batch_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if updated == 0 {
            return Err(StoreError::state_changed("批次", batch_id));
        }
        tx.commit().await?;
        Ok(cancelled)
    }

    /// 取消单个任务（只能取消还没发出去的）。
    pub async fn cancel_job(&self, id: &str, now: i64) -> Result<()> {
        let mut tx = self.begin_write().await?;
        let job = fetch_job(&mut tx, id).await?;
        let result = sqlx::query(
            "UPDATE job SET status = 'cancelled', finished_at = ?1, updated_at = ?1
             WHERE id = ?2 AND status IN ('queued', 'retry_wait')",
        )
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        expect_one(result.rows_affected(), id)?;
        refresh_batch_status(&mut tx, &job.batch_id, now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// 重试批次里所有失败的任务：清空错误和尝试次数，以 `priority` 重新排队，批次恢复运行。
    pub async fn retry_failed_jobs(&self, batch_id: &str, priority: i64, now: i64) -> Result<u64> {
        let mut tx = self.begin_write().await?;
        let failed_generate: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM job WHERE batch_id = ? AND status = 'failed' AND kind = 'generate'",
        )
        .bind(batch_id)
        .fetch_one(&mut *tx)
        .await?;
        let retried = sqlx::query(
            "UPDATE job SET status = 'queued', attempt = 0, priority = ?1, error_code = NULL,
                            error_message = NULL, next_run_at = NULL, remote_task_id = NULL,
                            finished_at = NULL, updated_at = ?2
             WHERE batch_id = ?3 AND status = 'failed'",
        )
        .bind(priority)
        .bind(now)
        .bind(batch_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if retried > 0 {
            sqlx::query(
                "UPDATE batch SET status = 'running', failed_jobs = failed_jobs - ?1,
                                  pause_reason = NULL, finished_at = NULL, updated_at = ?2
                 WHERE id = ?3 AND status <> 'cancelled'",
            )
            .bind(failed_generate)
            .bind(now)
            .bind(batch_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(retried)
    }

    /// 启动时调用一次，见详细设计 2.10：
    /// - 运行中、没有远端任务 ID 的任务：同步请求的结果已经拿不回来，放回队列，计入批次的重新提交数
    /// - 运行中、有远端任务 ID 的任务：改为轮询，继续查结果
    pub async fn recover_after_restart(&self, now: i64) -> Result<RecoveryReport> {
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "UPDATE batch SET resubmitted_jobs = resubmitted_jobs + (
                 SELECT COUNT(*) FROM job
                 WHERE job.batch_id = batch.id AND job.status = 'running' AND job.remote_task_id IS NULL
             ), updated_at = ?
             WHERE id IN (SELECT batch_id FROM job WHERE status = 'running' AND remote_task_id IS NULL)",
        )
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let resubmitted = sqlx::query(
            "UPDATE job SET status = 'queued', updated_at = ?
             WHERE status = 'running' AND remote_task_id IS NULL",
        )
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let resumed_polling = sqlx::query(
            "UPDATE job SET status = 'polling', updated_at = ?
             WHERE status = 'running' AND remote_task_id IS NOT NULL",
        )
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(RecoveryReport {
            resubmitted,
            resumed_polling,
        })
    }

    /// 内容审核拒绝后改写指令并放回队列（详细设计 4.4）。每个任务只能改写一次，
    /// 已经改写过返回 `StateChanged`。这次尝试不退回，审核拒绝也算一次失败。
    pub async fn rewrite_and_requeue(
        &self,
        id: &str,
        instruction: &str,
        request_params: Option<&str>,
        now: i64,
    ) -> Result<()> {
        let result = sqlx::query(
            "UPDATE job SET instruction = ?, request_params = COALESCE(?, request_params),
                            rewritten = 1, status = 'queued', remote_task_id = NULL, updated_at = ?
             WHERE id = ? AND rewritten = 0 AND status IN ('running', 'polling')",
        )
        .bind(instruction)
        .bind(request_params)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        expect_one(result.rows_affected(), id)
    }

    /// 最近一个到期的重试时间，调度器据此设置下一次醒来的时间。
    pub async fn next_retry_at(&self) -> Result<Option<i64>> {
        let next = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT MIN(j.next_run_at) FROM job j JOIN batch b ON b.id = j.batch_id
             WHERE j.status = 'retry_wait' AND b.status = 'running'",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(next)
    }

    /// 正在轮询的任务，最久没更新的排前面。
    pub async fn polling_jobs(&self, limit: i64) -> Result<Vec<JobRecord>> {
        let rows = sqlx::query_as::<_, JobRecord>(
            "SELECT * FROM job WHERE status = 'polling' ORDER BY updated_at LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

async fn fetch_job(conn: &mut SqliteConnection, id: &str) -> Result<JobRecord> {
    sqlx::query_as::<_, JobRecord>("SELECT * FROM job WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| StoreError::not_found("任务", id))
}

/// 在途任务进入终态。只接受运行中和轮询中的任务。
async fn finish_job(
    conn: &mut SqliteConnection,
    id: &str,
    status: &str,
    error_code: Option<&str>,
    error_message: Option<&str>,
    now: i64,
) -> Result<()> {
    let result = sqlx::query(
        "UPDATE job SET status = ?, error_code = ?, error_message = ?, finished_at = ?, updated_at = ?
         WHERE id = ? AND status IN ('running', 'polling')",
    )
    .bind(status)
    .bind(error_code)
    .bind(error_message)
    .bind(now)
    .bind(now)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    expect_one(result.rows_affected(), id)
}

/// 批次计数加 1。列名只来自本文件里的常量，不会拼接外部输入。
async fn bump_batch_counter(
    conn: &mut SqliteConnection,
    batch_id: &str,
    column: &'static str,
    now: i64,
) -> Result<()> {
    let sql = format!("UPDATE batch SET {column} = {column} + 1, updated_at = ? WHERE id = ?");
    sqlx::query(&sql)
        .bind(now)
        .bind(batch_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn insert_job_rows(
    conn: &mut SqliteConnection,
    jobs: &[NewJob],
    now: i64,
) -> Result<Vec<String>> {
    let mut ids = Vec::with_capacity(jobs.len());
    for job in jobs {
        let id = new_id();
        // batch_seq、source_seq 从批次和原图上冗余过来，调度排序要用；
        // 没有原图的任务（文案等）source_seq 为 0。原图必须属于同一个批次
        let result = sqlx::query(
            "INSERT INTO job (id, batch_id, batch_seq, source_id, source_seq, kind, task_type,
                              product_id, variant_index, axis, instruction, priority, status,
                              max_attempts, request_params, created_at, updated_at)
             SELECT ?1, b.id, b.seq, s.id, COALESCE(s.seq, 0), ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                    'queued', ?9, ?10, ?11, ?11
             FROM batch b LEFT JOIN source_image s ON s.id = ?13 AND s.batch_id = b.id
             WHERE b.id = ?12 AND (?13 IS NULL OR s.id IS NOT NULL)",
        )
        .bind(&id)
        .bind(job.kind.as_str())
        .bind(&job.task_type)
        .bind(&job.product_id)
        .bind(job.variant_index)
        .bind(&job.axis)
        .bind(&job.instruction)
        .bind(job.priority)
        .bind(job.max_attempts)
        .bind(&job.request_params)
        .bind(now)
        .bind(&job.batch_id)
        .bind(&job.source_id)
        .execute(&mut *conn)
        .await?;
        if result.rows_affected() == 0 {
            let missing = job.source_id.as_deref().unwrap_or(&job.batch_id);
            return Err(StoreError::not_found("批次或原图", missing));
        }
        if job.kind == JobKind::Generate {
            sqlx::query(
                "UPDATE batch SET total_jobs = total_jobs + 1, updated_at = ? WHERE id = ?",
            )
            .bind(now)
            .bind(&job.batch_id)
            .execute(&mut *conn)
            .await?;
        }
        ids.push(id);
    }
    Ok(ids)
}

fn expect_one(rows_affected: u64, id: &str) -> Result<()> {
    if rows_affected == 0 {
        return Err(StoreError::state_changed("任务", id));
    }
    Ok(())
}
