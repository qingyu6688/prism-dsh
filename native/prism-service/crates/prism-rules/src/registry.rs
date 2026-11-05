//! 规则注册表：加载内置和用户文件，按修订号决定用哪个版本（详细设计 10.1、10.6）。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::RulesError;
use crate::builtin::BUILTIN;
use crate::model::{
    ExportTemplate, MatchMode, Resource, ResourceKind, RulePack, Severity, WordList,
};
use crate::parse::{Problem, is_identifier, validate};

/// 生效版本来自哪里。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Origin {
    /// 随应用发布的内置版本
    Builtin,
    /// 用户导入的文件
    User { path: PathBuf },
}

/// 生效的一份规则。
#[derive(Debug, Clone, PartialEq)]
pub struct Entry<T> {
    pub value: T,
    pub origin: Origin,
    /// 被它盖住的另一个版本（内置或用户文件）的修订号，设置页提示用
    pub shadowed_revision: Option<u32>,
}

/// 加载时有问题、被跳过的文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoadProblem {
    /// 内置文件是仓库里的相对路径，用户文件是完整路径
    pub file: String,
    pub problems: Vec<Problem>,
}

/// 店铺适用的一个违禁词。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BannedWord {
    pub word: String,
    pub severity: Severity,
    pub suggestion: Option<String>,
    pub match_mode: MatchMode,
    /// 来自哪个词表；`None` 表示规则包自己的词
    pub list: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RuleRegistry {
    packs: BTreeMap<String, Entry<RulePack>>,
    word_lists: BTreeMap<String, Entry<WordList>>,
    export_templates: BTreeMap<String, Entry<ExportTemplate>>,
}

/// 加载过程中的一个候选版本。
struct Candidate {
    resource: Resource,
    origin: Origin,
    file: String,
}

impl RuleRegistry {
    /// 加载内置文件和用户目录（`<data_dir>/rules`）里的文件。有问题的文件跳过并列在返回的问题里，
    /// 不影响其他文件；用户文件有问题时自动用内置版本。
    pub fn load(user_dir: Option<&Path>) -> (Self, Vec<LoadProblem>) {
        let mut problems = Vec::new();
        let mut candidates = Vec::new();
        for builtin in BUILTIN {
            match parse_file(builtin.content.as_bytes(), builtin.kind, builtin.path) {
                Ok(resource) => candidates.push(Candidate {
                    resource,
                    origin: Origin::Builtin,
                    file: builtin.path.to_owned(),
                }),
                Err(found) => problems.push(LoadProblem {
                    file: builtin.path.to_owned(),
                    problems: found,
                }),
            }
        }
        if let Some(user_dir) = user_dir {
            read_user_files(user_dir, &mut candidates, &mut problems);
        }

        let mut registry = Self::default();
        let (lists, others): (Vec<_>, Vec<_>) = candidates
            .into_iter()
            .partition(|c| c.resource.kind() == ResourceKind::WordList);
        let (templates, packs): (Vec<_>, Vec<_>) = others
            .into_iter()
            .partition(|c| c.resource.kind() == ResourceKind::ExportTemplate);
        for candidate in templates {
            if let Resource::ExportTemplate(template) = candidate.resource {
                choose(
                    &mut registry.export_templates,
                    template.id.clone(),
                    template.revision,
                    template,
                    candidate.origin,
                );
            }
        }
        for candidate in lists {
            if let Resource::WordList(list) = candidate.resource {
                choose(
                    &mut registry.word_lists,
                    list.id.clone(),
                    list.revision,
                    list,
                    candidate.origin,
                );
            }
        }
        // 词表定下来以后再看规则包引用的词表在不在
        for candidate in packs {
            if let Resource::RulePack(pack) = candidate.resource {
                let missing = registry.missing_lists(&pack);
                if !missing.is_empty() {
                    problems.push(LoadProblem {
                        file: candidate.file,
                        problems: missing,
                    });
                    continue;
                }
                choose(
                    &mut registry.packs,
                    pack.id.clone(),
                    pack.revision,
                    *pack,
                    candidate.origin,
                );
            }
        }
        (registry, problems)
    }

    /// 某个平台的规则包。
    pub fn rule_pack(&self, platform: &str) -> Option<&Entry<RulePack>> {
        self.packs.get(platform)
    }

    /// 所有规则包，按平台标识排序。
    pub fn rule_packs(&self) -> impl Iterator<Item = &Entry<RulePack>> {
        self.packs.values()
    }

    pub fn word_list(&self, id: &str) -> Option<&Entry<WordList>> {
        self.word_lists.get(id)
    }

