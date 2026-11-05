//! 主键和时间戳。
//!
//! 主键用 UUID v7：按时间有序，插入 SQLite 时索引不容易碎片化，也可以直接当文件名。
//! 时间统一存 Unix 毫秒。

use std::time::{SystemTime, UNIX_EPOCH};

/// 生成一个新的主键。
pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// 当前时间，Unix 毫秒。
///
/// 系统时钟早于 1970 年这种异常情况下返回 0，不让整个应用因为时钟问题崩溃。
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_time_ordered() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        // v7 的前 48 位是毫秒时间戳，字符串比较即可得出先后
        assert!(a <= b);
        assert_eq!(a.len(), 36);
    }

    #[test]
    fn now_is_after_2026() {
        // 2026-01-01T00:00:00Z
        assert!(now_ms() > 1_767_225_600_000);
    }
}
