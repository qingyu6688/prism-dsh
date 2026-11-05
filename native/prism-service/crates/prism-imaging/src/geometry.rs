//! 尺寸和画布（详细设计 6.4）：等比缩小、铺满裁切、留白适配、扩展画布、裁切。

use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};

use crate::{ImagingError, Result};

/// 铺满裁切时保留哪一部分。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Anchor {
    #[default]
    Center,
    /// 保留上方（人像、上衣常用）
    Top,
    Bottom,
}

fn check_size(width: u32, height: u32) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(ImagingError::InvalidArgument(format!(
            "目标尺寸 {width}×{height} 不对，宽高都要大于 0"
        )));
    }
    Ok(())
}

/// 按比例算出的像素数，至少 1。
fn scaled(value: u32, scale: f64) -> u32 {
    let result = (f64::from(value) * scale).round();
    // 结果不会超过原尺寸乘以缩放比例，图片尺寸本身在 u32 范围内
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let result = result.max(1.0) as u32;
    result
}

fn resize(img: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    if img.dimensions() == (width, height) {
        return img.clone();
    }
    imageops::resize(img, width, height, FilterType::Lanczos3)
}

/// 等比缩小到长边不超过 `max_side`，本来就小的不放大。
pub fn shrink_to_fit(img: &RgbaImage, max_side: u32) -> RgbaImage {
    let (width, height) = img.dimensions();
    let longest = width.max(height);
    if longest <= max_side || max_side == 0 {
        return img.clone();
    }
    let scale = f64::from(max_side) / f64::from(longest);
    resize(img, scaled(width, scale), scaled(height, scale))
}

/// 等比缩放到刚好盖满目标尺寸，再按锚点裁掉多余部分。
pub fn cover(img: &RgbaImage, width: u32, height: u32, anchor: Anchor) -> Result<RgbaImage> {
    check_size(width, height)?;
    let (w, h) = img.dimensions();
    let scale = (f64::from(width) / f64::from(w)).max(f64::from(height) / f64::from(h));
    let resized = resize(
        img,
        scaled(w, scale).max(width),
        scaled(h, scale).max(height),
    );
    let x = (resized.width() - width) / 2;
    let y = match anchor {
        Anchor::Center => (resized.height() - height) / 2,
        Anchor::Top => 0,
        Anchor::Bottom => resized.height() - height,
    };
    Ok(imageops::crop_imm(&resized, x, y, width, height).to_image())
}

/// 等比缩放到刚好放进目标尺寸，空出来的部分填背景色。不会裁掉任何内容。
pub fn contain(
    img: &RgbaImage,
    width: u32,
    height: u32,
    background: Rgba<u8>,
) -> Result<RgbaImage> {
    check_size(width, height)?;
    let (w, h) = img.dimensions();
    let scale = (f64::from(width) / f64::from(w)).min(f64::from(height) / f64::from(h));
    let resized = resize(
        img,
        scaled(w, scale).min(width),
        scaled(h, scale).min(height),
    );
    let mut canvas = RgbaImage::from_pixel(width, height, background);
    let x = i64::from((width - resized.width()) / 2);
    let y = i64::from((height - resized.height()) / 2);
    // 按透明度叠加：半透明的边缘和底色自然融合
    imageops::overlay(&mut canvas, &resized, x, y);
    Ok(canvas)
}

/// 不缩放，四周补背景色，扩展到指定比例（如 1:1、3:4）。
pub fn extend_to_ratio(
    img: &RgbaImage,
    ratio_width: u32,
    ratio_height: u32,
    background: Rgba<u8>,
) -> Result<RgbaImage> {
    check_size(ratio_width, ratio_height)?;
    let (w, h) = img.dimensions();
    // 用整数比较比例，避免浮点误差：w / h < rw / rh 等价于 w × rh < rw × h
    let (target_w, target_h) =
        if u64::from(w) * u64::from(ratio_height) < u64::from(ratio_width) * u64::from(h) {
            (
                div_ceil(
                    u64::from(h) * u64::from(ratio_width),
                    u64::from(ratio_height),
                ),
                u64::from(h),
            )
        } else {
            (
                u64::from(w),
                div_ceil(
                    u64::from(w) * u64::from(ratio_height),
                    u64::from(ratio_width),
                ),
            )
        };
    let (target_w, target_h) = (
        u32::try_from(target_w).map_err(|_| too_big())?,
        u32::try_from(target_h).map_err(|_| too_big())?,
    );
    let mut canvas = RgbaImage::from_pixel(target_w, target_h, background);
    // 原样拷贝像素，不和底色混合
    imageops::replace(
        &mut canvas,
        img,
        i64::from((target_w - w) / 2),
        i64::from((target_h - h) / 2),
    );
    Ok(canvas)
}

