//! 裂变参数：模式、强度、锁定项、变化维度。

use serde::{Deserialize, Serialize};

/// 裂变模式，决定规划时变化指令落在哪些维度上。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FissionMode {
    /// 保持风格和配色，换元素、换构图
    #[default]
    Similar,
    /// 保持风格，换主题
    Theme,
    /// 保持内容，换配色
    Palette,
    /// 保持画面，换文字
    Text,
}

/// 变化幅度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    Low,
    #[default]
    Medium,
    High,
}

/// 用户勾选的"必须保持"项，写进提示词作为硬约束。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lock {
    Style,
    Palette,
    Subject,
    Text,
}

/// 一条变化指令改动的维度，由规划模型给出。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariantAxis {
    Element,
    Composition,
    Palette,
    Theme,
    Text,
}

impl FissionMode {
    /// 该模式下允许出现的变化维度。规划结果里出现别的维度时，编排层会要求模型重写。
    pub fn allowed_axes(self) -> &'static [VariantAxis] {
        use VariantAxis::*;
        match self {
            Self::Similar => &[Element, Composition, Palette, Theme],
            Self::Theme => &[Theme, Element],
            Self::Palette => &[Palette],
            Self::Text => &[Text],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_docs() {
        assert_eq!(FissionMode::default(), FissionMode::Similar);
        assert_eq!(Strength::default(), Strength::Medium);
    }

    #[test]
    fn single_axis_modes_stay_on_their_axis() {
        assert_eq!(FissionMode::Palette.allowed_axes(), &[VariantAxis::Palette]);
        assert_eq!(FissionMode::Text.allowed_axes(), &[VariantAxis::Text]);
        assert!(
            !FissionMode::Similar
                .allowed_axes()
                .contains(&VariantAxis::Text)
        );
    }

    #[test]
    fn serde_uses_snake_case() {
        let json = serde_json::to_string(&Lock::Subject).unwrap();
        assert_eq!(json, "\"subject\"");
        let mode: FissionMode = serde_json::from_str("\"palette\"").unwrap();
        assert_eq!(mode, FissionMode::Palette);
    }
}
