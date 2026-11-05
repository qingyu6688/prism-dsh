//! `import_mapping` 表：保存的表格列映射（迁移 0003）。

use prism_core::new_id;

use crate::{Result, Store, StoreError};

/// `import_mapping` 表的一行。
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ImportMappingRecord {
    pub id: String,
    /// 导入到哪里，如 `product`
    pub target: String,
    pub name: String,
    /// JSON 数组：保存时的表头
    pub headers: String,
    /// JSON 对象：字段键 → 表头
    pub columns: String,
    pub header_line: Option<i64>,
    pub use_count: i64,
    pub last_used_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewImportMapping {
    pub target: String,
    pub name: String,
    pub headers: String,
    pub columns: String,
    pub header_line: Option<i64>,
}

impl Store {
    /// 保存映射。同一个 `target` 下已有表头完全相同的，覆盖那条，不重复存。
    pub async fn save_import_mapping(
        &self,
        mapping: &NewImportMapping,
        now: i64,
    ) -> Result<ImportMappingRecord> {
        let mut tx = self.begin_write().await?;
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM import_mapping WHERE target = ? AND headers = ? LIMIT 1",
        )
        .bind(&mapping.target)
        .bind(&mapping.headers)
        .fetch_optional(&mut *tx)
        .await?;
        let id = match existing {
            Some(id) => {
                sqlx::query(
                    "UPDATE import_mapping SET name = ?, columns = ?, header_line = ?, updated_at = ?
                     WHERE id = ?",
                )
                .bind(&mapping.name)
                .bind(&mapping.columns)
                .bind(mapping.header_line)
                .bind(now)
                .bind(&id)
                .execute(&mut *tx)
                .await?;
                id
            }
            None => {
                let id = new_id();
                sqlx::query(
                    "INSERT INTO import_mapping
                         (id, target, name, headers, columns, header_line, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(&mapping.target)
                .bind(&mapping.name)
                .bind(&mapping.headers)
                .bind(&mapping.columns)
                .bind(mapping.header_line)
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                id
            }
        };
        tx.commit().await?;
        self.get_import_mapping(&id).await
    }

    pub async fn get_import_mapping(&self, id: &str) -> Result<ImportMappingRecord> {
        sqlx::query_as::<_, ImportMappingRecord>("SELECT * FROM import_mapping WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::not_found("列映射", id))
    }

    /// 某类导入保存过的映射，用得多的、最近用过的在前。
    pub async fn list_import_mappings(&self, target: &str) -> Result<Vec<ImportMappingRecord>> {
        let rows = sqlx::query_as::<_, ImportMappingRecord>(
            "SELECT * FROM import_mapping WHERE target = ?
             ORDER BY use_count DESC, last_used_at DESC, created_at DESC",
        )
        .bind(target)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 记一次套用。
    pub async fn mark_import_mapping_used(&self, id: &str, now: i64) -> Result<()> {
        let result = sqlx::query(
            "UPDATE import_mapping SET use_count = use_count + 1, last_used_at = ? WHERE id = ?",
        )
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::not_found("列映射", id));
        }
        Ok(())
    }

    pub async fn delete_import_mapping(&self, id: &str) -> Result<()> {
        let result = sqlx::query("DELETE FROM import_mapping WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::not_found("列映射", id));
        }
        Ok(())
    }
}
