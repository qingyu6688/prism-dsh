//! 本地图片处理（详细设计第 6 章）：尺寸和画布、合成和白底检查、去纯色背景、缩略图、
//! 感知哈希、SVG 排版模板渲染。
//!
//! 不联网，只用纯 Rust 实现，避免给贡献者增加 C 库依赖。图片统一用 RGBA8（`image::RgbaImage`）。

mod chroma;
mod compose;
mod encode;
mod geometry;
mod hash;
mod probe;
mod svg;
mod text_fit;

pub use chroma::{ChromaKey, border_color, remove_background};
pub use compose::{
    Bounds, bounds_on_white, has_white_border, overlay, place_on_background, product_ratio,
    subject_bounds,
};
pub use encode::{THUMBNAIL_SIDE, decode, encode_png, thumbnail_webp};
pub use geometry::{Anchor, contain, cover, crop, extend_to_ratio, shrink_to_fit};
pub use hash::{phash, phash_distance};
pub use image::{Rgba, RgbaImage};
pub use probe::{ImageFormat, ImageInfo, probe};
pub use svg::SvgRenderer;
pub use text_fit::{FitSpec, FittedText, fit_text};

/// 图片处理的错误。
#[derive(Debug, thiserror::Error)]
pub enum ImagingError {
    /// 不是支持的图片格式（按内容判断，不看扩展名）
    #[error("不支持的图片格式，只支持 PNG、JPEG、WebP")]
    UnsupportedFormat,

    /// 格式认得出来，但内容读不出来，多半是文件损坏或不完整
    #[error("图片文件已损坏或不完整：{0}")]
    Corrupted(String),

    /// 参数不对，比如目标尺寸是 0、裁切区域在图片外面
    #[error("{0}")]
    InvalidArgument(String),

    /// 编码输出图片失败
    #[error("生成图片失败：{0}")]
    Encode(String),

    /// SVG 模板有问题
    #[error("排版模板有问题：{0}")]
    Template(String),
}

/// 图片处理的返回类型。
pub type Result<T> = std::result::Result<T, ImagingError>;
