//! 素材：图片入库（按内容去重）、文案片段、标签、收藏、删除（详细设计 9.3）。

use std::collections::HashMap;
use std::path::Path;

use prism_core::{new_id, now_ms};
use prism_store::{AssetQuery, NewAsset};
use sha2::{Digest, Sha256};

use crate::error::invalid;
use crate::types::file_title;
use crate::validate::{
    self, ASSET_KINDS, ASSET_SOURCES, MAX_IMAGE_BYTES, TEXT_ASSET_MAX, TITLE_MAX,
};
use crate::{
    Asset, AssetFilter, Catalog, CatalogError, ImportOptions, ImportedAsset, Page, Result,
    TextAssetInput,
};

/// 素材图片放在数据目录下的这个文件夹。
const ASSET_DIR: &str = "assets";

impl Catalog {
    /// 从文件导入图片素材。标题不填时用文件名。
    pub async fn import_image(
        &self,
        path: &Path,
        options: &ImportOptions,
    ) -> Result<ImportedAsset> {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|error| CatalogError::read_file(path, error))?;
        // 先看大小再读，避免把一个几 GB 的文件读进内存
        check_size(metadata.len())?;
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|error| CatalogError::read_file(path, error))?;
        let mut options = options.clone();
        if options.title.as_deref().is_none_or(|t| t.trim().is_empty()) {
            options.title = file_title(path);
        }
        self.import_image_bytes(&bytes, &options).await
    }

    /// 导入内存里的图片（生成结果、采集到的图片）。
    pub async fn import_image_bytes(
        &self,
        bytes: &[u8],
        options: &ImportOptions,
    ) -> Result<ImportedAsset> {
        check_size(u64::try_from(bytes.len()).unwrap_or(u64::MAX))?;
        let source = validate::one_of("素材来源", &options.source, ASSET_SOURCES)?;
        let title = validate::optional("标题", options.title.as_deref(), TITLE_MAX)?;
        let tags = validate::tags(&options.tags)?;
        let info = prism_imaging::probe(bytes)?;
        if let Some(product_id) = &options.product_id {
            // 先确认商品存在，免得文件写好了建记录才失败，留下没人用的文件
            self.store.get_product(product_id).await?;
        }
        let sha256 = hex(&Sha256::digest(bytes));
        // 解码比较耗 CPU，放到阻塞线程池；能完整解码也说明像素数据没坏（文件头好、内容坏的会在这里拦下）
        let owned = bytes.to_vec();
        let (phash, thumbnail) = tokio::task::spawn_blocking(move || {
            let image = prism_imaging::decode(&owned)?;
            Ok::<_, prism_imaging::ImagingError>((
                prism_imaging::phash(&image),
                prism_imaging::thumbnail_webp(&image)?,
            ))
        })
        .await
        .map_err(|error| invalid(format!("处理图片时出错：{error}")))??;

        let _guard = self.import_lock.lock().await;
        if let Some(existing) = self.store.find_asset_by_sha256(&sha256).await? {
            let asset = self
                .merge_into_existing(&existing.id, existing.product_id.is_none(), options, tags)
                .await?;
            return Ok(ImportedAsset {
                asset,
                deduplicated: true,
            });
        }

        let relative = format!(
            "{ASSET_DIR}/{}/{sha256}.{}",
            &sha256[..2],
            info.format.extension()
        );
        let thumb = thumb_path(&relative);
        self.write_file(&relative, bytes).await?;
        self.write_file(&thumb, &thumbnail).await?;
        let record = self
            .store
            .create_asset(
                &NewAsset {
                    kind: "image".into(),
                    product_id: options.product_id.clone(),
                    source,
                    title,
                    file_path: Some(relative),
                    mime: Some(info.format.mime().into()),
                    width: Some(i64::from(info.width)),
                    height: Some(i64::from(info.height)),
                    sha256: Some(sha256),
                    origin_batch_id: options.origin_batch_id.clone(),
                    origin_job_id: options.origin_job_id.clone(),
                    thumb_path: Some(thumb),
                    phash: Some(phash),
                    ..NewAsset::default()
                },
                &tags,
                now_ms(),
            )
            .await?;
        Ok(ImportedAsset {
            asset: Asset::from_record(record, tags),
            deduplicated: false,
        })
    }

    /// 同样内容的图片已经在库里：没关联商品的顺带关联上，标签合并。
    async fn merge_into_existing(
        &self,
        asset_id: &str,
        unlinked: bool,
        options: &ImportOptions,
        tags: Vec<String>,
    ) -> Result<Asset> {
        let now = now_ms();
        if let (Some(product_id), true) = (&options.product_id, unlinked) {
            self.store
                .link_asset_product(asset_id, Some(product_id), now)
                .await?;
        }
        if !tags.is_empty() {
            let mut merged = self.store.asset_tags(asset_id).await?;
            merged.extend(tags);
            let merged = validate::tags(&merged)?;
            self.store.set_asset_tags(asset_id, &merged, now).await?;
        }
        self.get_asset(asset_id).await
    }

    /// 写到数据目录：先写临时文件再改名，中途失败不留半个文件。同样内容的文件已存在就不写。
    async fn write_file(&self, relative: &str, bytes: &[u8]) -> Result<()> {
        let target = self.data_dir.join(relative);
        let exists = tokio::fs::try_exists(&target)
            .await
            .map_err(|error| CatalogError::write_file(&target, error))?;
        if exists {
            return Ok(());
        }
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| CatalogError::write_file(parent, error))?;
        }
        let mut temp_name = target.as_os_str().to_owned();
        temp_name.push(format!(".{}.tmp", new_id()));
        let temp = std::path::PathBuf::from(temp_name);
        if let Err(error) = tokio::fs::write(&temp, bytes).await {
            remove_quietly(&temp).await;
            return Err(CatalogError::write_file(&target, error));
        }
        if let Err(error) = tokio::fs::rename(&temp, &target).await {
            remove_quietly(&temp).await;
            // 别的导入刚好写了同一个文件（内容一样），也算成功
            if tokio::fs::try_exists(&target).await.unwrap_or(false) {
                return Ok(());
            }
            return Err(CatalogError::write_file(&target, error));
        }
        Ok(())
    }

    /// 新建文案素材。
    pub async fn add_text_asset(&self, input: &TextAssetInput) -> Result<Asset> {
        let source = validate::one_of("素材来源", &input.source, ASSET_SOURCES)?;
        let content = validate::required("素材内容", &input.content, TEXT_ASSET_MAX)?;
        let tags = validate::tags(&input.tags)?;
        if let Some(product_id) = &input.product_id {
            self.store.get_product(product_id).await?;
        }
        let record = self
            .store
            .create_asset(
                &NewAsset {
                    kind: "text".into(),
                    product_id: input.product_id.clone(),
                    source,
                    title: validate::optional("标题", input.title.as_deref(), TITLE_MAX)?,
                    text_content: Some(content),
                    ..NewAsset::default()
                },
                &tags,
                now_ms(),
            )
            .await?;
        Ok(Asset::from_record(record, tags))
    }

    pub async fn get_asset(&self, id: &str) -> Result<Asset> {
        let record = self.store.get_asset(id).await?;
        let tags = self.store.asset_tags(id).await?;
        Ok(Asset::from_record(record, tags))
    }

    /// 素材列表，最新的在前，每条带标签。
    pub async fn list_assets(&self, filter: &AssetFilter) -> Result<Page<Asset>> {
        let (limit, offset) = validate::page(filter.limit, filter.offset);
        let kind = match non_blank(filter.kind.as_deref()) {
            Some(kind) => Some(validate::one_of("素材类型", kind, ASSET_KINDS)?),
            None => None,
        };
        let source = match non_blank(filter.source.as_deref()) {
            Some(source) => Some(validate::one_of("素材来源", source, ASSET_SOURCES)?),
            None => None,
        };
        let query = AssetQuery {
            product_id: non_blank(filter.product_id.as_deref()).map(str::to_owned),
            kind,
            source,
            // 标签按存的时候一样规整，"白底  主图"也能筛到"白底 主图"
            tag: non_blank(filter.tag.as_deref())
                .map(|tag| tag.split_whitespace().collect::<Vec<_>>().join(" ")),
            keyword: non_blank(filter.keyword.as_deref()).map(str::to_owned),
            starred_only: filter.starred_only,
            limit,
            offset,
        };
        let records = self.store.list_assets(&query).await?;
        let total = self.store.count_assets(&query).await?;
        let ids: Vec<String> = records.iter().map(|r| r.id.clone()).collect();
        let mut tags: HashMap<String, Vec<String>> = HashMap::new();
        for (asset_id, tag) in self.store.tags_of_assets(&ids).await? {
            tags.entry(asset_id).or_default().push(tag);
        }
        let items = records
            .into_iter()
            .map(|record| {
                let asset_tags = tags.remove(&record.id).unwrap_or_default();
                Asset::from_record(record, asset_tags)
            })
            .collect();
        Ok(Page { items, total })
    }

    /// 所有用过的标签和使用次数，标签筛选用。
    pub async fn list_tags(&self) -> Result<Vec<(String, i64)>> {
        Ok(self.store.list_asset_tags().await?)
    }

    /// 整体替换素材的标签。
    pub async fn set_asset_tags(&self, id: &str, tags: &[String]) -> Result<Asset> {
        self.store.get_asset(id).await?;
        let tags = validate::tags(tags)?;
        self.store.set_asset_tags(id, &tags, now_ms()).await?;
        self.get_asset(id).await
    }

    pub async fn star_asset(&self, id: &str, starred: bool) -> Result<()> {
        self.store.get_asset(id).await?;
        self.store.set_asset_starred(id, starred, now_ms()).await?;
        Ok(())
    }

    /// 关联到商品，`None` 取消关联。
    pub async fn link_asset(&self, id: &str, product_id: Option<&str>) -> Result<()> {
        self.store.get_asset(id).await?;
        if let Some(product_id) = product_id {
            self.store.get_product(product_id).await?;
        }
        self.store
            .link_asset_product(id, product_id, now_ms())
            .await?;
        Ok(())
    }

    /// 删除素材。图片文件没有别的素材在用时一起删；删文件失败只记日志，留给缓存清理。
    pub async fn delete_asset(&self, id: &str) -> Result<()> {
        let _guard = self.import_lock.lock().await;
        let record = self.store.delete_asset(id).await?;
        let Some(relative) = record.file_path else {
            return Ok(());
        };
        if self.store.count_assets_with_file(&relative).await? > 0 {
            return Ok(());
        }
        // 缩略图和原图同名、同生同灭。最后删掉的记录不一定带着缩略图路径（比如别处直接登记的），
        // 所以按文件名推出缩略图路径来删
        let mut files = vec![thumb_path(&relative), relative];
        if let Some(thumb) = record.thumb_path
            && !files.contains(&thumb)
        {
            files.push(thumb);
        }
        for file in files {
            let path = self.data_dir.join(&file);
            if let Err(error) = tokio::fs::remove_file(&path).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(path = %path.display(), "删除素材文件失败，留给缓存清理：{error}");
            }
        }
        Ok(())
    }
}

/// 缩略图放在原图旁边：`assets/ab/<sha>.png` → `assets/ab/<sha>.thumb.webp`。
fn thumb_path(relative: &str) -> String {
    let stem = relative.rsplit_once('.').map_or(relative, |(stem, _)| stem);
    format!("{stem}.thumb.webp")
}

fn check_size(len: u64) -> Result<()> {
    if len > MAX_IMAGE_BYTES {
        return Err(invalid(format!(
            "图片超过 {} MB，请压缩后再导入",
            MAX_IMAGE_BYTES / 1024 / 1024
        )));
    }
    if len == 0 {
        return Err(invalid("图片文件是空的"));
    }
    Ok(())
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 清理临时文件；失败只记日志，不影响主流程。
async fn remove_quietly(path: &Path) {
    if let Err(error) = tokio::fs::remove_file(path).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(path = %path.display(), "清理临时文件失败：{error}");
    }
}
