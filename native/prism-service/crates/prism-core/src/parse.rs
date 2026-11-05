//! 数据库里以字符串保存的枚举，统一从字符串解析回来。

/// 数据库里出现了代码不认识的枚举值，通常是数据被手动改过或者版本不匹配。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("无法识别的{kind}：{value}")]
pub struct ParseEnumError {
    pub kind: &'static str,
    pub value: String,
}

/// 给有 `as_str()` 的枚举生成 `ALL` 常量和 `FromStr` 实现，保证读写用的是同一套字符串。
macro_rules! impl_db_enum {
    ($ty:ident, $kind:literal, [$($variant:ident),+ $(,)?]) => {
        impl $ty {
            /// 全部取值
            pub const ALL: &'static [$ty] = &[$($ty::$variant),+];
        }

        impl std::str::FromStr for $ty {
            type Err = $crate::parse::ParseEnumError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::ALL
                    .iter()
                    .copied()
                    .find(|value| value.as_str() == s)
                    .ok_or_else(|| $crate::parse::ParseEnumError {
                        kind: $kind,
                        value: s.to_owned(),
                    })
            }
        }

        // 给数据库层用：读出来的是 String，直接转成枚举
        impl TryFrom<String> for $ty {
            type Error = $crate::parse::ParseEnumError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                value.parse()
            }
        }
    };
}
