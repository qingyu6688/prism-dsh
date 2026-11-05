//! 通知中心（迁移 0005，`database.md` 第 9 章）。

use prism_core::new_id;

use crate::{Result, Store};

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct NotificationRecord {
    pub id: String,
    pub conversation_id: String,
    /// 会话标题，列表里显示
    pub conversation_title: String,
    pub plan_id: Option<String>,
    pub kind: String,
    pub text: String,
    pub read_at: Option<i64>,
    pub created_at: i64,
}

impl Store {
    /// 记一条通知。同一计划的同一种通知只记一次，重复的返回 `false`。
    /// `read` 为真时直接记为已读（通知产生时用户正在看这个会话）。
    pub async fn add_notification(
        &self,
        conversation_id: &str,
        plan_id: Option<&str>,
        kind: &str,
        text: &str,
        read: bool,
        now: i64,
    ) -> Result<bool> {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO notification
                 (id, conversation_id, plan_id, kind, text, read_at, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new_id())
        .bind(conversation_id)
        .bind(plan_id)
        .bind(kind)
        .bind(text)
        .bind(read.then_some(now))
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// 最近的通知，新的在前，带会话标题。
    pub async fn list_notifications(&self, limit: i64) -> Result<Vec<NotificationRecord>> {
        let rows = sqlx::query_as::<_, NotificationRecord>(
            "SELECT n.id, n.conversation_id, c.title AS conversation_title, n.plan_id, n.kind,
                    n.text, n.read_at, n.created_at
             FROM notification n JOIN conversation c ON c.id = n.conversation_id
             ORDER BY n.created_at DESC, n.rowid DESC
             LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn unread_notifications(&self) -> Result<i64> {
        let count = sqlx::query_scalar("SELECT COUNT(*) FROM notification WHERE read_at IS NULL")
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }

    /// 标为已读：给了 ID 只标这些，没给标全部。返回改了几条。
    pub async fn mark_notifications_read(&self, ids: Option<&[String]>, now: i64) -> Result<u64> {
        let result = match ids {
            None => {
                sqlx::query("UPDATE notification SET read_at = ? WHERE read_at IS NULL")
                    .bind(now)
                    .execute(&self.pool)
                    .await?
            }
            Some([]) => return Ok(0),
            Some(ids) => {
                let placeholders = vec!["?"; ids.len()].join(", ");
                let sql = format!(
                    "UPDATE notification SET read_at = ? WHERE read_at IS NULL AND id IN ({placeholders})"
                );
                let mut query = sqlx::query(&sql).bind(now);
                for id in ids {
                    query = query.bind(id);
                }
                query.execute(&self.pool).await?
            }
        };
        Ok(result.rows_affected())
    }

    /// 打开会话时，这个会话的通知都标为已读。返回改了几条。
    pub async fn mark_conversation_read(&self, conversation_id: &str, now: i64) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE notification SET read_at = ? WHERE read_at IS NULL AND conversation_id = ?",
        )
        .bind(now)
        .bind(conversation_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}
