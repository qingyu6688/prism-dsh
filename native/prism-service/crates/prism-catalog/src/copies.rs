//! 商品文案：标题、五点、描述等，按语言和店铺区分。

use prism_core::now_ms;
use prism_store::NewCopy;

use crate::validate::{self, COPY_FIELDS, TEXT_ASSET_MAX};
use crate::{Catalog, Copy, CopyInput, Result};

const DRAFT: &str = "draft";
const APPROVED: &str = "approved";

impl Catalog {
    /// 新建一条文案，状态为草稿。
    pub async fn save_copy(&self, input: &CopyInput) -> Result<Copy> {
        self.store.get_product(&input.product_id).await?;
        if let Some(shop_id) = &input.shop_id {
            self.store.get_shop(shop_id).await?;
        }
        let copy = NewCopy {
            product_id: input.product_id.clone(),
            shop_id: input.shop_id.clone(),
            language: validate::language(&input.language)?,
            field: validate::one_of("文案字段", &input.field, COPY_FIELDS)?,
            content: validate::required("文案内容", &input.content, TEXT_ASSET_MAX)?,
            origin_job_id: None,
        };
        Ok(self.store.create_copy(&copy, now_ms()).await?.into())
    }

    /// 改文案内容。内容有变化时回到草稿，需要重新确认。
    pub async fn update_copy(&self, id: &str, content: &str) -> Result<Copy> {
        let current = self.store.get_copy(id).await?;
        let content = validate::required("文案内容", content, TEXT_ASSET_MAX)?;
        if content != current.content {
            self.store
                .update_copy(id, &content, DRAFT, now_ms())
                .await?;
        }
        Ok(self.store.get_copy(id).await?.into())
    }

    /// 确认文案，导出时优先用已确认的。
    pub async fn approve_copy(&self, id: &str) -> Result<Copy> {
        let current = self.store.get_copy(id).await?;
        self.store
            .update_copy(id, &current.content, APPROVED, now_ms())
            .await?;
        Ok(self.store.get_copy(id).await?.into())
    }

    /// 某个商品的文案，可按店铺、语言筛选。
    pub async fn list_copies(
        &self,
        product_id: &str,
        shop_id: Option<&str>,
        language: Option<&str>,
    ) -> Result<Vec<Copy>> {
        let language = match language.map(str::trim).filter(|l| !l.is_empty()) {
            Some(language) => Some(validate::language(language)?),
            None => None,
        };
        let copies = self
            .store
            .list_copies(product_id, shop_id, language.as_deref())
            .await?;
        Ok(copies.into_iter().map(Copy::from).collect())
    }

    pub async fn get_copy(&self, id: &str) -> Result<Copy> {
        Ok(self.store.get_copy(id).await?.into())
    }

    pub async fn delete_copy(&self, id: &str) -> Result<()> {
        self.store.get_copy(id).await?;
        self.store.delete_copy(id).await?;
        Ok(())
    }
}
