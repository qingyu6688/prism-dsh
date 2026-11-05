//! 商品库：商品、规格、店铺、素材、文案的业务层（详细设计第 9 章）。
//!
//! 在 `prism-store` 的读写之上做输入校验和规整、JSON 字段的结构化、素材文件入库和删除、
//! 商品汇总。不生成内容，也不知道平台规则（规则在 `prism-rules`）。

mod assets;
mod copies;
mod error;
mod import;
mod pairs;
mod products;
mod shops;
mod types;
mod validate;

use std::path::PathBuf;

use prism_store::Store;
use tokio::sync::Mutex;

pub use error::CatalogError;
pub use import::{
    ImportMode, ImportPreview, ImportRequest, ImportResult, MatchedMapping, SavedMapping,
    product_import_fields,
};
pub use types::{
    Asset, AssetFilter, Attribute, BatchCount, Copy, CopyCount, CopyInput, ImportOptions,
    ImportedAsset, Page, Product, ProductDetail, ProductFilter, ProductInput, ProductSummary, Shop,
    ShopInput, Sku, SkuInput, TextAssetInput,
};
pub use validate::MAX_IMAGE_BYTES;

/// 商品库的返回类型。
pub type Result<T> = std::result::Result<T, CatalogError>;

/// 商品库。持有存储和数据目录（素材图片存在数据目录的 `assets/` 下）。
pub struct Catalog {
    store: Store,
    data_dir: PathBuf,
    /// 图片入库时"查重 → 写文件 → 建记录"要串行，否则同一张图并发导入会建出两条记录
    import_lock: Mutex<()>,
}

impl Catalog {
    pub fn new(store: Store, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            store,
            data_dir: data_dir.into(),
            import_lock: Mutex::new(()),
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// 素材图片在磁盘上的完整路径；文案素材返回 `None`。
    pub fn asset_path(&self, asset: &Asset) -> Option<PathBuf> {
        asset
            .file_path
            .as_deref()
            .map(|relative| self.data_dir.join(relative))
    }
}
