//! 规则包的集成测试：内置文件、JSON Schema、校验提示、用户版本覆盖。

// 测试里的辅助函数出错就该直接失败，用 unwrap 最直接
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use prism_core::{ApiError, ApiErrorCode};
use prism_rules::{
    LengthUnit, MatchMode, Origin, Problem, ResourceKind, RuleRegistry, RulesError, Severity,
    import, json_schema, measure, remove_user_file, validate,
};
use serde_json::{Value, json};
use tempfile::TempDir;

fn repo_file(relative: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// 内置的 Amazon 规则包，测试在它的基础上改。
fn amazon() -> Value {
    serde_json::from_str(&std::fs::read_to_string(repo_file("rules/packs/amazon.json")).unwrap())
        .unwrap()
}

fn problems(value: &Value) -> Vec<Problem> {
    validate(value.to_string().as_bytes()).unwrap_err()
}

fn messages(value: &Value) -> Vec<String> {
    problems(value).iter().map(ToString::to_string).collect()
}

#[test]
fn builtin_rules_all_load() {
    let (registry, problems) = RuleRegistry::load(None);
    assert!(problems.is_empty(), "{problems:#?}");
    let packs: Vec<&str> = registry.rule_packs().map(|e| e.value.id.as_str()).collect();
    assert_eq!(
        packs,
        [
            "amazon",
            "douyin_shop",
            "pinduoduo",
            "shopee",
            "taobao",
            "temu",
            "tiktok_shop"
        ]
    );
    assert_eq!(registry.word_lists().count(), 2);
    let amazon = registry.rule_pack("amazon").unwrap();
    assert_eq!(amazon.origin, Origin::Builtin);
    assert_eq!(amazon.shadowed_revision, None);
    let title = amazon.value.text.title.as_ref().unwrap();
    assert_eq!((title.max_length, title.unit), (200, LengthUnit::Char));

    // 淘宝标题 60 字符 = 30 个汉字
    let taobao = &registry.rule_pack("taobao").unwrap().value;
    let limit = taobao.text.title.as_ref().unwrap();
    assert_eq!(measure(&"字".repeat(30), limit.unit), 60);
}

#[test]
fn banned_words_combine_pack_and_lists() {
    let (registry, _) = RuleRegistry::load(None);
    let words = registry.banned_words("amazon").unwrap();
    let own = words.iter().find(|w| w.word == "FDA approved").unwrap();
    assert_eq!(
        (own.list.as_deref(), own.match_mode),
        (None, MatchMode::Word)
    );
    let listed = words.iter().find(|w| w.word == "free shipping").unwrap();
    assert_eq!(listed.list.as_deref(), Some("promo_en"));
    assert_eq!(listed.severity, Severity::Error);

    let taobao = registry.banned_words("taobao").unwrap();
    let best = taobao.iter().find(|w| w.word == "最佳").unwrap();
    assert_eq!(best.match_mode, MatchMode::Substring);
    assert_eq!(best.suggestion.as_deref(), Some("优质"));
    assert!(registry.banned_words("ebay").is_none());
}

#[test]
fn schema_files_match_the_code() {
    // 改了规则包的字段后跑 `PRISM_UPDATE_SCHEMA=1 cargo test -p prism-rules` 更新
    for (kind, file) in [
        (ResourceKind::RulePack, "rules/schema/rule-pack.schema.json"),
        (ResourceKind::WordList, "rules/schema/word-list.schema.json"),
        (
            ResourceKind::ExportTemplate,
            "rules/schema/export-template.schema.json",
        ),
    ] {
        let generated = json_schema(kind);
        let path = repo_file(file);
        if std::env::var_os("PRISM_UPDATE_SCHEMA").is_some() {
            std::fs::write(&path, &generated).unwrap();
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            committed.replace("\r\n", "\n"),
            generated,
            "{file} 和代码不一致，跑 PRISM_UPDATE_SCHEMA=1 cargo test -p prism-rules 更新"
        );
    }
}

// ---------- 校验提示 ----------

#[test]
fn format_errors_point_at_the_field() {
    let mut typo = amazon();
    typo["images"]["main"]["maxCont"] = json!(9);
    assert_eq!(
        messages(&typo),
        ["images.main.maxCont：不认识的字段 maxCont，请检查拼写"]
    );

    let mut bad_enum = amazon();
    bad_enum["images"]["main"]["whiteBackground"] = json!("square");
    assert_eq!(
        messages(&bad_enum),
        ["images.main.whiteBackground：不支持的值 square，可选：none / first / all"]
    );

    let mut negative = amazon();
    negative["text"]["title"]["maxLength"] = json!(-1);
    assert_eq!(
        messages(&negative),
        ["text.title.maxLength：类型不对，应该是非负整数"]
    );

    let mut missing = amazon();
    missing.as_object_mut().unwrap().remove("market");
    assert_eq!(messages(&missing), ["缺少必填字段 market"]);
}

#[test]
fn content_errors_are_listed_together() {
    let mut pack = amazon();
    pack["images"]["main"]["minCount"] = json!(5);
    pack["images"]["main"]["maxCount"] = json!(3);
    pack["images"]["main"]["aspectRatios"] = json!(["1x1"]);
    pack["images"]["main"]["minProductRatio"] = json!(85);
    pack["name"] = json!({ "zh-CN": "亚马逊" });
    pack["updatedAt"] = json!("2026-02-30");
    pack["sites"] = json!(["us", "US"]);
    pack["bannedWords"]["words"] = json!([
        { "word": "Free", "severity": "error" },
        { "word": "free", "severity": "warning" }
    ]);
    let found = messages(&pack);
    for expected in [
        "images.main.maxCount：不能小于 minCount（5）",
        "images.main.aspectRatios[0]：「1x1」应该写成 宽:高，如 1:1、3:4",
        "images.main.minProductRatio：应该在 0～1 之间，如 0.85",
        "name：缺少 en 的文字",
        "updatedAt：「2026-02-30」不是有效日期，格式是 YYYY-MM-DD",
        "sites[0]：「us」应该是两位大写地区代码，如 US",
        "bannedWords.words[1].word：「free」重复了",
    ] {
        assert!(
            found.iter().any(|m| m == expected),
            "缺少：{expected}\n实际：{found:#?}"
        );
    }
}

#[test]
fn file_level_errors() {
    assert_eq!(
        validate(b"{ \"kind\": ").unwrap_err()[0].message,
        "不是有效的 JSON：第 1 行第 10 列附近有错误"
    );
    assert_eq!(
        messages(&json!({ "kind": "report_template" })),
        ["kind：不支持的文件类型 report_template，可选：rule_pack / word_list / export_template"]
    );
    let mut future = amazon();
    future["formatVersion"] = json!(2);
    future["somethingNew"] = json!(true);
    assert_eq!(
        messages(&future),
        ["formatVersion：文件格式版本 2 比应用支持的 1 新，请升级应用"]
    );
    let mut domestic = amazon();
    domestic["market"] = json!("domestic");
    assert_eq!(messages(&domestic), ["sites：国内平台不填站点"]);
}

// ---------- 用户版本 ----------

fn with_revision(revision: u32) -> Vec<u8> {
    let mut pack = amazon();
    pack["revision"] = json!(revision);
    pack["text"]["title"]["maxLength"] = json!(150);
    pack.to_string().into_bytes()
}

#[test]
fn newer_user_versions_override_builtin_ones() {
    let dir = TempDir::new().unwrap();
    let (registry, _) = RuleRegistry::load(Some(dir.path()));
    import(dir.path(), &with_revision(2), &registry).unwrap();

    let (registry, problems) = RuleRegistry::load(Some(dir.path()));
    assert!(problems.is_empty(), "{problems:#?}");
    let amazon = registry.rule_pack("amazon").unwrap();
    assert!(
        matches!(&amazon.origin, Origin::User { path } if path.ends_with("packs/amazon.json") || path.ends_with("packs\\amazon.json"))
    );
    assert_eq!(amazon.value.text.title.as_ref().unwrap().max_length, 150);
    assert_eq!(amazon.shadowed_revision, Some(1));

    // 比生效版本旧的不让导入
    let outdated = import(dir.path(), &with_revision(1), &registry).unwrap_err();
    assert!(matches!(
        outdated,
        RulesError::Outdated {
            current: 2,
            imported: 1,
            ..
        }
    ));
    assert_eq!(
        outdated.to_string(),
        "amazon 当前用的是修订号 2 的版本，导入的是 1，比现在的旧"
    );

    // 删掉用户版本，回到内置
    assert!(remove_user_file(dir.path(), ResourceKind::RulePack, "amazon").unwrap());
    assert!(!remove_user_file(dir.path(), ResourceKind::RulePack, "amazon").unwrap());
    let (registry, _) = RuleRegistry::load(Some(dir.path()));
    assert_eq!(
        registry.rule_pack("amazon").unwrap().origin,
        Origin::Builtin
    );
    assert!(remove_user_file(dir.path(), ResourceKind::RulePack, "../amazon").is_err());
}

#[test]
fn same_revision_user_file_wins() {
    let dir = TempDir::new().unwrap();
    let (registry, _) = RuleRegistry::load(Some(dir.path()));
    import(dir.path(), &with_revision(1), &registry).unwrap();
    let (registry, _) = RuleRegistry::load(Some(dir.path()));
    let amazon = registry.rule_pack("amazon").unwrap();
    assert!(matches!(amazon.origin, Origin::User { .. }));
    assert_eq!(amazon.value.text.title.as_ref().unwrap().max_length, 150);
    assert_eq!(amazon.shadowed_revision, Some(1));
}

#[test]
fn broken_user_files_fall_back_to_builtin() {
    let dir = TempDir::new().unwrap();
    let packs = dir.path().join("packs");
    std::fs::create_dir_all(&packs).unwrap();
    std::fs::write(packs.join("amazon.json"), "{ 坏了").unwrap();
    // 文件名和 id 不一致
    std::fs::write(packs.join("amzn.json"), with_revision(5)).unwrap();
    // 放错目录的词表
    std::fs::write(
        packs.join("promo_en.json"),
        std::fs::read(repo_file("rules/words/promo_en.json")).unwrap(),
    )
    .unwrap();

    let (registry, problems) = RuleRegistry::load(Some(dir.path()));
    assert_eq!(
        registry.rule_pack("amazon").unwrap().origin,
        Origin::Builtin
    );
    assert_eq!(problems.len(), 3, "{problems:#?}");
    let all: Vec<String> = problems
        .iter()
        .flat_map(|p| p.problems.iter().map(ToString::to_string))
        .collect();
    assert!(all.iter().any(|m| m.starts_with("不是有效的 JSON")));
    assert!(all.contains(&"id：文件名应该是 amazon.json，和 id 一致".to_owned()));
    assert!(all.contains(&"kind：这是词表，应该放在 words 目录".to_owned()));
}

#[test]
fn rule_packs_must_reference_existing_word_lists() {
    let dir = TempDir::new().unwrap();
    let (registry, _) = RuleRegistry::load(Some(dir.path()));
    let mut pack = amazon();
    pack["revision"] = json!(3);
    pack["bannedWords"]["lists"] = json!(["promo_en", "promo_fr"]);
    let error = import(dir.path(), pack.to_string().as_bytes(), &registry).unwrap_err();
    let RulesError::Invalid(found) = &error else {
        panic!("{error:?}");
    };
    assert_eq!(
        found[0].to_string(),
        "bannedWords.lists[1]：引用的词表 promo_fr 不存在"
    );
    assert!(!dir.path().join("packs").exists());

    // 给界面的错误带明细
    let api: ApiError = error.into();
    assert_eq!(api.code, ApiErrorCode::InvalidInput);
    assert_eq!(api.detail.unwrap()[0]["path"], "bannedWords.lists[1]");
}

#[test]
fn imported_word_lists_take_effect_for_rule_packs() {
    let dir = TempDir::new().unwrap();
    let (registry, _) = RuleRegistry::load(Some(dir.path()));
    let mut list: Value = serde_json::from_str(
        &std::fs::read_to_string(repo_file("rules/words/promo_en.json")).unwrap(),
    )
    .unwrap();
    list["revision"] = json!(2);
    list["words"] = json!([{ "word": "mega deal", "severity": "error" }]);
    import(dir.path(), list.to_string().as_bytes(), &registry).unwrap();

    let (registry, _) = RuleRegistry::load(Some(dir.path()));
    let words = registry.banned_words("temu").unwrap();
    assert_eq!(words.len(), 1);
    assert_eq!(words[0].word, "mega deal");
}

// ---------- 导出模板 ----------

fn template() -> Value {
    json!({
        "kind": "export_template", "formatVersion": 1, "id": "temu_basic", "revision": 1,
        "name": { "zh-CN": "Temu 基础刊登表", "en": "Temu basic listing" },
        "status": "draft", "updatedAt": "2026-11-04",
        "platform": "temu",
        "sheetName": "Template",
        "preamble": [["Version=2026.1"]],
        "rowPer": "sku",
        "columns": [
            { "header": "Product Name", "source": "copy.title", "required": true },
            { "header": "SKU", "source": "sku.code" },
            { "header": "Price", "source": "sku.price", "format": "money" },
            { "header": "Main Image", "source": "image.main[0]" },
            { "header": "Condition", "source": "const:New" }
        ]
    })
}

#[test]
fn export_templates_are_validated_and_imported() {
    let dir = TempDir::new().unwrap();
    let (registry, _) = RuleRegistry::load(Some(dir.path()));
    assert!(registry.export_templates(None).is_empty());
    import(dir.path(), template().to_string().as_bytes(), &registry).unwrap();
    let (registry, problems) = RuleRegistry::load(Some(dir.path()));
    assert!(problems.is_empty(), "{problems:#?}");
    let temu = registry.export_templates(Some("temu"));
    assert_eq!(temu.len(), 1);
    assert_eq!(
        temu[0].value.columns[2].format,
        prism_rules::CellFormat::Money
    );
    assert!(registry.export_templates(Some("amazon")).is_empty());
    assert!(registry.export_template("temu_basic").is_some());

    let mut bad = template();
    bad["sheetName"] = json!("Sheet/1");
    bad["preamble"] = json!([["a", "b", "c", "d", "e", "f"]]);
    bad["columns"][1]["header"] = json!("Product Name");
    bad["columns"][2]["source"] = json!("item.price");
    let found = messages(&bad);
    for expected in [
        "sheetName：工作表名称要 1～31 个字，不能含 [ ] : * ? / \\",
        "preamble[0]：不能比列数（5）多",
        "columns[1].header：「Product Name」重复了",
        "columns[2].source：不认识的数据来源「item.price」：应该以 product / sku / copy / image / shop 开头，或者写成 const:固定值",
    ] {
        assert!(
            found.iter().any(|m| m == expected),
            "缺少：{expected}\n实际：{found:#?}"
        );
    }
}
