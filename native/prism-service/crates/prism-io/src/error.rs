use prism_core::{ApiError, ApiErrorCode};

/// 表格读写的错误。消息直接给用户看。
#[derive(Debug, thiserror::Error)]
pub enum IoError {
    #[error("不支持的表格格式「{0}」，支持 xlsx、xlsm、xls、ods、csv、tsv")]
    Unsupported(String),

    #[error("表格文件超过 {limit_mb} MB，请拆分后再导入")]
    TooLarge { limit_mb: u64 },

    #[error("表格超过 {limit} 行，请分批导入")]
    TooManyRows { limit: usize },

    #[error("表格是空的")]
    Empty,

    #[error("找不到工作表「{0}」")]
    SheetNotFound(String),

    #[error("指定的表头行 {line} 超出了表格范围（共 {total} 行）")]
    HeaderOutOfRange { line: usize, total: usize },

    /// 文件打不开，多半是损坏或者不是表格文件
    #[error("表格文件打不开：{0}")]
    Corrupted(String),

    /// 列映射和表格对不上
    #[error("{0}")]
    Mapping(String),

    #[error("读取文件「{path}」失败：{source}")]
    Read {
        path: String,
        source: std::io::Error,
    },

    #[error("写入文件「{path}」失败：{source}")]
    Write {
        path: String,
        source: std::io::Error,
    },

    /// 生成 xlsx / ZIP 内部出错
    #[error("生成文件失败：{0}")]
    Generate(String),
}

impl IoError {
    pub(crate) fn read(path: &std::path::Path, source: std::io::Error) -> Self {
        Self::Read {
            path: path.display().to_string(),
            source,
        }
    }

    pub(crate) fn write(path: &std::path::Path, source: std::io::Error) -> Self {
        Self::Write {
            path: path.display().to_string(),
            source,
        }
    }
}

impl From<IoError> for ApiError {
    fn from(error: IoError) -> Self {
        match error {
            IoError::Write { .. } => ApiError::new(
                ApiErrorCode::Storage,
                "写入文件失败，请检查目标位置的磁盘空间和权限",
            ),
            // 原始错误只进日志
            IoError::Generate(_) => ApiError::internal("生成文件失败，请稍后重试"),
            other => ApiError::invalid_input(other.to_string()),
        }
    }
}
