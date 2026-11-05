//! 按规则包的长度单位计长度。生成文案和检查字数都用这里，保证计法一致。

use crate::model::LengthUnit;

/// 按单位计算文本长度。
pub fn measure(text: &str, unit: LengthUnit) -> usize {
    match unit {
        LengthUnit::Char => text.chars().count(),
        LengthUnit::Byte => text.len(),
        LengthUnit::CjkDouble => text.chars().map(|c| if is_wide(c) { 2 } else { 1 }).sum(),
    }
}

/// 淘宝、拼多多按"汉字算 2 个字符"计数的字符：中日韩文字、假名、谚文和全角符号。
fn is_wide(c: char) -> bool {
    matches!(
        u32::from(c),
        0x1100..=0x115F       // 谚文字母
        | 0x2E80..=0x303F     // 中日韩部首、标点
        | 0x3040..=0x30FF     // 平假名、片假名
        | 0x3100..=0x31FF     // 注音、片假名扩展
        | 0x3400..=0x4DBF     // 中日韩统一表意文字扩展 A
        | 0x4E00..=0x9FFF     // 中日韩统一表意文字
        | 0xAC00..=0xD7AF     // 谚文音节
        | 0xF900..=0xFAFF     // 兼容表意文字
        | 0xFE30..=0xFE4F     // 竖排标点
        | 0xFF00..=0xFF60     // 全角 ASCII、全角标点
        | 0xFFE0..=0xFFE6     // 全角符号
        | 0x20000..=0x3FFFD   // 扩展 B 以后
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_by_unit() {
        let text = "纯棉T恤 2件";
        assert_eq!(measure(text, LengthUnit::Char), 7);
        assert_eq!(measure(text, LengthUnit::Byte), 15);
        // 纯 棉 恤 件 各 2，T 空格 2 各 1
        assert_eq!(measure(text, LengthUnit::CjkDouble), 11);
        assert_eq!(measure("，！", LengthUnit::CjkDouble), 4);
        assert_eq!(measure("", LengthUnit::CjkDouble), 0);
        // 30 个汉字正好是淘宝标题的 60 字符
        assert_eq!(measure(&"字".repeat(30), LengthUnit::CjkDouble), 60);
    }
}
