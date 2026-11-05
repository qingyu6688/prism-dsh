//! 合成和白底检查（详细设计 6.5）。

use image::imageops;
use image::{Rgba, RgbaImage};

use crate::geometry::crop;
use crate::{ImagingError, Result};

/// 透明度低于这个值的像素当作看不见，算主体范围时忽略（去背景后常有零星的残留）。
const VISIBLE_ALPHA: u8 = 8;

/// 主体的外接矩形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// 把 `top` 按透明度叠到 `base` 的 (x, y) 位置，超出的部分截掉。
pub fn overlay(base: &mut RgbaImage, top: &RgbaImage, x: i64, y: i64) {
    imageops::overlay(base, top, x, y);
}

/// 满足条件的像素的外接矩形。
fn bounds_where(img: &RgbaImage, keep: impl Fn(&Rgba<u8>) -> bool) -> Option<Bounds> {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, pixel) in img.enumerate_pixels() {
        if keep(pixel) {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    (min_x != u32::MAX).then(|| Bounds {
        x: min_x,
        y: min_y,
        width: max_x - min_x + 1,
        height: max_y - min_y + 1,
    })
}

/// 透明背景图的主体范围：看得见的像素。全透明时返回 `None`。
pub fn subject_bounds(img: &RgbaImage) -> Option<Bounds> {
    bounds_where(img, |p| p[3] >= VISIBLE_ALPHA)
}

fn is_white(pixel: &Rgba<u8>, tolerance: u8) -> bool {
    let floor = 255 - tolerance;
    pixel[3] == 255 && pixel[0] >= floor && pixel[1] >= floor && pixel[2] >= floor
}

/// 白底图的主体范围：不是白色的像素。整张都是白色时返回 `None`。
pub fn bounds_on_white(img: &RgbaImage, tolerance: u8) -> Option<Bounds> {
    bounds_where(img, |p| p[3] >= VISIBLE_ALPHA && !is_white(p, tolerance))
}

/// 商品占比：主体宽、高分别除以画面宽、高，取较大的。
pub fn product_ratio(bounds: Bounds, width: u32, height: u32) -> f64 {
    if width == 0 || height == 0 {
        return 0.0;
    }
    (f64::from(bounds.width) / f64::from(width)).max(f64::from(bounds.height) / f64::from(height))
}

/// 四条边最外圈的像素是不是都是白色（透明不算白）。
pub fn has_white_border(img: &RgbaImage, tolerance: u8) -> bool {
    let (width, height) = img.dimensions();
    if width == 0 || height == 0 {
        return false;
    }
    let top_bottom = (0..width).all(|x| {
        is_white(img.get_pixel(x, 0), tolerance)
            && is_white(img.get_pixel(x, height - 1), tolerance)
    });
    let left_right = (0..height).all(|y| {
        is_white(img.get_pixel(0, y), tolerance) && is_white(img.get_pixel(width - 1, y), tolerance)
    });
    top_bottom && left_right
}

/// 把透明背景的商品图放到纯色底上：先裁到主体范围，按商品占比缩放后居中。
pub fn place_on_background(
    subject: &RgbaImage,
    width: u32,
    height: u32,
    background: Rgba<u8>,
    fill_ratio: f64,
) -> Result<RgbaImage> {
    if !(fill_ratio > 0.0 && fill_ratio <= 1.0) {
        return Err(ImagingError::InvalidArgument(format!(
            "商品占比 {fill_ratio} 不对，应该在 0～1 之间"
        )));
    }
    let bounds = subject_bounds(subject)
        .ok_or_else(|| ImagingError::InvalidArgument("图片是全透明的，找不到商品".into()))?;
    let trimmed = crop(subject, bounds.x, bounds.y, bounds.width, bounds.height)?;
    let box_width = (f64::from(width) * fill_ratio).round();
    let box_height = (f64::from(height) * fill_ratio).round();
    // 两个值都在 1～目标尺寸之间
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (box_width, box_height) = (box_width.max(1.0) as u32, box_height.max(1.0) as u32);
    crate::geometry::contain(&trimmed, box_width, box_height, Rgba([0, 0, 0, 0])).map(|fitted| {
        let mut canvas = RgbaImage::from_pixel(width, height, background);
        overlay(
            &mut canvas,
            &fitted,
            i64::from((width - box_width) / 2),
            i64::from((height - box_height) / 2),
        );
        canvas
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
    const BLUE: Rgba<u8> = Rgba([0, 0, 255, 255]);
    const CLEAR: Rgba<u8> = Rgba([0, 0, 0, 0]);

    /// 透明底上一个蓝色方块。
    fn subject() -> RgbaImage {
        let mut img = RgbaImage::from_pixel(100, 100, CLEAR);
        for y in 30..50 {
            for x in 20..60 {
                img.put_pixel(x, y, BLUE);
            }
        }
        img
    }

    #[test]
    fn finds_the_subject() {
        assert_eq!(
            subject_bounds(&subject()),
            Some(Bounds {
                x: 20,
                y: 30,
                width: 40,
                height: 20
            })
        );
        assert_eq!(subject_bounds(&RgbaImage::from_pixel(5, 5, CLEAR)), None);
    }

    #[test]
    fn white_background_product_is_centered_at_the_ratio() {
        let main = place_on_background(&subject(), 1000, 1000, WHITE, 0.85).unwrap();
        assert!(has_white_border(&main, 0));
        let bounds = bounds_on_white(&main, 0).unwrap();
        let ratio = product_ratio(bounds, 1000, 1000);
        assert!((ratio - 0.85).abs() < 0.01, "{ratio}");
        // 宽 40 高 20 的商品，按宽撑满 850，居中
        assert!((bounds.width as i64 - 850).abs() <= 1);
        assert!((bounds.x as i64 - 75).abs() <= 1);
        assert!((bounds.y as i64 - 287).abs() <= 2);

        assert!(place_on_background(&subject(), 10, 10, WHITE, 1.5).is_err());
        assert!(
            place_on_background(&RgbaImage::from_pixel(5, 5, CLEAR), 10, 10, WHITE, 0.8).is_err()
        );
    }

    #[test]
    fn white_border_check_is_strict_by_default() {
        let mut img = RgbaImage::from_pixel(10, 10, WHITE);
        assert!(has_white_border(&img, 0));
        img.put_pixel(9, 4, Rgba([254, 255, 255, 255]));
        assert!(!has_white_border(&img, 0));
        assert!(has_white_border(&img, 2));
        img.put_pixel(9, 4, Rgba([255, 255, 255, 0]));
        assert!(!has_white_border(&img, 10), "透明不算白");
    }

    #[test]
    fn overlay_blends_by_alpha() {
        let mut base = RgbaImage::from_pixel(4, 4, WHITE);
        let top = RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 128]));
        overlay(&mut base, &top, 3, 3);
        let mixed = base.get_pixel(3, 3);
        assert!(mixed[0] > 120 && mixed[0] < 135, "{mixed:?}");
        assert_eq!(*base.get_pixel(0, 0), WHITE);
    }
}