fn div_ceil(a: u64, b: u64) -> u64 {
    a.div_ceil(b)
}

fn too_big() -> ImagingError {
    ImagingError::InvalidArgument("扩展后的画布太大".into())
}

/// 按矩形裁切，超出图片的部分截掉；完全在图片外面时报错。
pub fn crop(img: &RgbaImage, x: u32, y: u32, width: u32, height: u32) -> Result<RgbaImage> {
    check_size(width, height)?;
    let (w, h) = img.dimensions();
    if x >= w || y >= h {
        return Err(ImagingError::InvalidArgument(format!(
            "裁切起点 ({x}, {y}) 在图片 {w}×{h} 外面"
        )));
    }
    let width = width.min(w - x);
    let height = height.min(h - y);
    Ok(imageops::crop_imm(img, x, y, width, height).to_image())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba<u8> = Rgba([255, 0, 0, 255]);
    const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);

    fn solid(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, height, RED)
    }

    #[test]
    fn shrinks_but_never_enlarges() {
        assert_eq!(shrink_to_fit(&solid(400, 200), 100).dimensions(), (100, 50));
        assert_eq!(shrink_to_fit(&solid(80, 60), 100).dimensions(), (80, 60));
        assert_eq!(shrink_to_fit(&solid(3000, 1), 100).dimensions(), (100, 1));
    }

    #[test]
    fn cover_fills_the_target() {
        let wide = cover(&solid(400, 200), 100, 100, Anchor::Center).unwrap();
        assert_eq!(wide.dimensions(), (100, 100));
        assert!(wide.pixels().all(|p| *p == RED));
        // 上下各一半颜色，按锚点保留上面
        let mut half = RgbaImage::from_pixel(100, 200, WHITE);
        for y in 0..100 {
            for x in 0..100 {
                half.put_pixel(x, y, RED);
            }
        }
        let top = cover(&half, 100, 100, Anchor::Top).unwrap();
        assert_eq!(*top.get_pixel(50, 50), RED);
        let bottom = cover(&half, 100, 100, Anchor::Bottom).unwrap();
        assert_eq!(*bottom.get_pixel(50, 50), WHITE);
        assert!(cover(&half, 0, 10, Anchor::Center).is_err());
    }

    #[test]
    fn contain_pads_with_background() {
        let padded = contain(&solid(200, 100), 100, 100, WHITE).unwrap();
        assert_eq!(padded.dimensions(), (100, 100));
        assert_eq!(*padded.get_pixel(50, 5), WHITE);
        assert_eq!(*padded.get_pixel(50, 50), RED);
    }

    #[test]
    fn extend_keeps_pixels_and_reaches_the_ratio() {
        let square = extend_to_ratio(&solid(300, 100), 1, 1, WHITE).unwrap();
        assert_eq!(square.dimensions(), (300, 300));
        assert_eq!(*square.get_pixel(150, 150), RED);
        assert_eq!(*square.get_pixel(150, 10), WHITE);
        let portrait = extend_to_ratio(&solid(300, 300), 3, 4, WHITE).unwrap();
        assert_eq!(portrait.dimensions(), (300, 400));
        let same = extend_to_ratio(&solid(300, 400), 3, 4, WHITE).unwrap();
        assert_eq!(same.dimensions(), (300, 400));
    }

    #[test]
    fn crop_is_clipped_to_the_image() {
        assert_eq!(
            crop(&solid(100, 100), 80, 90, 50, 50).unwrap().dimensions(),
            (20, 10)
        );
        assert!(crop(&solid(100, 100), 100, 0, 10, 10).is_err());
    }
}
