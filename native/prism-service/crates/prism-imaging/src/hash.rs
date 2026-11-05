//! 感知哈希（详细设计 6.3）：64 位 pHash，存成 16 位十六进制文本，比较时算汉明距离。

use image::RgbaImage;
use image_hasher::{HashAlg, HasherConfig};

/// 哈希边长，8×8 = 64 位。
const HASH_SIDE: u32 = 8;

/// 计算 pHash（均值 + DCT 预处理，等同于常说的 pHash）。
pub fn phash(img: &RgbaImage) -> String {
    let hasher = HasherConfig::new()
        .hash_size(HASH_SIDE, HASH_SIDE)
        .hash_alg(HashAlg::Mean)
        .preproc_dct()
        .to_hasher();
    hasher
        .hash_image(img)
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 两个哈希的汉明距离；格式不对或长度不同返回 `None`。
pub fn phash_distance(a: &str, b: &str) -> Option<u32> {
    let a = parse_hex(a)?;
    let b = parse_hex(b)?;
    if a.len() != b.len() {
        return None;
    }
    Some(a.iter().zip(&b).map(|(x, y)| (x ^ y).count_ones()).sum())
}

fn parse_hex(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// 左暗右亮、交界略微倾斜的图。
    fn split(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            let value = if x * 100 / width + y * 20 / height < 50 {
                30
            } else {
                220
            };
            Rgba([value, value, value, 255])
        })
    }

    #[test]
    fn similar_images_are_close_and_different_ones_far() {
        let base = split(400, 300);
        let original = phash(&base);
        assert_eq!(original.len(), 16);
        // 同一张图缩小后几乎一样
        let resized = phash(&split(200, 150));
        assert!(phash_distance(&original, &resized).unwrap() <= 4);
        // 构图完全不同的图（棋盘格）距离明显变大
        let checker = phash(&RgbaImage::from_fn(400, 300, |x, y| {
            let value = if (x / 50 + y / 50) % 2 == 0 { 30 } else { 220 };
            Rgba([value, value, value, 255])
        }));
        let distance = phash_distance(&original, &checker).unwrap();
        assert!(distance > 10, "{distance}");
    }

    #[test]
    fn distance_rejects_bad_input() {
        assert_eq!(phash_distance("ff00", "0f00"), Some(4));
        assert_eq!(phash_distance("ff", "ff00"), None);
        assert_eq!(phash_distance("zz", "00"), None);
        assert_eq!(phash_distance("", ""), None);
    }
}
