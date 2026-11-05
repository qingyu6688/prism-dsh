//! 店铺。

use prism_core::now_ms;
use prism_store::NewShop;

use crate::validate::{self, NAME_MAX, NOTES_MAX};
use crate::{Catalog, Result, Shop, ShopInput};

impl Catalog {
    pub async fn create_shop(&self, input: &ShopInput) -> Result<Shop> {
        let shop = check_shop(input)?;
        Ok(self.store.create_shop(&shop, now_ms()).await?.into())
    }

    pub async fn update_shop(&self, id: &str, input: &ShopInput) -> Result<Shop> {
        let shop = check_shop(input)?;
        Ok(self.store.update_shop(id, &shop, now_ms()).await?.into())
    }

    pub async fn get_shop(&self, id: &str) -> Result<Shop> {
        Ok(self.store.get_shop(id).await?.into())
    }

    /// 店铺列表，可按平台筛选。
    pub async fn list_shops(&self, platform: Option<&str>) -> Result<Vec<Shop>> {
        let platform = match platform.map(str::trim).filter(|p| !p.is_empty()) {
            Some(platform) => Some(validate::platform(platform)?),
            None => None,
        };
        let shops = self.store.list_shops(platform.as_deref()).await?;
        Ok(shops.into_iter().map(Shop::from).collect())
    }

    /// 删除店铺。针对这个店铺写的文案保留，变成通用文案。
    pub async fn delete_shop(&self, id: &str) -> Result<()> {
        self.store.get_shop(id).await?;
        self.store.delete_shop(id).await?;
        Ok(())
    }
}

fn check_shop(input: &ShopInput) -> Result<NewShop> {
    Ok(NewShop {
        name: validate::required("店铺名称", &input.name, NAME_MAX)?,
        platform: validate::platform(&input.platform)?,
        site: validate::optional_region("站点", input.site.as_deref())?,
        language: validate::language(&input.language)?,
        currency: validate::currency(&input.currency)?,
        notes: validate::optional("备注", input.notes.as_deref(), NOTES_MAX)?,
    })
}
