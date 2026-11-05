//! 会话、消息（迁移 0001）和 Agent 步骤计划（迁移 0004）。

use prism_core::new_id;
use sqlx::SqliteConnection;

use crate::{Result, Store, StoreError};

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ConversationRecord {
    pub id: String,
    pub title: String,
    pub next_source_no: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct MessageRecord {
    pub id: String,
    pub conversation_id: String,
    /// `user` / `assistant` / `tool` / `event`
    pub role: String,
    /// JSON，格式见详细设计 5.9
    pub content: String,
    /// `done` / `pending_confirm` / `confirmed` / `rejected`
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct PlanRecord {
    pub id: String,
    pub conversation_id: String,
    pub message_id: Option<String>,
    pub tool_call_id: String,
    pub status: String,
    pub estimated_jobs: i64,
    pub finished_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StepRecord {
    pub id: String,
    pub plan_id: String,
    pub step_key: String,
    pub seq: i64,
    pub tool: String,
    pub title: String,
    pub arguments: String,
    /// JSON 数组：前置步骤的 `step_key`
    pub depends_on: String,
    pub status: String,
    pub estimated_jobs: i64,
    pub handle: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPlan {
    pub conversation_id: String,
    pub message_id: Option<String>,
    pub tool_call_id: String,
    pub status: String,
    pub estimated_jobs: i64,
    pub steps: Vec<NewStep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewStep {
    pub step_key: String,
    pub tool: String,
    pub title: String,
    pub arguments: String,
    pub depends_on: String,
    pub estimated_jobs: i64,
}

/// 计划结束的状态。
const PLAN_FINISHED: &[&str] = &["succeeded", "partial", "failed", "cancelled"];

// ---------- 会话和消息 ----------

impl Store {
    pub async fn create_conversation(&self, title: &str, now: i64) -> Result<ConversationRecord> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO conversation (id, title, created_at, updated_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(title)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.get_conversation(&id).await
    }

    pub async fn get_conversation(&self, id: &str) -> Result<ConversationRecord> {
        sqlx::query_as::<_, ConversationRecord>("SELECT * FROM conversation WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::not_found("会话", id))
    }

    /// 会话列表，最近有动静的在前。
    pub async fn list_conversations(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ConversationRecord>> {
        let rows = sqlx::query_as::<_, ConversationRecord>(
            "SELECT * FROM conversation ORDER BY updated_at DESC, id LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn rename_conversation(&self, id: &str, title: &str, now: i64) -> Result<()> {
        let result = sqlx::query("UPDATE conversation SET title = ?, updated_at = ? WHERE id = ?")
            .bind(title)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::not_found("会话", id));
        }
        Ok(())
    }

    /// 删除会话：消息、计划级联删除；批次只是解除关联，产出的素材保留。
    pub async fn delete_conversation(&self, id: &str) -> Result<()> {
        let result = sqlx::query("DELETE FROM conversation WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::not_found("会话", id));
        }
        Ok(())
    }

    /// 从某条用户消息重来（详细设计 5.10）：一个事务里删掉这条和之后的消息、之后建的计划。
    /// 之后有计划在执行时拒绝，什么都不删。
    pub async fn rewind_from(&self, conversation_id: &str, message_id: &str) -> Result<()> {
        let mut tx = self.begin_write().await?;
        let target: Option<(i64, i64, String)> = sqlx::query_as(
            "SELECT rowid, created_at, role FROM message WHERE id = ? AND conversation_id = ?",
        )
        .bind(message_id)
        .bind(conversation_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((rowid, created_at, role)) = target else {
            return Err(StoreError::not_found("消息", message_id));
        };
        if role != "user" {
            return Err(StoreError::Invalid("只能修改自己发的消息".into()));
        }
        let running: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_plan
             WHERE conversation_id = ? AND created_at >= ? AND status = 'running'",
        )
        .bind(conversation_id)
        .bind(created_at)
        .fetch_one(&mut *tx)
        .await?;
        if running > 0 {
            return Err(StoreError::Invalid(
                "这条消息之后有计划正在执行，先取消计划再修改".into(),
            ));
        }
        sqlx::query("DELETE FROM agent_plan WHERE conversation_id = ? AND created_at >= ?")
            .bind(conversation_id)
            .bind(created_at)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM message
             WHERE conversation_id = ?1 AND (created_at > ?2 OR (created_at = ?2 AND rowid >= ?3))",
        )
        .bind(conversation_id)
        .bind(created_at)
        .bind(rowid)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// 这个会话有没有正在执行的计划，删除会话前检查。
    pub async fn has_running_plan(&self, conversation_id: &str) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_plan WHERE conversation_id = ? AND status = 'running'",
        )
        .bind(conversation_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    /// 追加一条消息，同时更新会话的最后活动时间。
    pub async fn append_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        status: &str,
        now: i64,
    ) -> Result<MessageRecord> {
        let id = new_id();
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "INSERT INTO message (id, conversation_id, role, content, status, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(conversation_id)
        .bind(role)
        .bind(content)
        .bind(status)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| foreign_key(error, "会话", conversation_id))?;
        sqlx::query("UPDATE conversation SET updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(conversation_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.get_message(&id).await
    }

    pub async fn get_message(&self, id: &str) -> Result<MessageRecord> {
        sqlx::query_as::<_, MessageRecord>("SELECT * FROM message WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::not_found("消息", id))
    }

    /// 最近的 `limit` 条消息，按时间先后排。同一毫秒的按插入顺序。
    pub async fn recent_messages(
        &self,
        conversation_id: &str,
        limit: i64,
    ) -> Result<Vec<MessageRecord>> {
        let mut rows = sqlx::query_as::<_, MessageRecord>(
            "SELECT * FROM message WHERE conversation_id = ?
             ORDER BY created_at DESC, rowid DESC LIMIT ?",
        )
        .bind(conversation_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.reverse();
        Ok(rows)
    }

    pub async fn set_message_status(&self, id: &str, status: &str, now: i64) -> Result<()> {
        sqlx::query("UPDATE message SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

/// 外键约束失败多半是引用的对象不存在，转成"找不到"。
fn foreign_key(error: sqlx::Error, entity: &'static str, id: &str) -> StoreError {
    match &error {
        sqlx::Error::Database(db) if db.message().contains("FOREIGN KEY") => {
            StoreError::not_found(entity, id)
        }
        _ => error.into(),
    }
}

// ---------- 计划和步骤 ----------

impl Store {
    /// 新建计划和全部步骤，在一个事务里。步骤初始都是 `pending`。
    pub async fn create_plan(
        &self,
        plan: &NewPlan,
        now: i64,
    ) -> Result<(PlanRecord, Vec<StepRecord>)> {
        let id = new_id();
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "INSERT INTO agent_plan
                 (id, conversation_id, message_id, tool_call_id, status, estimated_jobs, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&plan.conversation_id)
        .bind(&plan.message_id)
        .bind(&plan.tool_call_id)
        .bind(&plan.status)
        .bind(plan.estimated_jobs)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| foreign_key(error, "会话", &plan.conversation_id))?;
        for (seq, step) in plan.steps.iter().enumerate() {
            insert_step(
                &mut tx,
                &id,
                i64::try_from(seq).unwrap_or(i64::MAX),
                step,
                now,
            )
            .await?;
        }
        tx.commit().await?;
        Ok((self.get_plan(&id).await?, self.plan_steps(&id).await?))
    }

    pub async fn get_plan(&self, id: &str) -> Result<PlanRecord> {
        sqlx::query_as::<_, PlanRecord>("SELECT * FROM agent_plan WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::not_found("计划", id))
    }

    pub async fn plan_steps(&self, plan_id: &str) -> Result<Vec<StepRecord>> {
        let rows = sqlx::query_as::<_, StepRecord>(
            "SELECT * FROM agent_step WHERE plan_id = ? ORDER BY seq",
        )
        .bind(plan_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_step(&self, id: &str) -> Result<StepRecord> {
        sqlx::query_as::<_, StepRecord>("SELECT * FROM agent_step WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::not_found("步骤", id))
    }

    /// 某条助手消息（确认卡片）对应的计划，界面显示卡片时用。
    pub async fn plan_for_message(&self, message_id: &str) -> Result<Option<PlanRecord>> {
        let row = sqlx::query_as::<_, PlanRecord>(
            "SELECT * FROM agent_plan WHERE message_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// 某个会话里指定状态的计划，最新的在前。
    pub async fn plans_with_status(
        &self,
        conversation_id: &str,
        status: &str,
    ) -> Result<Vec<PlanRecord>> {
        let rows = sqlx::query_as::<_, PlanRecord>(
            "SELECT * FROM agent_plan WHERE conversation_id = ? AND status = ? ORDER BY created_at DESC",
        )
        .bind(conversation_id)
        .bind(status)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 所有正在执行的计划，启动恢复用。
    pub async fn running_plans(&self) -> Result<Vec<PlanRecord>> {
        let rows = sqlx::query_as::<_, PlanRecord>(
            "SELECT * FROM agent_plan WHERE status = 'running' ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 改计划状态；改成结束状态时记下结束时间。`from` 不为空时只在当前状态是它时才改，返回是否改了。
    pub async fn set_plan_status(
        &self,
        id: &str,
        from: Option<&str>,
        status: &str,
        now: i64,
    ) -> Result<bool> {
        let finished = PLAN_FINISHED.contains(&status).then_some(now);
        let result = sqlx::query(
            "UPDATE agent_plan SET status = ?1, finished_at = COALESCE(?2, finished_at), updated_at = ?3
             WHERE id = ?4 AND (?5 IS NULL OR status = ?5)",
        )
        .bind(status)
        .bind(finished)
        .bind(now)
        .bind(id)
        .bind(from)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// `pending` → `running`。已经不是 `pending`（比如被取消了）返回 `false`。
    pub async fn start_step(&self, id: &str, handle: Option<&str>, now: i64) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE agent_step SET status = 'running', handle = ?, started_at = ?, updated_at = ?
             WHERE id = ? AND status = 'pending'",
        )
        .bind(handle)
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// 给运行中的步骤记上句柄（工具开始后才知道批次 ID）。
    pub async fn set_step_handle(&self, id: &str, handle: &str, now: i64) -> Result<()> {
        sqlx::query(
            "UPDATE agent_step SET handle = ?, updated_at = ? WHERE id = ? AND status = 'running'",
        )
        .bind(handle)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 结束一步：只有 `pending` / `running` 的步骤能结束，返回是否改了（重复通知时为 `false`）。
    pub async fn finish_step(
        &self,
        id: &str,
        status: &str,
        output: Option<&str>,
        error: Option<&str>,
        now: i64,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE agent_step SET status = ?, output = ?, error = ?, finished_at = ?, updated_at = ?
             WHERE id = ? AND status IN ('pending', 'running')",
        )
        .bind(status)
        .bind(output)
        .bind(error)
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// 按句柄找运行中的步骤（模块通知完成时用）。
    pub async fn find_running_step_by_handle(&self, handle: &str) -> Result<Option<StepRecord>> {
        let row = sqlx::query_as::<_, StepRecord>(
            "SELECT * FROM agent_step WHERE handle = ? AND status = 'running' LIMIT 1",
        )
        .bind(handle)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// 所有运行中的步骤，模块启动时核对用。
    pub async fn running_steps(&self) -> Result<Vec<StepRecord>> {
        let rows = sqlx::query_as::<_, StepRecord>(
            "SELECT * FROM agent_step WHERE status = 'running' ORDER BY created_at, seq",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

async fn insert_step(
    tx: &mut SqliteConnection,
    plan_id: &str,
    seq: i64,
    step: &NewStep,
    now: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO agent_step
             (id, plan_id, step_key, seq, tool, title, arguments, depends_on, status, estimated_jobs,
              created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?)",
    )
    .bind(new_id())
    .bind(plan_id)
    .bind(&step.step_key)
    .bind(seq)
    .bind(&step.tool)
    .bind(&step.title)
    .bind(&step.arguments)
    .bind(&step.depends_on)
    .bind(step.estimated_jobs)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    Ok(())
}
