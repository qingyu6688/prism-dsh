//! 识别图片格式、读宽高。只解析文件头，不解码像素，几十 MB 的图也很快。

use std::io::Cursor;

use image::ImageReader;

use crate::ImagingError;

/// 支持的图片格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

impl ImageFormat {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }

    /// 存文件用的扩展名。
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }
}

/// 图片的基本信息。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
}

/// 按文件内容识别格式并读出宽高。
pub fn probe(bytes: &[u8]) -> Result<ImageInfo, ImagingError> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| ImagingError::Corrupted(error.to_string()))?;
    let format = match reader.format() {
        Some(image::ImageFormat::Png) => ImageFormat::Png,
        Some(image::ImageFormat::Jpeg) => ImageFormat::Jpeg,
        Some(image::ImageFormat::WebP) => ImageFormat::Webp,
        _ => return Err(ImagingError::UnsupportedFormat),
    };
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| ImagingError::Corrupted(error.to_string()))?;
    Ok(ImageInfo {
        format,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    fn encode(width: u32, height: u32, format: image::ImageFormat) -> Vec<u8> {
        let buffer: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(width, height);
        let mut bytes = Vec::new();
        buffer
            .write_to(&mut Cursor::new(&mut bytes), format)
            .unwrap();
        bytes
    }

    #[test]
    fn reads_format_and_size_from_content() {
        let png = probe(&encode(30, 20, image::ImageFormat::Png)).unwrap();
        assert_eq!(
            png,
            ImageInfo {
                format: ImageFormat::Png,
                width: 30,
                height: 20
            }
        );
        let jpeg = probe(&encode(8, 9, image::ImageFormat::Jpeg)).unwrap();
        assert_eq!(
            (jpeg.format, jpeg.width, jpeg.height),
            (ImageFormat::Jpeg, 8, 9)
        );
        assert_eq!(jpeg.format.mime(), "image/jpeg");
        assert_eq!(jpeg.format.extension(), "jpg");
    }

    #[test]
    fn rejects_other_content() {
        assert!(matches!(
            probe(b"GIF89a not really"),
            Err(ImagingError::UnsupportedFormat)
        ));
        assert!(matches!(
            probe(b"hello"),
            Err(ImagingError::UnsupportedFormat)
        ));
    }

    #[test]
    fn truncated_file_is_corrupted() {
        let png = encode(30, 20, image::ImageFormat::Png);
        assert!(matches!(probe(&png[..12]), Err(ImagingError::Corrupted(_))));
    }
}
