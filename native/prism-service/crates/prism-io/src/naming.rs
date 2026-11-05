//! 导出文件命名：命名模板替换变量，去掉 Windows 不允许的字符。

use std::collections::BTreeMap;

/// 文件名（不含扩展名）最长多少个字符，避免超过 Windows 路径长度限制。
const NAME_MAX_CHARS: usize = 120;
const FALLBACK_NAME: &str = "未命名";
/// Windows 保留的设备名，不能当文件名。
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 按命名模板生成文件名，如 `{product}_{index}`。不认识的变量替换成空；结果去掉非法字符。
pub fn render_name(pattern: &str, vars: &BTreeMap<&str, String>) -> String {
    let mut output = String::new();
    let mut rest = pattern;
    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                let name = after[..end].trim();
                if let Some(value) = vars.get(name) {
                    output.push_str(value);
                }
                rest = &after[end + 1..];
            }
            None => {
                // 没有配对的右括号，按原样保留
                output.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    output.push_str(rest);
    sanitize_file_name(&output)
}

/// 去掉 `<>:"/\|?*` 和控制字符，去掉结尾的点和空格，避开保留名，限制长度；空的用"未命名"。
pub fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed: String = cleaned
        .trim()
        .trim_end_matches(['.', ' '])
        .chars()
        .take(NAME_MAX_CHARS)
        .collect();
    let trimmed = trimmed.trim_end_matches(['.', ' ']).to_owned();
    if trimmed.is_empty() || trimmed.chars().all(|c| c == '_') {
        return FALLBACK_NAME.into();
    }
    let stem = trimmed.split('.').next().unwrap_or_default();
    if RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
    {
        return format!("_{trimmed}");
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_patterns() {
        let mut vars = BTreeMap::new();
        vars.insert("product", "T 恤/白色".to_owned());
        vars.insert("index", "3".to_owned());
        assert_eq!(render_name("{product}_{index}", &vars), "T 恤_白色_3");
        assert_eq!(render_name("{ product }-{missing}", &vars), "T 恤_白色-");
        assert_eq!(render_name("img_{index", &vars), "img_{index");
        assert_eq!(render_name("{missing}", &vars), "未命名");
    }

    #[test]
    fn sanitizes_windows_names() {
        assert_eq!(sanitize_file_name(" a:b?.png. "), "a_b_.png");
        assert_eq!(sanitize_file_name("CON"), "_CON");
        assert_eq!(sanitize_file_name("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_file_name("..."), "未命名");
        assert_eq!(sanitize_file_name(&"长".repeat(200)).chars().count(), 120);
    }
}
