//! `setting` 键值表。值是 JSON 字符串，由调用方序列化。

use crate::{Result, Store};

impl Store {
    /// 读取一个设置项，没有时返回 `None`。
    pub async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let value = sqlx::query_scalar::<_, String>("SELECT value FROM setting WHERE id = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(value)
    }

    /// 写入一个设置项，已存在就覆盖。
    pub async fn set_setting(&self, key: &str, value: &str, now: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO setting (id, value, created_at, updated_at) VALUES (?, ?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(value)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
