//! 商品和规格。

use std::collections::HashMap;

use prism_core::now_ms;
use prism_store::{NewProduct, NewSku, ProductQuery};

use crate::error::invalid;
use crate::types::{list_to_json, pairs_to_json};
use crate::validate::{
    self, ATTRIBUTE_NAME_MAX, ATTRIBUTE_VALUE_MAX, ATTRIBUTES_MAX, DESCRIPTION_MAX, NAME_MAX,
    PairLimits, SELLING_POINT_MAX, SELLING_POINTS_MAX, SHORT_TEXT_MAX, SKUS_MAX, SPEC_ITEMS_MAX,
    SPEC_NAME_MAX, SPEC_VALUE_MAX,
};
use crate::{
    Catalog, Page, Product, ProductDetail, ProductFilter, ProductInput, Result, Sku, SkuInput,
};

const ATTRIBUTE_LIMITS: PairLimits = PairLimits {
    max_items: ATTRIBUTES_MAX,
    name_max: ATTRIBUTE_NAME_MAX,
    value_max: ATTRIBUTE_VALUE_MAX,
};

const SPEC_LIMITS: PairLimits = PairLimits {
    max_items: SPEC_ITEMS_MAX,
    name_max: SPEC_NAME_MAX,
    value_max: SPEC_VALUE_MAX,
};

impl Catalog {
    pub async fn create_product(&self, input: &ProductInput) -> Result<Product> {
        let product = check_product(input)?;
        Ok(self.store.create_product(&product, now_ms()).await?.into())
    }

    pub async fn update_product(&self, id: &str, input: &ProductInput) -> Result<Product> {
        let product = check_product(input)?;
        Ok(self
            .store
            .update_product(id, &product, now_ms())
            .await?
            .into())
    }

    pub async fn get_product(&self, id: &str) -> Result<Product> {
        Ok(self.store.get_product(id).await?.into())
    }

    /// 商品详情：商品、规格、汇总（详细设计 9.4）。
    pub async fn product_detail(&self, id: &str) -> Result<ProductDetail> {
        let product = self.store.get_product(id).await?.into();
        let skus = self.store.list_skus(id).await?;
        let summary = self.store.product_summary(id).await?.into();
        Ok(ProductDetail {
            product,
            skus: skus.into_iter().map(Sku::from).collect(),
            summary,
        })
    }

    /// 商品列表，最近更新的在前，带总数。
    pub async fn list_products(&self, filter: &ProductFilter) -> Result<Page<Product>> {
        let (limit, offset) = validate::page(filter.limit, filter.offset);
        let query = ProductQuery {
            keyword: filter
                .keyword
                .as_deref()
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(str::to_owned),
            status: Some(
                if filter.archived {
                    "archived"
                } else {
                    "active"
                }
                .into(),
            ),
            limit,
            offset,
        };
        let items = self.store.list_products(&query).await?;
        let total = self.store.count_products(&query).await?;
        Ok(Page {
            items: items.into_iter().map(Product::from).collect(),
            total,
        })
    }

    /// 归档：默认不出现在列表里，数据都保留。
    pub async fn archive_product(&self, id: &str) -> Result<()> {
        self.set_status(id, "archived").await
    }

    pub async fn restore_product(&self, id: &str) -> Result<()> {
        self.set_status(id, "active").await
    }

    async fn set_status(&self, id: &str, status: &str) -> Result<()> {
        self.store.get_product(id).await?;
        self.store.set_product_status(id, status, now_ms()).await?;
        Ok(())
    }

    /// 删除商品：规格和文案一起删，素材保留（取消关联）。
    pub async fn delete_product(&self, id: &str) -> Result<()> {
        self.store.get_product(id).await?;
        self.store.delete_product(id).await?;
        Ok(())
    }

    /// 整体替换商品的规格，顺序就是显示顺序。
    pub async fn replace_skus(&self, product_id: &str, inputs: &[SkuInput]) -> Result<Vec<Sku>> {
        self.store.get_product(product_id).await?;
        let skus = check_skus(inputs)?;
        let records = self.store.replace_skus(product_id, &skus, now_ms()).await?;
        Ok(records.into_iter().map(Sku::from).collect())
    }
}

fn check_product(input: &ProductInput) -> Result<NewProduct> {
    let selling_points = validate::lines(
        "卖点",
        &input.selling_points,
        SELLING_POINTS_MAX,
        SELLING_POINT_MAX,
    )?;
    let attributes = validate::pairs("参数", &input.attributes, &ATTRIBUTE_LIMITS)?;
    let markets = validate::markets(&input.target_markets)?;
    Ok(NewProduct {
        code: validate::code("商品编码", input.code.as_deref())?,
        name: validate::required("商品名称", &input.name, NAME_MAX)?,
        category: validate::optional("类目", input.category.as_deref(), SHORT_TEXT_MAX)?,
        brand: validate::optional("品牌", input.brand.as_deref(), SHORT_TEXT_MAX)?,
        description: validate::optional("商品说明", input.description.as_deref(), DESCRIPTION_MAX)?,
        selling_points: list_to_json(&selling_points),
        attributes: pairs_to_json(&attributes),
        cost_cents: validate::non_negative("成本", input.cost_cents)?,
        currency: validate::currency(&input.currency)?,
        target_markets: list_to_json(&markets),
    })
}

fn check_skus(inputs: &[SkuInput]) -> Result<Vec<NewSku>> {
    if inputs.len() > SKUS_MAX {
        return Err(invalid(format!("每个商品最多 {SKUS_MAX} 个规格")));
    }
    // 规格组合 → 第几个，用来指出和谁重复
    let mut specs: HashMap<Vec<(String, String)>, usize> = HashMap::new();
    let mut codes: HashMap<String, usize> = HashMap::new();
    let mut skus = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        let number = index + 1;
        let field = format!("第 {number} 个规格");
        let spec = validate::pairs(&field, &input.spec, &SPEC_LIMITS)?;
        if spec.is_empty() {
            return Err(invalid(format!("{field}没有填规格，如 颜色 = 红")));
        }
        if let Some(previous) = specs.insert(spec.clone(), number) {
            return Err(invalid(format!(
                "第 {number} 个规格和第 {previous} 个重复了"
            )));
        }
        let code = validate::code(&format!("{field}的编码"), input.code.as_deref())?;
        if let Some(code) = &code
            && let Some(previous) = codes.insert(code.clone(), number)
        {
            return Err(invalid(format!(
                "第 {number} 个规格的编码「{code}」和第 {previous} 个重复了"
            )));
        }
        skus.push(NewSku {
            code,
            spec: crate::pairs::to_json(&spec),
            price_cents: validate::non_negative(&format!("{field}的售价"), input.price_cents)?,
            cost_cents: validate::non_negative(&format!("{field}的成本"), input.cost_cents)?,
            stock: validate::non_negative(&format!("{field}的库存"), input.stock)?,
        });
    }
    Ok(skus)
}
