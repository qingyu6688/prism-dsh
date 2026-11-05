//! `api_profile` 表：API 配置。

use prism_core::new_id;

use crate::{ApiProfileRecord, NewApiProfile, Result, Store, StoreError};

impl Store {
    /// 原子保存一份连接及可选的生图子配置，旧的单用途配置继续使用原 ID。
    pub async fn save_profile_connection(
        &self,
        id: &str,
        profile: &NewApiProfile,
        image_model: Option<&str>,
        now: i64,
    ) -> Result<ApiProfileRecord> {
        let mut tx = self.begin_write().await?;
        if image_model.is_some() && profile.purpose != "text" {
            return Err(StoreError::Invalid("共享生图模型需要文本主配置".into()));
        }
        let child: Option<String> =
            sqlx::query_scalar("SELECT id FROM api_profile WHERE credential_owner_id = ?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        upsert_connection(&mut tx, id, profile, None, now).await?;
        if let Some(model) = image_model {
            let image = NewApiProfile {
                purpose: "image".into(),
                provider_type: "openai_images".into(),
                model: model.into(),
                timeout_secs: profile.timeout_secs.max(600),
                ..profile.clone()
            };
            upsert_connection(
                &mut tx,
                &child.unwrap_or_else(new_id),
                &image,
                Some(id),
                now,
            )
            .await?;
        } else if let Some(child) = child {
            sqlx::query("DELETE FROM api_profile WHERE id = ?")
                .bind(child)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        self.get_profile(id).await
    }

    /// 新建配置，返回完整记录。排序号放到同用途配置的最后。
    pub async fn create_profile(
        &self,
        profile: &NewApiProfile,
        now: i64,
    ) -> Result<ApiProfileRecord> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO api_profile (
                id, name, purpose, access_type, provider_type, base_url, model, model_is_custom,
                max_concurrency, rpm_limit, weight, timeout_secs, transparent_method,
                capabilities, custom_spec, sort_order, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM api_profile WHERE purpose = ?), ?, ?)",
        )
        .bind(&id)
        .bind(&profile.name)
        .bind(&profile.purpose)
        .bind(&profile.access_type)
        .bind(&profile.provider_type)
        .bind(&profile.base_url)
        .bind(&profile.model)
        .bind(profile.model_is_custom)
        .bind(profile.max_concurrency)
        .bind(profile.rpm_limit)
        .bind(profile.weight)
        .bind(profile.timeout_secs)
        .bind(&profile.transparent_method)
        .bind(&profile.capabilities)
        .bind(&profile.custom_spec)
        .bind(&profile.purpose)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.get_profile(&id).await
    }

    /// 整体更新配置的可编辑字段。
    pub async fn update_profile(
        &self,
        id: &str,
        profile: &NewApiProfile,
        now: i64,
    ) -> Result<ApiProfileRecord> {
        let result = sqlx::query(
            "UPDATE api_profile SET
                name = ?, purpose = ?, access_type = ?, provider_type = ?, base_url = ?, model = ?,
                model_is_custom = ?, max_concurrency = ?, rpm_limit = ?, weight = ?, timeout_secs = ?,
                transparent_method = ?, capabilities = ?, custom_spec = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&profile.name)
        .bind(&profile.purpose)
        .bind(&profile.access_type)
        .bind(&profile.provider_type)
        .bind(&profile.base_url)
        .bind(&profile.model)
        .bind(profile.model_is_custom)
        .bind(profile.max_concurrency)
        .bind(profile.rpm_limit)
        .bind(profile.weight)
        .bind(profile.timeout_secs)
        .bind(&profile.transparent_method)
        .bind(&profile.capabilities)
        .bind(&profile.custom_spec)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::not_found("API 配置", id));
        }
        self.get_profile(id).await
    }

    pub async fn get_profile(&self, id: &str) -> Result<ApiProfileRecord> {
        sqlx::query_as::<_, ApiProfileRecord>("SELECT * FROM api_profile WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::not_found("API 配置", id))
    }

    /// 按用途列出配置（`image` / `text`），`None` 列出全部；按排序号排列。
    pub async fn list_profiles(&self, purpose: Option<&str>) -> Result<Vec<ApiProfileRecord>> {
        let rows = sqlx::query_as::<_, ApiProfileRecord>(
            "SELECT * FROM api_profile WHERE (?1 IS NULL OR purpose = ?1)
             ORDER BY purpose, sort_order, created_at",
        )
        .bind(purpose)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 启用或停用配置。引擎遇到 401 / 403 时用它自动停用，并写明原因。
    pub async fn set_profile_enabled(
        &self,
        id: &str,
        enabled: bool,
        reason: Option<&str>,
        now: i64,
    ) -> Result<()> {
        let result = sqlx::query(
            "UPDATE api_profile SET enabled = ?, disabled_reason = ?, updated_at = ? WHERE id = ?",
        )
        .bind(enabled)
        .bind(if enabled { None } else { reason })
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::not_found("API 配置", id));
        }
        Ok(())
    }

    /// 保存从 `/models` 拉到的模型列表（JSON 数组字符串）。
    pub async fn save_models_cache(&self, id: &str, models_json: &str, now: i64) -> Result<()> {
        sqlx::query(
            "UPDATE api_profile SET models_cache = ?, models_fetched_at = ?, updated_at = ? WHERE id = ? OR credential_owner_id = ?",
        )
        .bind(models_json)
        .bind(now)
        .bind(now)
        .bind(id)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 删除配置。引用它的任务的 `profile_id` 会被置空。Key 由调用方从凭据管理器里删。
    pub async fn delete_profile(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM api_profile WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

async fn upsert_connection(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
    profile: &NewApiProfile,
    owner: Option<&str>,
    now: i64,
) -> Result<()> {
    sqlx::query("INSERT INTO api_profile (
        id, name, purpose, access_type, provider_type, base_url, model, model_is_custom,
        max_concurrency, rpm_limit, weight, timeout_secs, transparent_method, capabilities, custom_spec,
        credential_owner_id, sort_order, created_at, updated_at
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
        (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM api_profile WHERE purpose = ?), ?, ?)
    ON CONFLICT(id) DO UPDATE SET
        name=excluded.name, purpose=excluded.purpose, access_type=excluded.access_type,
        provider_type=excluded.provider_type, base_url=excluded.base_url, model=excluded.model,
        model_is_custom=excluded.model_is_custom, max_concurrency=excluded.max_concurrency,
        rpm_limit=excluded.rpm_limit, timeout_secs=excluded.timeout_secs, updated_at=excluded.updated_at,
        credential_owner_id=excluded.credential_owner_id, models_cache=NULL, models_fetched_at=NULL,
        enabled=1, disabled_reason=NULL")
        .bind(id).bind(&profile.name).bind(&profile.purpose).bind(&profile.access_type)
        .bind(&profile.provider_type).bind(&profile.base_url).bind(&profile.model).bind(profile.model_is_custom)
        .bind(profile.max_concurrency).bind(profile.rpm_limit).bind(profile.weight).bind(profile.timeout_secs)
        .bind(&profile.transparent_method).bind(&profile.capabilities).bind(&profile.custom_spec)
        .bind(owner).bind(&profile.purpose).bind(now).bind(now)
        .execute(&mut **tx).await?;
    Ok(())
}
