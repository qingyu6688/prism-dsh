//! 通用平台的表：店铺、商品、规格、素材（含标签）、文案。表结构见 `docs/database.md` 第 6 章。
//!
//! JSON 字段（卖点、参数、规格、素材元信息）在这一层保持字符串，由 `prism-catalog` 负责解析和校验。

use prism_core::new_id;

use crate::{Result, Store, StoreError};

// ---------- 记录 ----------

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ShopRecord {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub site: Option<String>,
    pub language: String,
    pub currency: String,
    pub notes: Option<String>,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewShop {
    pub name: String,
    pub platform: String,
    pub site: Option<String>,
    pub language: String,
    pub currency: String,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ProductRecord {
    pub id: String,
    pub code: Option<String>,
    pub name: String,
    pub category: Option<String>,
    pub brand: Option<String>,
    pub description: Option<String>,
    /// JSON 数组
    pub selling_points: Option<String>,
    /// JSON 对象
    pub attributes: Option<String>,
    pub cost_cents: Option<i64>,
    pub currency: String,
    /// JSON 数组
    pub target_markets: Option<String>,
    /// `active` / `archived`
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NewProduct {
    pub code: Option<String>,
    pub name: String,
    pub category: Option<String>,
    pub brand: Option<String>,
    pub description: Option<String>,
    pub selling_points: Option<String>,
    pub attributes: Option<String>,
    pub cost_cents: Option<i64>,
    pub currency: String,
    pub target_markets: Option<String>,
}

/// 商品列表的筛选条件。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProductQuery {
    /// 按名称、编码、类目、品牌模糊搜索
    pub keyword: Option<String>,
    /// `None` 只看在售（`active`）
    pub status: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct SkuRecord {
    pub id: String,
    pub product_id: String,
    pub code: Option<String>,
    /// JSON 对象
    pub spec: String,
    pub price_cents: Option<i64>,
    pub cost_cents: Option<i64>,
    pub stock: Option<i64>,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewSku {
    pub code: Option<String>,
    pub spec: String,
    pub price_cents: Option<i64>,
    pub cost_cents: Option<i64>,
    pub stock: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct AssetRecord {
    pub id: String,
    /// `image` / `text`
    pub kind: String,
    pub product_id: Option<String>,
    /// `upload` / `generated` / `captured` / `imported`
    pub source: String,
    pub title: Option<String>,
    pub file_path: Option<String>,
    pub thumb_path: Option<String>,
    pub text_content: Option<String>,
    pub mime: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub sha256: Option<String>,
    pub phash: Option<String>,
    pub origin_batch_id: Option<String>,
    pub origin_job_id: Option<String>,
    /// JSON 对象
    pub meta: Option<String>,
    pub starred: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NewAsset {
    pub kind: String,
    pub product_id: Option<String>,
    pub source: String,
    pub title: Option<String>,
    pub file_path: Option<String>,
    pub thumb_path: Option<String>,
    pub text_content: Option<String>,
    pub mime: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub sha256: Option<String>,
    pub phash: Option<String>,
    pub origin_batch_id: Option<String>,
    pub origin_job_id: Option<String>,
    pub meta: Option<String>,
}

/// 素材列表的筛选条件，条件之间是"并且"。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AssetQuery {
    pub product_id: Option<String>,
    pub kind: Option<String>,
    pub source: Option<String>,
    pub tag: Option<String>,
    /// 在标题、文案内容、标签里搜索，为空不筛
    pub keyword: Option<String>,
    pub starred_only: bool,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct CopyRecord {
    pub id: String,
    pub product_id: String,
    pub shop_id: Option<String>,
    pub language: String,
    /// `title` / `bullets` / `description` / `keywords` / `attributes` / `selling_points`
    pub field: String,
    pub content: String,
    /// `draft` / `approved`
    pub status: String,
    pub origin_job_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewCopy {
    pub product_id: String,
    pub shop_id: Option<String>,
    pub language: String,
    pub field: String,
    pub content: String,
    pub origin_job_id: Option<String>,
}

// ---------- 店铺 ----------

impl Store {
    pub async fn create_shop(&self, shop: &NewShop, now: i64) -> Result<ShopRecord> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO shop (id, name, platform, site, language, currency, notes, sort_order,
                               created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM shop),
                     ?8, ?8)",
        )
        .bind(&id)
        .bind(&shop.name)
        .bind(&shop.platform)
        .bind(&shop.site)
        .bind(&shop.language)
        .bind(&shop.currency)
        .bind(&shop.notes)
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.get_shop(&id).await
    }

    pub async fn update_shop(&self, id: &str, shop: &NewShop, now: i64) -> Result<ShopRecord> {
        let result = sqlx::query(
            "UPDATE shop SET name = ?, platform = ?, site = ?, language = ?, currency = ?, notes = ?,
                             updated_at = ?
             WHERE id = ?",
        )
        .bind(&shop.name)
        .bind(&shop.platform)
        .bind(&shop.site)
        .bind(&shop.language)
        .bind(&shop.currency)
        .bind(&shop.notes)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::not_found("店铺", id));
        }
        self.get_shop(id).await
    }

    pub async fn get_shop(&self, id: &str) -> Result<ShopRecord> {
        sqlx::query_as::<_, ShopRecord>("SELECT * FROM shop WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::not_found("店铺", id))
    }

    /// 列出店铺，`platform` 为空时列出全部。
    pub async fn list_shops(&self, platform: Option<&str>) -> Result<Vec<ShopRecord>> {
        let rows = sqlx::query_as::<_, ShopRecord>(
            "SELECT * FROM shop WHERE (?1 IS NULL OR platform = ?1) ORDER BY sort_order, created_at",
        )
        .bind(platform)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 删除店铺。这家店的文案保留，只是不再关联店铺。
    pub async fn delete_shop(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM shop WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

// ---------- 商品和规格 ----------

impl Store {
    pub async fn create_product(&self, product: &NewProduct, now: i64) -> Result<ProductRecord> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO product (id, code, name, category, brand, description, selling_points,
                                  attributes, cost_cents, currency, target_markets, created_at,
                                  updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
        )
        .bind(&id)
        .bind(&product.code)
        .bind(&product.name)
        .bind(&product.category)
        .bind(&product.brand)
        .bind(&product.description)
        .bind(&product.selling_points)
        .bind(&product.attributes)
        .bind(product.cost_cents)
        .bind(&product.currency)
        .bind(&product.target_markets)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| duplicate_code(e, product.code.as_deref()))?;
        self.get_product(&id).await
    }

    pub async fn update_product(
        &self,
        id: &str,
        product: &NewProduct,
        now: i64,
    ) -> Result<ProductRecord> {
        let result = sqlx::query(
            "UPDATE product SET code = ?, name = ?, category = ?, brand = ?, description = ?,
                                selling_points = ?, attributes = ?, cost_cents = ?, currency = ?,
                                target_markets = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&product.code)
        .bind(&product.name)
        .bind(&product.category)
        .bind(&product.brand)
        .bind(&product.description)
        .bind(&product.selling_points)
        .bind(&product.attributes)
        .bind(product.cost_cents)
        .bind(&product.currency)
        .bind(&product.target_markets)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| duplicate_code(e, product.code.as_deref()))?;
        if result.rows_affected() == 0 {
            return Err(StoreError::not_found("商品", id));
        }
        self.get_product(id).await
    }

    pub async fn get_product(&self, id: &str) -> Result<ProductRecord> {
        sqlx::query_as::<_, ProductRecord>("SELECT * FROM product WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::not_found("商品", id))
    }

    /// 按商品编码找商品，表格导入时用来判断"新增还是更新"。
    pub async fn find_product_by_code(&self, code: &str) -> Result<Option<ProductRecord>> {
        let row = sqlx::query_as::<_, ProductRecord>("SELECT * FROM product WHERE code = ?")
            .bind(code)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    /// 商品列表，最近更新的在前。
    pub async fn list_products(&self, query: &ProductQuery) -> Result<Vec<ProductRecord>> {
        let rows = sqlx::query_as::<_, ProductRecord>(&format!(
            "SELECT * FROM product WHERE {PRODUCT_FILTER}
             ORDER BY updated_at DESC, id LIMIT ?3 OFFSET ?4"
        ))
        .bind(query.status.as_deref().unwrap_or("active"))
        .bind(like_pattern(query.keyword.as_deref()))
        .bind(query.limit)
        .bind(query.offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 和 `list_products` 同样条件下的总数，分页用。
    pub async fn count_products(&self, query: &ProductQuery) -> Result<i64> {
        let count = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM product WHERE {PRODUCT_FILTER}"
        ))
        .bind(query.status.as_deref().unwrap_or("active"))
        .bind(like_pattern(query.keyword.as_deref()))
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    /// 归档或恢复商品。归档的商品默认不出现在列表里，数据都保留。
    pub async fn set_product_status(&self, id: &str, status: &str, now: i64) -> Result<()> {
        let result = sqlx::query("UPDATE product SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::not_found("商品", id));
        }
        Ok(())
    }

    /// 删除商品：规格和文案一起删，素材、批次、任务保留但不再关联。
    pub async fn delete_product(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM product WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// 整体替换商品的规格表（界面上一般是整张表一起编辑保存）。
    pub async fn replace_skus(
        &self,
        product_id: &str,
        skus: &[NewSku],
        now: i64,
    ) -> Result<Vec<SkuRecord>> {
        let mut tx = self.begin_write().await?;
        let exists: Option<String> = sqlx::query_scalar("SELECT id FROM product WHERE id = ?")
            .bind(product_id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            return Err(StoreError::not_found("商品", product_id));
        }
        sqlx::query("DELETE FROM product_sku WHERE product_id = ?")
            .bind(product_id)
            .execute(&mut *tx)
            .await?;
        for (index, sku) in skus.iter().enumerate() {
            sqlx::query(
                "INSERT INTO product_sku (id, product_id, code, spec, price_cents, cost_cents, stock,
                                          sort_order, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            )
            .bind(new_id())
            .bind(product_id)
            .bind(&sku.code)
            .bind(&sku.spec)
            .bind(sku.price_cents)
            .bind(sku.cost_cents)
            .bind(sku.stock)
            .bind(i64::try_from(index).unwrap_or(i64::MAX))
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.list_skus(product_id).await
    }

    pub async fn list_skus(&self, product_id: &str) -> Result<Vec<SkuRecord>> {
        let rows = sqlx::query_as::<_, SkuRecord>(
            "SELECT * FROM product_sku WHERE product_id = ? ORDER BY sort_order",
        )
        .bind(product_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

/// 商品列表的筛选：?1 状态，?2 关键词（LIKE 模式，为空不筛）。
/// 素材筛选条件，参数 ?1～?5：商品、类型、来源、标签、只看收藏。
const ASSET_FILTER: &str = "(?1 IS NULL OR a.product_id = ?1)
    AND (?2 IS NULL OR a.kind = ?2)
    AND (?3 IS NULL OR a.source = ?3)
    AND (?4 IS NULL OR EXISTS (SELECT 1 FROM asset_tag t WHERE t.asset_id = a.id AND t.tag = ?4))
    AND (?5 = 0 OR a.starred = 1)
    AND (?6 IS NULL OR a.title LIKE ?6 ESCAPE '\\' OR a.text_content LIKE ?6 ESCAPE '\\'
         OR EXISTS (SELECT 1 FROM asset_tag k WHERE k.asset_id = a.id AND k.tag LIKE ?6 ESCAPE '\\'))";

const PRODUCT_FILTER: &str = "status = ?1
    AND (?2 IS NULL OR name LIKE ?2 ESCAPE '\\' OR code LIKE ?2 ESCAPE '\\'
         OR category LIKE ?2 ESCAPE '\\' OR brand LIKE ?2 ESCAPE '\\')";

/// 商品维度的汇总，商品详情页用。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProductSummaryRecord {
    pub sku_count: i64,
    pub image_assets: i64,
    pub text_assets: i64,
    pub starred_assets: i64,
    /// `(语言, 总条数, 已确认条数)`，按语言排序
    pub copies_by_language: Vec<(String, i64, i64)>,
    /// `(模块, 批次数)`，按模块排序
    pub batches_by_module: Vec<(String, i64)>,
    pub last_batch_at: Option<i64>,
}

impl Store {
    /// 商品的汇总：规格数、素材数、各语言文案数、各模块批次数。商品不存在时报"找不到"。
    pub async fn product_summary(&self, product_id: &str) -> Result<ProductSummaryRecord> {
        self.get_product(product_id).await?;
        let sku_count = sqlx::query_scalar("SELECT COUNT(*) FROM product_sku WHERE product_id = ?")
            .bind(product_id)
            .fetch_one(&self.pool)
            .await?;
        let (image_assets, text_assets, starred_assets): (i64, i64, i64) = sqlx::query_as(
            "SELECT COALESCE(SUM(kind = 'image'), 0), COALESCE(SUM(kind = 'text'), 0),
                    COALESCE(SUM(starred), 0)
             FROM asset WHERE product_id = ?",
        )
        .bind(product_id)
        .fetch_one(&self.pool)
        .await?;
        let copies_by_language = sqlx::query_as(
            "SELECT language, COUNT(*), COALESCE(SUM(status = 'approved'), 0)
             FROM product_copy WHERE product_id = ? GROUP BY language ORDER BY language",
        )
        .bind(product_id)
        .fetch_all(&self.pool)
        .await?;
        let batches_by_module = sqlx::query_as(
            "SELECT module, COUNT(*) FROM batch WHERE product_id = ? GROUP BY module ORDER BY module",
        )
        .bind(product_id)
        .fetch_all(&self.pool)
        .await?;
        let last_batch_at =
            sqlx::query_scalar("SELECT MAX(created_at) FROM batch WHERE product_id = ?")
                .bind(product_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(ProductSummaryRecord {
            sku_count,
            image_assets,
            text_assets,
            starred_assets,
            copies_by_language,
            batches_by_module,
            last_batch_at,
        })
    }
}

// ---------- 素材 ----------

impl Store {
    /// 新增素材并打标签，一个事务完成。
    pub async fn create_asset(
        &self,
        asset: &NewAsset,
        tags: &[String],
        now: i64,
    ) -> Result<AssetRecord> {
        let id = new_id();
        let mut tx = self.begin_write().await?;
        sqlx::query(
            "INSERT INTO asset (id, kind, product_id, source, title, file_path, thumb_path,
                                text_content, mime, width, height, sha256, phash, origin_batch_id,
                                origin_job_id, meta, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17)",
        )
        .bind(&id)
        .bind(&asset.kind)
        .bind(&asset.product_id)
        .bind(&asset.source)
        .bind(&asset.title)
        .bind(&asset.file_path)
        .bind(&asset.thumb_path)
        .bind(&asset.text_content)
        .bind(&asset.mime)
        .bind(asset.width)
        .bind(asset.height)
        .bind(&asset.sha256)
        .bind(&asset.phash)
        .bind(&asset.origin_batch_id)
        .bind(&asset.origin_job_id)
        .bind(&asset.meta)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|e| check_violation(e, "图片素材必须有文件，文案素材必须有内容"))?;
        insert_tags(&mut tx, &id, tags, now).await?;
        tx.commit().await?;
        self.get_asset(&id).await
    }

    pub async fn get_asset(&self, id: &str) -> Result<AssetRecord> {
        sqlx::query_as::<_, AssetRecord>("SELECT * FROM asset WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::not_found("素材", id))
    }

    /// 按内容哈希找已有的图片素材，同一张图不重复存。
    pub async fn find_asset_by_sha256(&self, sha256: &str) -> Result<Option<AssetRecord>> {
        let row = sqlx::query_as::<_, AssetRecord>(
            "SELECT * FROM asset WHERE sha256 = ? ORDER BY created_at LIMIT 1",
        )
        .bind(sha256)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// 素材列表，最新的在前。
    pub async fn list_assets(&self, query: &AssetQuery) -> Result<Vec<AssetRecord>> {
        let rows = sqlx::query_as::<_, AssetRecord>(&format!(
            "SELECT a.* FROM asset a WHERE {ASSET_FILTER}
             ORDER BY a.created_at DESC, a.id
             LIMIT ?7 OFFSET ?8"
        ))
        .bind(&query.product_id)
        .bind(&query.kind)
        .bind(&query.source)
        .bind(query.tag.as_deref().map(str::trim))
        .bind(query.starred_only)
        .bind(like_pattern(query.keyword.as_deref()))
        .bind(query.limit)
        .bind(query.offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 和 `list_assets` 同样条件下的总数，分页用。
    pub async fn count_assets(&self, query: &AssetQuery) -> Result<i64> {
        let count = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM asset a WHERE {ASSET_FILTER}"
        ))
        .bind(&query.product_id)
        .bind(&query.kind)
        .bind(&query.source)
        .bind(query.tag.as_deref().map(str::trim))
        .bind(query.starred_only)
        .bind(like_pattern(query.keyword.as_deref()))
        .fetch_one(&self.pool)
        .await?;
        Ok(count)
    }

    /// 一页素材的标签，一次查出来，返回 `(素材 ID, 标签)`，按素材、标签排序。
    pub async fn tags_of_assets(&self, asset_ids: &[String]) -> Result<Vec<(String, String)>> {
        if asset_ids.is_empty() {
            return Ok(Vec::new());
        }
        // 一页最多几百个，直接拼占位符；SQLite 默认最多 32766 个参数
        let placeholders = vec!["?"; asset_ids.len()].join(", ");
        let sql = format!(
            "SELECT asset_id, tag FROM asset_tag WHERE asset_id IN ({placeholders})
             ORDER BY asset_id, tag"
        );
        let mut query = sqlx::query_as(&sql);
        for id in asset_ids {
            query = query.bind(id);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn asset_tags(&self, asset_id: &str) -> Result<Vec<String>> {
        let tags = sqlx::query_scalar("SELECT tag FROM asset_tag WHERE asset_id = ? ORDER BY tag")
            .bind(asset_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(tags)
    }

    /// 整体替换素材的标签。
    pub async fn set_asset_tags(&self, asset_id: &str, tags: &[String], now: i64) -> Result<()> {
        let mut tx = self.begin_write().await?;
        sqlx::query("DELETE FROM asset_tag WHERE asset_id = ?")
            .bind(asset_id)
            .execute(&mut *tx)
            .await?;
        insert_tags(&mut tx, asset_id, tags, now).await?;
        sqlx::query("UPDATE asset SET updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(asset_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// 所有用过的标签和使用次数，标签筛选下拉框用。
    pub async fn list_asset_tags(&self) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query_as(
            "SELECT tag, COUNT(*) FROM asset_tag GROUP BY tag ORDER BY COUNT(*) DESC, tag",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn set_asset_starred(&self, asset_id: &str, starred: bool, now: i64) -> Result<()> {
        sqlx::query("UPDATE asset SET starred = ?, updated_at = ? WHERE id = ?")
            .bind(starred)
            .bind(now)
            .bind(asset_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// 把素材关联到商品，`None` 取消关联。
    pub async fn link_asset_product(
        &self,
        asset_id: &str,
        product_id: Option<&str>,
        now: i64,
    ) -> Result<()> {
        sqlx::query("UPDATE asset SET product_id = ?, updated_at = ? WHERE id = ?")
            .bind(product_id)
            .bind(now)
            .bind(asset_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// 删除素材记录，返回记录本身，由调用方决定要不要删文件（同一个文件可能被别的素材引用）。
    pub async fn delete_asset(&self, asset_id: &str) -> Result<AssetRecord> {
        let asset = self.get_asset(asset_id).await?;
        sqlx::query("DELETE FROM asset WHERE id = ?")
            .bind(asset_id)
            .execute(&self.pool)
            .await?;
        Ok(asset)
    }

    /// 还有多少素材引用这个文件，删文件前检查。
    pub async fn count_assets_with_file(&self, file_path: &str) -> Result<i64> {
        let count = sqlx::query_scalar("SELECT COUNT(*) FROM asset WHERE file_path = ?")
            .bind(file_path)
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }
}

// ---------- 文案 ----------

impl Store {
    pub async fn create_copy(&self, copy: &NewCopy, now: i64) -> Result<CopyRecord> {
        let id = new_id();
        sqlx::query(
            "INSERT INTO product_copy (id, product_id, shop_id, language, field, content,
                                       origin_job_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        )
        .bind(&id)
        .bind(&copy.product_id)
        .bind(&copy.shop_id)
        .bind(&copy.language)
        .bind(&copy.field)
        .bind(&copy.content)
        .bind(&copy.origin_job_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| check_violation(e, "文案字段不正确"))?;
        self.get_copy(&id).await
    }

    pub async fn get_copy(&self, id: &str) -> Result<CopyRecord> {
        sqlx::query_as::<_, CopyRecord>("SELECT * FROM product_copy WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::not_found("文案", id))
    }

    /// 一个商品的文案。`shop_id` / `language` 为空时不按它筛选。
    pub async fn list_copies(
        &self,
        product_id: &str,
        shop_id: Option<&str>,
        language: Option<&str>,
    ) -> Result<Vec<CopyRecord>> {
        let rows = sqlx::query_as::<_, CopyRecord>(
            "SELECT * FROM product_copy
             WHERE product_id = ?1 AND (?2 IS NULL OR shop_id = ?2) AND (?3 IS NULL OR language = ?3)
             ORDER BY shop_id, language, field, created_at DESC",
        )
        .bind(product_id)
        .bind(shop_id)
        .bind(language)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 修改文案内容和状态（草稿 / 已确认）。
    pub async fn update_copy(&self, id: &str, content: &str, status: &str, now: i64) -> Result<()> {
        let result = sqlx::query(
            "UPDATE product_copy SET content = ?, status = ?, updated_at = ? WHERE id = ?",
        )
        .bind(content)
        .bind(status)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| check_violation(e, "文案状态只能是草稿或已确认"))?;
        if result.rows_affected() == 0 {
            return Err(StoreError::not_found("文案", id));
        }
        Ok(())
    }

    pub async fn delete_copy(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM product_copy WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

// ---------- 工具函数 ----------

async fn insert_tags(
    conn: &mut sqlx::SqliteConnection,
    asset_id: &str,
    tags: &[String],
    now: i64,
) -> Result<()> {
    for tag in tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        sqlx::query("INSERT OR IGNORE INTO asset_tag (asset_id, tag, created_at) VALUES (?, ?, ?)")
            .bind(asset_id)
            .bind(tag)
            .bind(now)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// 把关键词变成 LIKE 模式：转义 `%`、`_`，两边加 `%`。空关键词返回 `None`（不筛选）。
pub(crate) fn like_pattern(keyword: Option<&str>) -> Option<String> {
    let keyword = keyword?.trim();
    if keyword.is_empty() {
        return None;
    }
    let escaped = keyword
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    Some(format!("%{escaped}%"))
}

fn duplicate_code(error: sqlx::Error, code: Option<&str>) -> StoreError {
    match &error {
        sqlx::Error::Database(db) if db.is_unique_violation() => StoreError::Duplicate {
            entity: "商品",
            field: "编码",
            value: code.unwrap_or_default().to_owned(),
        },
        _ => StoreError::Database(error),
    }
}

fn check_violation(error: sqlx::Error, message: &str) -> StoreError {
    match &error {
        sqlx::Error::Database(db) if db.is_check_violation() => {
            StoreError::Invalid(message.to_owned())
        }
        sqlx::Error::Database(db) if db.is_foreign_key_violation() => {
            StoreError::Invalid("关联的商品、店铺或批次不存在".to_owned())
        }
        _ => StoreError::Database(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_pattern_escapes_wildcards() {
        assert_eq!(like_pattern(Some(" T恤 ")), Some("%T恤%".into()));
        assert_eq!(like_pattern(Some("50%_off")), Some("%50\\%\\_off%".into()));
        assert_eq!(like_pattern(Some("   ")), None);
        assert_eq!(like_pattern(None), None);
    }
}
