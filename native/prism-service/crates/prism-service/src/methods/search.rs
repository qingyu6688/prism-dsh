//! `search.global`：商品、素材各取前几条（原 Tauri 版 `search_commands.rs`）。
//!
//! 会话搜索用 dsh 自带的；批次在 D4 有了生图之后再加。

use prism_catalog::{Asset, AssetFilter, Product, ProductFilter};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Context, fail, parse, reply};
use crate::protocol::RpcError;

/// 每组最多几条，搜索框里放得下。
const GROUP_LIMIT: i64 = 5;

#[derive(Debug, Deserialize)]
struct Global {
    keyword: String,
}

#[derive(Debug, Default, Serialize)]
struct Results {
    products: Vec<Product>,
    assets: Vec<Asset>,
}

pub(super) async fn dispatch(
    ctx: &Context,
    method: &str,
    name: &str,
    params: Value,
) -> Result<Value, RpcError> {
    if name != "global" {
        return Err(RpcError::method_not_found(method));
    }
    let p: Global = parse(params)?;
    let keyword = p.keyword.trim();
    if keyword.is_empty() {
        return reply(Results::default());
    }
    let products = ctx
        .catalog
        .list_products(&ProductFilter {
            keyword: Some(keyword.to_owned()),
            archived: false,
            limit: GROUP_LIMIT,
            offset: 0,
        })
        .await
        .map_err(fail)?
        .items;
    let assets = ctx
        .catalog
        .list_assets(&AssetFilter {
            keyword: Some(keyword.to_owned()),
            limit: GROUP_LIMIT,
            ..AssetFilter::default()
        })
        .await
        .map_err(fail)?
        .items;
    reply(Results { products, assets })
}
