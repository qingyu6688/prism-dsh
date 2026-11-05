//! 去纯色背景（详细设计 6.1）：按色距给透明度，边缘反算主体颜色去溢色。

use image::{Rgb, Rgba, RgbaImage};

/// 背景色和两个距离阈值。距离是 RGB 空间里的欧氏距离（0～441）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChromaKey {
    pub color: Rgb<u8>,
    /// 距离不超过它的完全透明
    pub inner: f32,
    /// 距离不小于它的完全保留，中间线性过渡
    pub outer: f32,
}

impl ChromaKey {
    /// 生图时让模型画的纯绿背景。
    pub const GREEN: Self = Self::new(Rgb([0, 255, 0]));
    /// 主体本身有绿色时改用洋红。
    pub const MAGENTA: Self = Self::new(Rgb([255, 0, 255]));

    pub const fn new(color: Rgb<u8>) -> Self {
        // 一半主体一半背景的边缘像素，离背景色大约是"主体到背景色距离"的一半（红色约 150），
        // 外阈值要比这个大，边缘才会变成半透明；常见商品颜色（白、黑、红、灰、肤色）离纯绿都在 220 以上
        Self {
            color,
            inner: 90.0,
            outer: 200.0,
        }
    }
}

fn distance(pixel: &Rgba<u8>, key: Rgb<u8>) -> f32 {
    let channel = |i: usize| f32::from(pixel[i]) - f32::from(key[i]);
    (channel(0).powi(2) + channel(1).powi(2) + channel(2).powi(2)).sqrt()
}

/// 去掉纯色背景，返回带透明通道的新图。
pub fn remove_background(img: &RgbaImage, key: &ChromaKey) -> RgbaImage {
    let span = (key.outer - key.inner).max(f32::EPSILON);
    let mut output = img.clone();
    for pixel in output.pixels_mut() {
        let d = distance(pixel, key.color);
        let alpha = ((d - key.inner) / span).clamp(0.0, 1.0);
        if alpha <= 0.0 {
            *pixel = Rgba([0, 0, 0, 0]);
            continue;
        }
        if alpha < 1.0 {
            // 边缘像素 = α × 主体 + (1 − α) × 背景色，反算主体颜色，去掉绿边
            for i in 0..3 {
                let observed = f32::from(pixel[i]);
                let background = f32::from(key.color[i]);
                let original = ((observed - (1.0 - alpha) * background) / alpha).clamp(0.0, 255.0);
                pixel[i] = to_u8(original);
            }
        }
        pixel[3] = to_u8(f32::from(pixel[3]) * alpha);
    }
    output
}

/// 已经限制在 0～255 的值转成 u8。
fn to_u8(value: f32) -> u8 {
    // 调用前都 clamp 到 0～255 了，不会截断
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let result = value.round().clamp(0.0, 255.0) as u8;
    result
}

/// 四条边最外圈像素每个通道的中位数，背景色不确定时当作背景色。
pub fn border_color(img: &RgbaImage) -> Rgb<u8> {
    let (width, height) = img.dimensions();
    let mut channels: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut push = |x: u32, y: u32| {
        let pixel = img.get_pixel(x, y);
        for (i, values) in channels.iter_mut().enumerate() {
            values.push(pixel[i]);
        }
    };
    for x in 0..width {
        push(x, 0);
        push(x, height.saturating_sub(1));
    }
    for y in 0..height {
        push(0, y);
        push(width.saturating_sub(1), y);
    }
    let median = |values: &mut Vec<u8>| {
        values.sort_unstable();
        values.get(values.len() / 2).copied().unwrap_or(0)
    };
    let [mut r, mut g, mut b] = channels;
    Rgb([median(&mut r), median(&mut g), median(&mut b)])
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREEN: Rgba<u8> = Rgba([0, 255, 0, 255]);
    const RED: Rgba<u8> = Rgba([200, 30, 30, 255]);

    #[test]
    fn background_becomes_transparent_and_subject_stays() {
        let mut img = RgbaImage::from_pixel(10, 10, GREEN);
        img.put_pixel(5, 5, RED);
        // 接近背景的噪点也去掉
        img.put_pixel(1, 1, Rgba([20, 240, 10, 255]));
        let out = remove_background(&img, &ChromaKey::GREEN);
        assert_eq!(out.get_pixel(0, 0)[3], 0);
        assert_eq!(out.get_pixel(1, 1)[3], 0);
        assert_eq!(*out.get_pixel(5, 5), RED);
    }

    #[test]
    fn edges_are_unmixed_from_the_key_color() {
        // 一半红一半绿混出来的边缘像素
        let edge = Rgba([100, 142, 15, 255]);
        let img = RgbaImage::from_pixel(1, 1, edge);
        let out = remove_background(&img, &ChromaKey::GREEN);
        let pixel = out.get_pixel(0, 0);
        assert!(pixel[3] > 0 && pixel[3] < 255, "{pixel:?}");
        // 反算后绿色明显减少，不再发绿
        assert!(pixel[1] < 100, "{pixel:?}");
    }

    #[test]
    fn border_median_finds_the_background() {
        let mut img = RgbaImage::from_pixel(20, 20, Rgba([255, 0, 255, 255]));
        img.put_pixel(0, 0, RED);
        img.put_pixel(10, 10, RED);
        assert_eq!(border_color(&img), Rgb([255, 0, 255]));
    }
}
