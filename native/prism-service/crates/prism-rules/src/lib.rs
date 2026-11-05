//! 平台规则包和词表：格式定义、加载、校验、按修订号覆盖（详细设计第 10 章）。
//!
//! 规则是数据文件（仓库 `rules/`），新增或修正平台规则改 JSON 就行。这里只负责把文件读对、
//! 查出格式问题、决定用哪个版本；按规则检查文案和图片由各业务模块做。

mod builtin;
mod measure;
mod model;
mod parse;
mod registry;

use std::path::Path;

use prism_core::{ApiError, ApiErrorCode};

pub use measure::measure;
pub use model::{
    BannedWords, BulletRules, CellFormat, ExportColumn, ExportTemplate, FORMAT_VERSION,
    ImageFileFormat, ImageRules, ImageSpec, LengthUnit, LocalizedText, Market, MatchMode,
    RequiredAttribute, Resource, ResourceKind, RowPer, RulePack, SOURCE_ROOTS, Severity, Status,
    TextLimit, TextRules, WhiteBackground, WordEntry, WordList,
};
pub use parse::{Problem, validate};
pub use registry::{
    BannedWord, Entry, LoadProblem, Origin, RuleRegistry, import, remove_user_file,
};

/// 导入、删除规则文件时的错误。
#[derive(Debug, thiserror::Error)]
pub enum RulesError {
    /// 文件内容有问题，列出全部问题
    #[error("文件有 {} 处问题：{}", .0.len(), join(.0))]
    Invalid(Vec<Problem>),

    /// 导入的版本比当前生效的旧
    #[error("{id} 当前用的是修订号 {current} 的版本，导入的是 {imported}，比现在的旧")]
    Outdated {
        id: String,
        current: u32,
        imported: u32,
    },

    #[error("读写「{path}」失败：{source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

impl RulesError {
    pub(crate) fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.display().to_string(),
            source,
        }
    }
}

fn join(problems: &[Problem]) -> String {
    problems
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("；")
}

impl From<RulesError> for ApiError {
    fn from(error: RulesError) -> Self {
        match &error {
            RulesError::Invalid(problems) => {
                let mut api = ApiError::invalid_input(error.to_string());
                // 问题明细给界面逐条显示
                api.detail = serde_json::to_value(problems).ok();
                api
            }
            RulesError::Outdated { .. } => ApiError::invalid_input(error.to_string()),
            RulesError::Io { .. } => ApiError::new(
                ApiErrorCode::Storage,
                "保存规则文件失败，请检查数据目录所在磁盘的空间和权限",
            ),
        }
    }
}

/// 某类文件的 JSON Schema（美化过的 JSON 文本），和仓库 `rules/schema/` 下的文件一致。
pub fn json_schema(kind: ResourceKind) -> String {
    let schema = match kind {
        ResourceKind::RulePack => schemars::schema_for!(RulePack),
        ResourceKind::WordList => schemars::schema_for!(WordList),
        ResourceKind::ExportTemplate => schemars::schema_for!(ExportTemplate),
    };
    // Schema 由固定的类型生成，序列化不会失败
    let mut text = serde_json::to_string_pretty(&schema).unwrap_or_default();
    text.push('\n');
    text
}
