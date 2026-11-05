//! 解码、编码和缩略图（详细设计 6.2）。

use std::io::Cursor;

use image::codecs::webp::WebPEncoder;
use image::{ExtendedColorType, ImageFormat as CodecFormat, RgbaImage};

use crate::geometry::shrink_to_fit;
use crate::probe::{ImageFormat, probe};
use crate::{ImagingError, Result};

/// 缩略图的长边。
pub const THUMBNAIL_SIDE: u32 = 256;

/// 解码 PNG / JPEG / WebP 成 RGBA。先按内容识别格式，其他格式不解码。
pub fn decode(bytes: &[u8]) -> Result<RgbaImage> {
    let info = probe(bytes)?;
    let format = match info.format {
        ImageFormat::Png => CodecFormat::Png,
        ImageFormat::Jpeg => CodecFormat::Jpeg,
        ImageFormat::Webp => CodecFormat::WebP,
    };
    image::load_from_memory_with_format(bytes, format)
        .map(|img| img.to_rgba8())
        .map_err(|error| ImagingError::Corrupted(error.to_string()))
}

pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut bytes), CodecFormat::Png)
        .map_err(|error| ImagingError::Encode(error.to_string()))?;
    Ok(bytes)
}

/// 长边 256 的无损 WebP 缩略图，保留透明。
pub fn thumbnail_webp(img: &RgbaImage) -> Result<Vec<u8>> {
    let small = shrink_to_fit(img, THUMBNAIL_SIDE);
    let mut bytes = Vec::new();
    WebPEncoder::new_lossless(&mut bytes)
        .encode(
            small.as_raw(),
            small.width(),
            small.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| ImagingError::Encode(error.to_string()))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn png_round_trips_and_thumbnails_keep_alpha() {
        // 左边一块透明区域，缩小后还是透明
        let img = RgbaImage::from_fn(600, 300, |x, _| {
            if x < 200 {
                Rgba([0, 0, 0, 0])
            } else {
                Rgba([10, 20, 30, 255])
            }
        });
        let png = encode_png(&img).unwrap();
        assert_eq!(decode(&png).unwrap(), img);

        let thumb = thumbnail_webp(&img).unwrap();
        let info = probe(&thumb).unwrap();
        assert_eq!(
            (info.format, info.width, info.height),
            (ImageFormat::Webp, 256, 128)
        );
        let decoded = decode(&thumb).unwrap();
        assert_eq!(decoded.get_pixel(10, 60)[3], 0, "透明保留下来了");
        assert_eq!(decoded.get_pixel(200, 60)[3], 255);
    }

    #[test]
    fn broken_images_are_reported() {
        let png = encode_png(&RgbaImage::from_pixel(50, 50, Rgba([1, 2, 3, 255]))).unwrap();
        // 文件头完整、像素数据被截断
        assert!(matches!(
            decode(&png[..png.len() / 2]),
            Err(ImagingError::Corrupted(_))
        ));
        assert!(matches!(
            decode(b"GIF89a"),
            Err(ImagingError::UnsupportedFormat)
        ));
    }
}
