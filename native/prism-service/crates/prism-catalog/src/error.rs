use std::path::Path;

use prism_core::{ApiError, ApiErrorCode};
use prism_imaging::ImagingError;
use prism_store::StoreError;

/// 商品库的错误。
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    /// 输入不符合要求，`String` 是给用户看的说明，写明哪个字段、为什么
    #[error("{0}")]
    Invalid(String),

    #[error(transparent)]
    Store(#[from] StoreError),

    #[error(transparent)]
    Imaging(#[from] ImagingError),

    /// 读表格、对列出错
    #[error(transparent)]
    Table(#[from] prism_io::IoError),

    /// 读用户选的文件失败（不存在、没权限）
    #[error("读取文件「{path}」失败：{source}")]
    ReadFile {
        path: String,
        source: std::io::Error,
    },

    /// 写数据目录失败（磁盘满、没权限）
    #[error("保存素材文件「{path}」失败：{source}")]
    WriteFile {
        path: String,
        source: std::io::Error,
    },
}

impl CatalogError {
    pub(crate) fn read_file(path: &Path, source: std::io::Error) -> Self {
        Self::ReadFile {
            path: path.display().to_string(),
            source,
        }
    }

    pub(crate) fn write_file(path: &Path, source: std::io::Error) -> Self {
        Self::WriteFile {
            path: path.display().to_string(),
            source,
        }
    }
}

pub(crate) fn invalid(message: impl Into<String>) -> CatalogError {
    CatalogError::Invalid(message.into())
}

impl From<CatalogError> for ApiError {
    fn from(error: CatalogError) -> Self {
        match error {
            CatalogError::Invalid(_) | CatalogError::Imaging(_) => {
                ApiError::invalid_input(error.to_string())
            }
            CatalogError::Store(error) => error.into(),
            CatalogError::Table(error) => error.into(),
            CatalogError::ReadFile { ref path, .. } => ApiError::invalid_input(format!(
                "读取文件「{path}」失败，请确认文件存在并且可以打开"
            )),
            // 原始错误只进日志，给用户的说法要能看懂
            CatalogError::WriteFile { .. } => ApiError::new(
                ApiErrorCode::Storage,
                "保存素材文件失败，请检查数据目录所在磁盘的空间和权限",
            ),
        }
    }
}