    pub fn word_lists(&self) -> impl Iterator<Item = &Entry<WordList>> {
        self.word_lists.values()
    }

    pub fn export_template(&self, id: &str) -> Option<&Entry<ExportTemplate>> {
        self.export_templates.get(id)
    }

    /// 导出模板，`platform` 不为空时只要这个平台的。
    pub fn export_templates(&self, platform: Option<&str>) -> Vec<&Entry<ExportTemplate>> {
        self.export_templates
            .values()
            .filter(|entry| platform.is_none_or(|p| entry.value.platform.as_deref() == Some(p)))
            .collect()
    }

    /// 平台适用的全部违禁词：先是规则包自己的，再是引用的词表；同一个词只保留第一次出现的。
    pub fn banned_words(&self, platform: &str) -> Option<Vec<BannedWord>> {
        let pack = &self.packs.get(platform)?.value;
        let mut seen = HashSet::new();
        let mut words = Vec::new();
        let own = pack
            .banned_words
            .words
            .iter()
            .map(|entry| (entry, pack.banned_words.match_mode, None));
        let listed = pack
            .banned_words
            .lists
            .iter()
            .filter_map(|id| self.word_lists.get(id))
            .flat_map(|entry| {
                let list = &entry.value;
                list.words
                    .iter()
                    .map(move |word| (word, list.match_mode, Some(list.id.clone())))
            });
        for (entry, match_mode, list) in own.chain(listed) {
            if seen.insert(entry.word.to_lowercase()) {
                words.push(BannedWord {
                    word: entry.word.clone(),
                    severity: entry.severity,
                    suggestion: entry.suggestion.clone(),
                    match_mode,
                    list,
                });
            }
        }
        Some(words)
    }

    /// 当前生效版本的修订号。
    fn current_revision(&self, kind: ResourceKind, id: &str) -> Option<u32> {
        match kind {
            ResourceKind::RulePack => self.packs.get(id).map(|e| e.value.revision),
            ResourceKind::WordList => self.word_lists.get(id).map(|e| e.value.revision),
            ResourceKind::ExportTemplate => self.export_templates.get(id).map(|e| e.value.revision),
        }
    }

    fn missing_lists(&self, pack: &RulePack) -> Vec<Problem> {
        pack.banned_words
            .lists
            .iter()
            .enumerate()
            .filter(|(_, id)| !self.word_lists.contains_key(*id))
            .map(|(index, id)| Problem {
                path: format!("bannedWords.lists[{index}]"),
                message: format!("引用的词表 {id} 不存在"),
            })
            .collect()
    }
}

/// 同一个 ID 保留修订号大的；一样大时用户文件优先（用户特意导入的）。
fn choose<T>(
    map: &mut BTreeMap<String, Entry<T>>,
    id: String,
    revision: u32,
    value: T,
    origin: Origin,
) where
    T: HasRevision,
{
    let user = matches!(origin, Origin::User { .. });
    match map.get_mut(&id) {
        None => {
            map.insert(
                id,
                Entry {
                    value,
                    origin,
                    shadowed_revision: None,
                },
            );
        }
        Some(current) => {
            let current_revision = current.value.revision();
            if revision > current_revision || (revision == current_revision && user) {
                *current = Entry {
                    value,
                    origin,
                    shadowed_revision: Some(current_revision),
                };
            } else {
                current.shadowed_revision = Some(revision);
            }
        }
    }
}

trait HasRevision {
    fn revision(&self) -> u32;
}

impl HasRevision for RulePack {
    fn revision(&self) -> u32 {
        self.revision
    }
}

impl HasRevision for ExportTemplate {
    fn revision(&self) -> u32 {
        self.revision
    }
}

impl HasRevision for WordList {
    fn revision(&self) -> u32 {
        self.revision
    }
}

/// 解析一个文件，并检查它放对了目录、文件名和 `id` 一致。
fn parse_file(bytes: &[u8], kind: ResourceKind, file: &str) -> Result<Resource, Vec<Problem>> {
    let resource = validate(bytes)?;
    let mut problems = Vec::new();
    if resource.kind() != kind {
        problems.push(Problem {
            path: "kind".into(),
            message: format!(
                "这是{}，应该放在 {} 目录",
                resource.kind().label(),
                resource.kind().dir_name()
            ),
        });
    }
    let stem = Path::new(file)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    if stem != resource.id() {
        problems.push(Problem {
            path: "id".into(),
            message: format!("文件名应该是 {}.json，和 id 一致", resource.id()),
        });
    }
    if problems.is_empty() {
        Ok(resource)
    } else {
        Err(problems)
    }
}

