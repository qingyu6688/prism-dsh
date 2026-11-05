//! SVG 排版模板（详细设计 6.6）：替换变量、自动排版带 `data-max-width` 的文字、用 resvg 渲染。

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use image::{Rgba, RgbaImage};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{self, fontdb, roxmltree};

use crate::text_fit::{FitSpec, FittedText, MEASURE_SIZE, fit_text};
use crate::{ImagingError, Result};

/// 渲染画布的上限，防止模板写错尺寸把内存吃光。
const MAX_PIXELS: u64 = 8192 * 8192;
const DEFAULT_MAX_LINES: usize = 1;
const DEFAULT_LINE_HEIGHT: f32 = 1.25;
const DEFAULT_FONT_SIZE: f32 = 16.0;
const SHRINK_STEP: f32 = 2.0;
/// 排版后实测校正最多几轮。
const FIT_ROUNDS: usize = 3;
/// 实测宽度允许超出多少像素（抗锯齿的半个像素等）。
const FIT_TOLERANCE: f32 = 1.0;
const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

/// 渲染器。加载字体比较慢，应用里建一个反复用。
pub struct SvgRenderer {
    fontdb: Arc<fontdb::Database>,
}

impl SvgRenderer {
    /// 加载系统字体和额外目录（如数据目录的 `fonts/`）里的字体。目录不存在时跳过。
    pub fn new(extra_font_dirs: &[PathBuf]) -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        for dir in extra_font_dirs {
            db.load_fonts_dir(dir);
        }
        Self {
            fontdb: Arc::new(db),
        }
    }

    /// 加载到的字体数量。为 0 时文字渲染不出来（比如精简的 Linux 环境）。
    pub fn font_count(&self) -> usize {
        self.fontdb.len()
    }

    fn options(&self) -> usvg::Options<'static> {
        usvg::Options {
            fontdb: Arc::clone(&self.fontdb),
            ..usvg::Options::default()
        }
    }

    /// 替换变量并排版，返回最终交给 resvg 的 SVG 文本。模板调试、预览用。
    pub fn fill(&self, template: &str, vars: &BTreeMap<String, String>) -> Result<String> {
        self.layout(&substitute(template, vars))
    }

    /// 替换变量、排版、渲染成 RGBA 图片。
    pub fn render(&self, template: &str, vars: &BTreeMap<String, String>) -> Result<RgbaImage> {
        let laid_out = self.fill(template, vars)?;
        let tree = usvg::Tree::from_str(&laid_out, &self.options())
            .map_err(|error| ImagingError::Template(error.to_string()))?;
        let size = tree.size().to_int_size();
        if u64::from(size.width()) * u64::from(size.height()) > MAX_PIXELS {
            return Err(ImagingError::Template(format!(
                "画布 {}×{} 太大",
                size.width(),
                size.height()
            )));
        }
        let mut pixmap = Pixmap::new(size.width(), size.height())
            .ok_or_else(|| ImagingError::Template("画布尺寸为 0".into()))?;
        resvg::render(&tree, Transform::default(), &mut pixmap.as_mut());
        // tiny-skia 用预乘透明度，转回普通 RGBA
        let mut image = RgbaImage::new(size.width(), size.height());
        for (target, source) in image.pixels_mut().zip(pixmap.pixels()) {
            let color = source.demultiply();
            *target = Rgba([color.red(), color.green(), color.blue(), color.alpha()]);
        }
        Ok(image)
    }

    /// 给带 `data-max-width` 的 `<text>` 排版，换成多行 `<tspan>`。
    fn layout(&self, svg: &str) -> Result<String> {
        let options = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..roxmltree::ParsingOptions::default()
        };
        let document = roxmltree::Document::parse_with_options(svg, options)
            .map_err(|error| ImagingError::Template(error.to_string()))?;
        let mut replacements = Vec::new();
        for node in document.descendants() {
            if node.is_element()
                && node.tag_name().name() == "text"
                && node.attribute("data-max-width").is_some()
            {
                replacements.push((node.range(), self.fit_element(node)?));
            }
        }
        // 从后往前替换，前面的位置不受影响
        let mut output = svg.to_owned();
        for (range, replacement) in replacements.into_iter().rev() {
            output.replace_range(range, &replacement);
        }
        Ok(output)
    }

    fn fit_element(&self, node: roxmltree::Node<'_, '_>) -> Result<String> {
        let number = |name: &str| -> Result<Option<f32>> {
            node.attribute(name)
                .map(|value| {
                    value
                        .trim()
                        .trim_end_matches("px")
                        .split([' ', ','])
                        .next()
                        .unwrap_or_default()
                        .parse::<f32>()
                        .map_err(|_| ImagingError::Template(format!("{name}=\"{value}\" 不是数字")))
                })
                .transpose()
        };
        let max_width = number("data-max-width")?.unwrap_or_default();
        if max_width <= 0.0 {
            return Err(ImagingError::Template("data-max-width 要大于 0".into()));
        }
        let font_size = number("font-size")?.unwrap_or(DEFAULT_FONT_SIZE);
        let max_lines = number("data-max-lines")?.map_or(DEFAULT_MAX_LINES, |n| {
            // 行数是小的正整数
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let lines = n.max(1.0) as usize;
            lines
        });
        let spec = FitSpec {
            max_width,
            max_lines,
            font_size,
            min_font_size: number("data-min-font-size")?.unwrap_or(font_size / 2.0),
            step: SHRINK_STEP,
        };
        let line_height = number("data-line-height")?.unwrap_or(DEFAULT_LINE_HEIGHT);
        let x = number("x")?.unwrap_or(0.0);
        let y = number("y")?.unwrap_or(0.0);

        let text: String = node
            .descendants()
            .filter(|n| n.is_text())
            .filter_map(|n| n.text())
            .collect();
        let font = FontAttributes {
            family: node.attribute("font-family").unwrap_or("sans-serif"),
            weight: node.attribute("font-weight").unwrap_or("normal"),
            style: node.attribute("font-style").unwrap_or("normal"),
        };
        let fitted = self.fit_checked(&text, spec, &font);

        let mut element = String::from("<text");
        for attribute in node.attributes() {
            let name = attribute.name();
            if name.starts_with("data-") || name == "font-size" {
                continue;
            }
            let prefix = if attribute.namespace() == Some(XML_NAMESPACE) {
                "xml:"
            } else {
                ""
            };
            // 写入 String 不会失败
            let _ = write!(element, " {prefix}{name}=\"{}\"", escape(attribute.value()));
        }
        let _ = write!(element, " font-size=\"{}\">", fitted.font_size);
        for (index, line) in fitted.lines.iter().enumerate() {
            if index == 0 {
                let _ = write!(element, "<tspan x=\"{x}\" y=\"{y}\">");
            } else {
                let _ = write!(
                    element,
                    "<tspan x=\"{x}\" dy=\"{}\">",
                    fitted.font_size * line_height
                );
            }
            element.push_str(&escape(line));
            element.push_str("</tspan>");
        }
        element.push_str("</text>");
        Ok(element)
    }

    /// 排版后逐行实际渲染量一次。逐字测量和整行渲染会有出入（比如字体回退时，中文后面的英文字母
    /// 也用了中文字体），超出时按超出的比例收窄可用宽度重排，最多 3 轮。
    fn fit_checked(&self, text: &str, spec: FitSpec, font: &FontAttributes<'_>) -> FittedText {
        let mut budget = spec;
        let mut measure = |piece: &str| self.measure(piece, font);
        let mut fitted = fit_text(text, &budget, &mut measure);
        for _ in 1..FIT_ROUNDS {
            let widest = fitted
                .lines
                .iter()
                .map(|line| self.ink_width(line, font) * fitted.font_size / MEASURE_SIZE)
                .fold(0.0_f32, f32::max);
            if widest <= spec.max_width + FIT_TOLERANCE {
                break;
            }
            budget.max_width = budget.max_width * spec.max_width / widest;
            fitted = fit_text(text, &budget, &mut measure);
        }
        fitted
    }

    /// 100 号字下一段文字的前进宽度。用"H + 文字 + H"减去"HH"的宽度，空格这种没有笔画的也能量出来。
    fn measure(&self, piece: &str, font: &FontAttributes<'_>) -> f32 {
        let piece = format!("H{piece}H");
        (self.ink_width(&piece, font) - self.ink_width("HH", font)).max(0.0)
    }

    /// 100 号字下一段文字实际画出来的宽度（笔画的外接矩形）。
    fn ink_width(&self, content: &str, font: &FontAttributes<'_>) -> f32 {
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10000\" height=\"400\">\
             <text x=\"10\" y=\"200\" font-family=\"{}\" font-size=\"{MEASURE_SIZE}\" \
             font-weight=\"{}\" font-style=\"{}\" xml:space=\"preserve\">{}</text></svg>",
            escape(font.family),
            escape(font.weight),
            escape(font.style),
            escape(content)
        );
        usvg::Tree::from_str(&svg, &self.options())
            .map(|tree| tree.root().abs_bounding_box().width())
            .unwrap_or(0.0)
    }
}

