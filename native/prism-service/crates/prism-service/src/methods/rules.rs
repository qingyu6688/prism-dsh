//! `rules.*`：规则包、词表、导出模板（原 Tauri 版 `rules_commands.rs`）。
//!
//! 内置规则随程序发布，用户导入的放在数据目录的 `rules/` 下，同 id 的用户版本盖住内置版本。
//! 启动时加载一次，导入、删除后重新加载。

use std::path::PathBuf;
use std::sync::{PoisonError, RwLock};

use prism_core::ApiError;
use prism_rules::{
    Entry, ExportTemplate, LoadProblem, Origin, ResourceKind, RulePack, RuleRegistry, WordList,
    import, remove_user_file,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Context, fail, parse, reply};
use crate::protocol::RpcError;

/// 数据目录里放用户规则文件的子目录。
pub const RULES_DIR: &str = "rules";

/// 加载好的规则和加载时的问题。
#[derive(Debug)]
struct Loaded {
    registry: RuleRegistry,
    problems: Vec<LoadProblem>,
}

/// 当前生效的规则。读多写少，用读写锁。
#[derive(Debug)]
pub(super) struct Rules {
    dir: PathBuf,
    loaded: RwLock<Loaded>,
}

impl Rules {
    pub(super) fn load(dir: PathBuf) -> Self {
        let loaded = load(&dir);
        Self {
            dir,
            loaded: RwLock::new(loaded),
        }
    }

    /// 锁坏了（之前有线程在持锁时崩溃）也照样读，规则是只读数据。
    fn read<T>(&self, f: impl FnOnce(&Loaded) -> T) -> T {
        let guard = self.loaded.read().unwrap_or_else(PoisonError::into_inner);
        f(&guard)
    }

    fn reload(&self) {
        let loaded = load(&self.dir);
        *self.loaded.write().unwrap_or_else(PoisonError::into_inner) = loaded;
    }
}

fn load(dir: &std::path::Path) -> Loaded {
    let (registry, problems) = RuleRegistry::load(Some(dir));
    for problem in &problems {
        tracing::warn!(file = %problem.file, "规则文件没有加载：{:?}", problem.problems);
    }
    Loaded { registry, problems }
}

/// 一份生效的规则和它的来源。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuleRow<T> {
    origin: Origin,
    /// 被它盖住的另一个版本的修订号
    shadowed_revision: Option<u32>,
    value: T,
}

fn row<T: Clone>(entry: &Entry<T>) -> RuleRow<T> {
    RuleRow {
        origin: entry.origin.clone(),
        shadowed_revision: entry.shadowed_revision,
        value: entry.value.clone(),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Overview {
    packs: Vec<RuleRow<RulePack>>,
    word_lists: Vec<RuleRow<WordList>>,
    export_templates: Vec<RuleRow<ExportTemplate>>,
    /// 加载时有问题、被跳过的文件
    problems: Vec<LoadProblem>,
}

#[derive(Debug, Deserialize)]
struct ImportFile {
    path: PathBuf,
}

#[derive(Debug, Serialize)]
struct Imported {
    kind: ResourceKind,
    id: String,
    revision: u32,
}

#[derive(Debug, Deserialize)]
struct RemoveUser {
    kind: ResourceKind,
    id: String,
}

#[derive(Debug, Serialize)]
struct Removed {
    removed: bool,
}

pub(super) fn dispatch(
    ctx: &Context,
    method: &str,
    name: &str,
    params: Value,
) -> Result<Value, RpcError> {
    let rules = &ctx.rules;
    match name {
        "list" => reply(rules.read(|loaded| {
            Overview {
                packs: loaded.registry.rule_packs().map(row).collect(),
                word_lists: loaded.registry.word_lists().map(row).collect(),
                export_templates: loaded
                    .registry
                    .export_templates(None)
                    .into_iter()
                    .map(row)
                    .collect(),
                problems: loaded.problems.clone(),
            }
        })),
        "importFile" => {
            let p: ImportFile = parse(params)?;
            let bytes = std::fs::read(&p.path).map_err(|error| {
                tracing::warn!(path = %p.path.display(), %error, "读取规则文件失败");
                RpcError::from(ApiError::invalid_input(format!(
                    "读取文件「{}」失败，请确认文件存在并且可以打开",
                    p.path.display()
                )))
            })?;
            // 导入失败时规则没有变化，不用重新加载
            let resource = rules
                .read(|loaded| import(&rules.dir, &bytes, &loaded.registry))
                .map_err(fail)?;
            rules.reload();
            reply(Imported {
                kind: resource.kind(),
                id: resource.id().to_owned(),
                revision: resource.revision(),
            })
        }
        "removeUser" => {
            let p: RemoveUser = parse(params)?;
            let removed = remove_user_file(&rules.dir, p.kind, &p.id).map_err(fail)?;
            rules.reload();
            reply(Removed { removed })
        }
        _ => Err(RpcError::method_not_found(method)),
    }
}
