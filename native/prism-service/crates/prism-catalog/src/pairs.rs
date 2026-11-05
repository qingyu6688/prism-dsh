//! 有序键值对和 JSON 对象互转。
//!
//! 商品参数、SKU 规格在库里存成 JSON 对象（`{"颜色":"红","尺码":"M"}`），但用户在意顺序：
//! 颜色要在尺码前面。serde_json 默认把对象读进按键名排序的 map，顺序就丢了，
//! 所以这里自己按出现顺序写、按出现顺序读，不用额外开 `preserve_order` 特性。

use std::fmt;

use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserializer, Serialize, Serializer};

/// 按给定顺序写成 JSON 对象。
pub(crate) fn to_json(pairs: &[(String, String)]) -> String {
    struct Ordered<'a>(&'a [(String, String)]);

    impl Serialize for Ordered<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut map = serializer.serialize_map(Some(self.0.len()))?;
            for (key, value) in self.0 {
                map.serialize_entry(key, value)?;
            }
            map.end()
        }
    }

    // 键和值都是字符串，序列化不会失败；万一失败宁可存空对象，也不让保存整个商品失败
    serde_json::to_string(&Ordered(pairs)).unwrap_or_else(|_| String::from("{}"))
}

/// 按出现顺序读 JSON 对象。值不是字符串的（手工改过数据库）转成 JSON 文本。
pub(crate) fn from_json(text: &str) -> Result<Vec<(String, String)>, serde_json::Error> {
    struct PairsVisitor;

    impl<'de> Visitor<'de> for PairsVisitor {
        type Value = Vec<(String, String)>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("JSON 对象")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut pairs = Vec::new();
            while let Some((key, value)) = map.next_entry::<String, serde_json::Value>()? {
                let value = match value {
                    serde_json::Value::String(text) => text,
                    other => other.to_string(),
                };
                pairs.push((key, value));
            }
            Ok(pairs)
        }
    }

    let mut deserializer = serde_json::Deserializer::from_str(text);
    let pairs = (&mut deserializer).deserialize_map(PairsVisitor)?;
    deserializer.end()?;
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn keeps_the_order_users_entered() {
        let spec = pairs(&[("颜色", "红"), ("尺码", "M"), ("款式", "圆领")]);
        let json = to_json(&spec);
        assert_eq!(json, r#"{"颜色":"红","尺码":"M","款式":"圆领"}"#);
        assert_eq!(from_json(&json).unwrap(), spec);
    }

    #[test]
    fn non_string_values_become_text() {
        let read = from_json(r#"{"重量":1.5,"防水":true,"b":"x"}"#).unwrap();
        assert_eq!(
            read,
            pairs(&[("重量", "1.5"), ("防水", "true"), ("b", "x")])
        );
    }

    #[test]
    fn rejects_non_objects() {
        assert!(from_json("[1,2]").is_err());
        assert!(from_json(r#"{"a":"b"} trailing"#).is_err());
        assert_eq!(to_json(&[]), "{}");
    }
}