struct FontAttributes<'a> {
    family: &'a str,
    weight: &'a str,
    style: &'a str,
}

/// 把 `{{名称}}` 换成变量值（XML 转义）；没给值的换成空，没配对的 `{{` 原样保留。
fn substitute(template: &str, vars: &BTreeMap<String, String>) -> String {
    let mut output = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                if let Some(value) = vars.get(after[..end].trim()) {
                    output.push_str(&escape(value));
                }
                rest = &after[end + 2..];
            }
            None => {
                output.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    output.push_str(rest);
    output
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(list: &[(&str, &str)]) -> BTreeMap<String, String> {
        list.iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn variables_are_escaped() {
        let filled = substitute(
            "<text>{{ title }}|{{missing}}|{{price}</text>",
            &vars(&[("title", "A&B <新品>")]),
        );
        assert_eq!(filled, "<text>A&amp;B &lt;新品&gt;||{{price}</text>");
    }

    #[test]
    fn renders_shapes_without_fonts() {
        // 不依赖字体的部分：尺寸、颜色、透明
        let renderer = SvgRenderer {
            fontdb: Arc::new(fontdb::Database::new()),
        };
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">
            <rect x="0" y="0" width="20" height="20" fill="#ff0000"/></svg>"##;
        let image = renderer.render(svg, &BTreeMap::new()).unwrap();
        assert_eq!(image.dimensions(), (40, 20));
        assert_eq!(*image.get_pixel(5, 5), Rgba([255, 0, 0, 255]));
        assert_eq!(image.get_pixel(30, 5)[3], 0);
        assert!(matches!(
            renderer.render("<svg", &BTreeMap::new()),
            Err(ImagingError::Template(_))
        ));
    }

    #[test]
    fn layout_rewrites_slots_into_tspans() {
        let renderer = SvgRenderer {
            fontdb: Arc::new(fontdb::Database::new()),
        };
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><g><text x="10" y="40" font-size="20" fill="red" xml:space="preserve" data-max-width="80" data-max-lines="2">短</text></g></svg>"#;
        let laid_out = renderer.layout(svg).unwrap();
        // 没有字体时宽度量出来是 0，一行就够；属性保留，data-* 去掉
        assert!(laid_out.contains(r#"<text x="10" y="40" fill="red" xml:space="preserve" font-size="20"><tspan x="10" y="40">短</tspan></text>"#), "{laid_out}");
        assert!(!laid_out.contains("data-max-width"));
        assert!(matches!(
            renderer.layout(r#"<svg xmlns="http://www.w3.org/2000/svg"><text data-max-width="宽">x</text></svg>"#),
            Err(ImagingError::Template(_))
        ));
    }
}