fn read_user_files(
    user_dir: &Path,
    candidates: &mut Vec<Candidate>,
    problems: &mut Vec<LoadProblem>,
) {
    for kind in [
        ResourceKind::WordList,
        ResourceKind::RulePack,
        ResourceKind::ExportTemplate,
    ] {
        let dir = user_dir.join(kind.dir_name());
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            // 没导入过就没有这个目录，正常情况
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                problems.push(io_problem(&dir, &error));
                continue;
            }
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        paths.sort();
        for path in paths {
            let file = path.display().to_string();
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    problems.push(io_problem(&path, &error));
                    continue;
                }
            };
            match parse_file(&bytes, kind, &file) {
                Ok(resource) => candidates.push(Candidate {
                    resource,
                    origin: Origin::User { path },
                    file,
                }),
                Err(found) => problems.push(LoadProblem {
                    file,
                    problems: found,
                }),
            }
        }
    }
}

fn io_problem(path: &Path, error: &std::io::Error) -> LoadProblem {
    LoadProblem {
        file: path.display().to_string(),
        problems: vec![Problem {
            path: String::new(),
            message: format!("读取失败：{error}"),
        }],
    }
}

/// 导入一个文件到用户目录。校验通过、引用的词表存在、修订号不比当前生效版本小才写入。
/// 写入后调用方重新 `load` 生效。
pub fn import(
    user_dir: &Path,
    bytes: &[u8],
    registry: &RuleRegistry,
) -> Result<Resource, RulesError> {
    let resource = validate(bytes).map_err(RulesError::Invalid)?;
    if let Resource::RulePack(pack) = &resource {
        let missing = registry.missing_lists(pack);
        if !missing.is_empty() {
            return Err(RulesError::Invalid(missing));
        }
    }
    if let Some(current) = registry.current_revision(resource.kind(), resource.id())
        && resource.revision() < current
    {
        return Err(RulesError::Outdated {
            id: resource.id().to_owned(),
            current,
            imported: resource.revision(),
        });
    }
    let dir = user_dir.join(resource.kind().dir_name());
    std::fs::create_dir_all(&dir).map_err(|source| RulesError::io(&dir, source))?;
    let target = dir.join(format!("{}.json", resource.id()));
    let temp = dir.join(format!(".{}.json.tmp", resource.id()));
    // 先写临时文件再改名，写到一半失败不会留下坏文件；改名会覆盖旧的用户版本
    if let Err(source) = std::fs::write(&temp, bytes).and_then(|()| std::fs::rename(&temp, &target))
    {
        if let Err(cleanup) = std::fs::remove_file(&temp)
            && cleanup.kind() != std::io::ErrorKind::NotFound
        {
            return Err(RulesError::io(&temp, cleanup));
        }
        return Err(RulesError::io(&target, source));
    }
    Ok(resource)
}

/// 删除用户版本，回到内置版本。文件不存在返回 `false`。
pub fn remove_user_file(user_dir: &Path, kind: ResourceKind, id: &str) -> Result<bool, RulesError> {
    // id 会拼进路径，先确认格式，避免 `../` 之类
    if !is_identifier(id) {
        return Err(RulesError::Invalid(vec![Problem {
            path: "id".into(),
            message: format!("「{id}」不是有效的 ID"),
        }]));
    }
    let path = user_dir.join(kind.dir_name()).join(format!("{id}.json"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(RulesError::io(&path, source)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(u32);

    impl HasRevision for Fake {
        fn revision(&self) -> u32 {
            self.0
        }
    }

    fn user() -> Origin {
        Origin::User {
            path: PathBuf::from("x.json"),
        }
    }

    #[test]
    fn higher_revision_wins_and_ties_go_to_the_user() {
        // 应用升级带来更新的内置版本，旧的用户文件不能把它挡住
        let mut map = BTreeMap::new();
        choose(&mut map, "a".into(), 3, Fake(3), Origin::Builtin);
        choose(&mut map, "a".into(), 2, Fake(2), user());
        assert_eq!((map["a"].value.0, &map["a"].origin), (3, &Origin::Builtin));
        assert_eq!(map["a"].shadowed_revision, Some(2));

        // 修订号一样时用户文件优先，不管谁先加载
        let mut map = BTreeMap::new();
        choose(&mut map, "b".into(), 1, Fake(1), user());
        choose(&mut map, "b".into(), 1, Fake(1), Origin::Builtin);
        assert_eq!(map["b"].origin, user());
        choose(&mut map, "b".into(), 4, Fake(4), user());
        assert_eq!((map["b"].value.0, map["b"].shadowed_revision), (4, Some(1)));
    }
}
