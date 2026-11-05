//! 内置的规则文件，编译进程序。新增文件时在 [`BUILTIN`] 里加一行，测试会检查有没有漏。

use crate::model::ResourceKind;

pub(crate) struct Builtin {
    /// 仓库里的相对路径，报错时显示
    pub path: &'static str,
    pub kind: ResourceKind,
    pub content: &'static str,
}

macro_rules! builtin {
    ($kind:ident, $dir:literal, $file:literal) => {
        Builtin {
            path: concat!("rules/", $dir, "/", $file),
            kind: ResourceKind::$kind,
            content: include_str!(concat!("../../../rules/", $dir, "/", $file)),
        }
    };
}

pub(crate) const BUILTIN: &[Builtin] = &[
    builtin!(WordList, "words", "ad_superlatives_zh.json"),
    builtin!(WordList, "words", "promo_en.json"),
    builtin!(RulePack, "packs", "amazon.json"),
    builtin!(RulePack, "packs", "temu.json"),
    builtin!(RulePack, "packs", "tiktok_shop.json"),
    builtin!(RulePack, "packs", "shopee.json"),
    builtin!(RulePack, "packs", "taobao.json"),
    builtin!(RulePack, "packs", "pinduoduo.json"),
    builtin!(RulePack, "packs", "douyin_shop.json"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_file_is_registered() {
        for kind in [
            ResourceKind::RulePack,
            ResourceKind::WordList,
            ResourceKind::ExportTemplate,
        ] {
            let dir = format!(
                "{}/../../rules/{}",
                env!("CARGO_MANIFEST_DIR"),
                kind.dir_name()
            );
            // 还没有内置文件的类型可以没有目录
            let Ok(entries) = std::fs::read_dir(&dir) else {
                assert!(BUILTIN.iter().all(|b| b.kind != kind));
                continue;
            };
            let mut on_disk: Vec<String> = entries
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .filter(|name| name.ends_with(".json"))
                .map(|name| format!("rules/{}/{name}", kind.dir_name()))
                .collect();
            on_disk.sort();
            let mut registered: Vec<String> = BUILTIN
                .iter()
                .filter(|b| b.kind == kind)
                .map(|b| b.path.to_owned())
                .collect();
            registered.sort();
            assert_eq!(
                on_disk,
                registered,
                "rules/{} 和 BUILTIN 不一致",
                kind.dir_name()
            );
        }
    }
}
