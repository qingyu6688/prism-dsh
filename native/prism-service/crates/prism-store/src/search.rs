//! 全局搜索用的查询（接口设计 1.10）。商品、素材的搜索复用各自列表的筛选。

use crate::catalog::like_pattern;
use crate::{BatchRecord, ConversationRecord, Result, Store};

impl Store {
    /// 标题或消息内容里有关键词的会话，最近有动静的在前。关键词为空返回空列表。
    pub async fn search_conversations(
        &self,
        keyword: &str,
        limit: i64,
    ) -> Result<Vec<ConversationRecord>> {
        let Some(pattern) = like_pattern(Some(keyword)) else {
            return Ok(Vec::new());
        };
        let rows = sqlx::query_as::<_, ConversationRecord>(
            "SELECT c.* FROM conversation c
             WHERE c.title LIKE ?1 ESCAPE '\\'
                OR EXISTS (SELECT 1 FROM message m
                           WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
                             AND m.content LIKE ?1 ESCAPE '\\')
             ORDER BY c.updated_at DESC, c.id
             LIMIT ?2",
        )
        .bind(pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 按批次号找批次，没有返回 `None`。
    pub async fn batch_by_seq(&self, seq: i64) -> Result<Option<BatchRecord>> {
        let row = sqlx::query_as::<_, BatchRecord>("SELECT * FROM batch WHERE seq = ?")
            .bind(seq)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }
}
