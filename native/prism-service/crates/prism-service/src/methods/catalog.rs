//! `catalog.*`：店铺、商品、表格导入商品、素材（原 Tauri 版 `catalog_commands.rs`）。

use std::path::PathBuf;

use prism_catalog::{
    AssetFilter, ImportMode, ImportOptions, ImportRequest, ProductFilter, ProductInput, ShopInput,
};
use prism_io::{Mapping, ReadOptions};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Context, done, fail, parse, reply};
use crate::protocol::RpcError;

#[derive(Debug, Deserialize)]
struct Id {
    id: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ListShops {
    platform: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateShop {
    input: ShopInput,
}

#[derive(Debug, Deserialize)]
struct UpdateShop {
    id: String,
    input: ShopInput,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ListProducts {
    filter: ProductFilter,
}

#[derive(Debug, Deserialize)]
struct CreateProduct {
    input: ProductInput,
}

#[derive(Debug, Deserialize)]
struct UpdateProduct {
    id: String,
    input: ProductInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewImport {
    path: PathBuf,
    sheet: Option<String>,
    header_line: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportProducts {
    path: PathBuf,
    sheet: Option<String>,
    header_line: Option<usize>,
    mapping: Mapping,
    #[serde(default)]
    mode: ImportMode,
    save_as: Option<String>,
    matched_mapping_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ListAssets {
    filter: AssetFilter,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportAssetFiles {
    paths: Vec<PathBuf>,
    product_id: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SetAssetTags {
    id: String,
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct StarAsset {
    id: String,
    starred: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkAsset {
    id: String,
    product_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct TagCount {
    tag: String,
    count: i64,
}

#[derive(Debug, Serialize)]
struct FailedFile {
    path: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct ImportAssetFilesResult {
    imported: Vec<prism_catalog::ImportedAsset>,
    failed: Vec<FailedFile>,
}

pub(super) async fn dispatch(
    ctx: &Context,
    method: &str,
    name: &str,
    params: Value,
) -> Result<Value, RpcError> {
    let catalog = &ctx.catalog;
    match name {
        // ---------- 店铺 ----------
        "listShops" => {
            let p: ListShops = parse(params)?;
            reply(
                catalog
                    .list_shops(p.platform.as_deref())
                    .await
                    .map_err(fail)?,
            )
        }
        "createShop" => {
            let p: CreateShop = parse(params)?;
            reply(catalog.create_shop(&p.input).await.map_err(fail)?)
        }
        "updateShop" => {
            let p: UpdateShop = parse(params)?;
            reply(catalog.update_shop(&p.id, &p.input).await.map_err(fail)?)
        }
        "deleteShop" => {
            let p: Id = parse(params)?;
            catalog.delete_shop(&p.id).await.map_err(fail)?;
            Ok(done())
        }

        // ---------- 商品 ----------
        "listProducts" => {
            let p: ListProducts = parse(params)?;
            reply(catalog.list_products(&p.filter).await.map_err(fail)?)
        }
        "productDetail" => {
            let p: Id = parse(params)?;
            reply(catalog.product_detail(&p.id).await.map_err(fail)?)
        }
        "createProduct" => {
            let p: CreateProduct = parse(params)?;
            reply(catalog.create_product(&p.input).await.map_err(fail)?)
        }
        "updateProduct" => {
            let p: UpdateProduct = parse(params)?;
            reply(
                catalog
                    .update_product(&p.id, &p.input)
                    .await
                    .map_err(fail)?,
            )
        }
        "archiveProduct" => {
            let p: Id = parse(params)?;
            catalog.archive_product(&p.id).await.map_err(fail)?;
            Ok(done())
        }
        "restoreProduct" => {
            let p: Id = parse(params)?;
            catalog.restore_product(&p.id).await.map_err(fail)?;
            Ok(done())
        }
        "deleteProduct" => {
            let p: Id = parse(params)?;
            catalog.delete_product(&p.id).await.map_err(fail)?;
            Ok(done())
        }

        // ---------- 从表格导入商品 ----------
        "previewProductImport" => {
            let p: PreviewImport = parse(params)?;
            let read = ReadOptions {
                sheet: p.sheet,
                header_line: p.header_line,
            };
            reply(
                catalog
                    .preview_product_import(&p.path, &read)
                    .await
                    .map_err(fail)?,
            )
        }
        "importProducts" => {
            let p: ImportProducts = parse(params)?;
            let request = ImportRequest {
                path: p.path,
                read: ReadOptions {
                    sheet: p.sheet,
                    header_line: p.header_line,
                },
                mapping: p.mapping,
                mode: p.mode,
                save_as: p.save_as,
                matched_mapping_id: p.matched_mapping_id,
            };
            reply(catalog.import_products(&request).await.map_err(fail)?)
        }
        "listImportMappings" => reply(catalog.list_import_mappings().await.map_err(fail)?),
        "deleteImportMapping" => {
            let p: Id = parse(params)?;
            catalog.delete_import_mapping(&p.id).await.map_err(fail)?;
            Ok(done())
        }

        // ---------- 素材 ----------
        "listAssets" => {
            let p: ListAssets = parse(params)?;
            reply(catalog.list_assets(&p.filter).await.map_err(fail)?)
        }
        "listAssetTags" => {
            let tags = catalog.list_tags().await.map_err(fail)?;
            reply(
                tags.into_iter()
                    .map(|(tag, count)| TagCount { tag, count })
                    .collect::<Vec<_>>(),
            )
        }
        "importAssetFiles" => {
            let p: ImportAssetFiles = parse(params)?;
            reply(import_asset_files(ctx, p).await)
        }
        "setAssetTags" => {
            let p: SetAssetTags = parse(params)?;
            reply(catalog.set_asset_tags(&p.id, &p.tags).await.map_err(fail)?)
        }
        "starAsset" => {
            let p: StarAsset = parse(params)?;
            catalog.star_asset(&p.id, p.starred).await.map_err(fail)?;
            Ok(done())
        }
        "linkAsset" => {
            let p: LinkAsset = parse(params)?;
            catalog
                .link_asset(&p.id, p.product_id.as_deref())
                .await
                .map_err(fail)?;
            Ok(done())
        }
        "deleteAsset" => {
            let p: Id = parse(params)?;
            catalog.delete_asset(&p.id).await.map_err(fail)?;
            Ok(done())
        }
        _ => Err(RpcError::method_not_found(method)),
    }
}

/// 导入多个图片文件。一个文件失败不影响其他文件。
async fn import_asset_files(ctx: &Context, p: ImportAssetFiles) -> ImportAssetFilesResult {
    let options = ImportOptions {
        product_id: p.product_id,
        tags: p.tags,
        ..ImportOptions::default()
    };
    let mut result = ImportAssetFilesResult {
        imported: Vec::new(),
        failed: Vec::new(),
    };
    for path in p.paths {
        match ctx.catalog.import_image(&path, &options).await {
            Ok(imported) => result.imported.push(imported),
            Err(error) => {
                let message = super::fail(error).message;
                result.failed.push(FailedFile {
                    path: path.display().to_string(),
                    message,
                });
            }
        }
    }
    result
}
