//! SVG 排版模板用真实系统字体渲染（需求 FR-105，P2 验收"中英文渲染正确，长文字换行或缩小"）。

// 测试里的辅助函数出错就该直接失败，用 unwrap 最直接
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use prism_imaging::{SvgRenderer, bounds_on_white};

const TEMPLATE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="800" height="400">
  <rect width="800" height="400" fill="#ffffff"/>
  <text x="40" y="100" font-family="{{font}}" font-size="60" fill="#111111"
        data-max-width="720" data-max-lines="2" data-min-font-size="30">{{title}}</text>
</svg>"##;

fn vars(font: &str, title: &str) -> BTreeMap<String, String> {
    [("font", font), ("title", title)]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn tspans(svg: &str) -> usize {
    svg.matches("<tspan").count()
}

fn font_size(svg: &str) -> f32 {
    let start = svg.find("font-size=\"").unwrap() + "font-size=\"".len();
    let end = svg[start..].find('"').unwrap() + start;
    svg[start..end].parse().unwrap()
}

#[test]
fn english_text_wraps_and_shrinks() {
    let renderer = SvgRenderer::new(&[]);
    if renderer.font_count() == 0 {
        eprintln!("没有系统字体，跳过文字排版测试");
        return;
    }
    let short = renderer
        .fill(TEMPLATE, &vars("sans-serif", "Cotton Tee"))
        .unwrap();
    assert_eq!((tspans(&short), font_size(&short)), (1, 60.0));

    let long_title =
        "Breathable Organic Cotton Crew Neck T-Shirt for Summer Travel and Everyday Wear";
    let long = renderer
        .fill(TEMPLATE, &vars("sans-serif", long_title))
        .unwrap();
    assert_eq!(tspans(&long), 2, "{long}");
    assert!(font_size(&long) <= 60.0);

    // 渲染出来的文字在 720 像素宽度以内（左边距 40，右边留白）
    let image = renderer
        .render(TEMPLATE, &vars("sans-serif", long_title))
        .unwrap();
    let bounds = bounds_on_white(&image, 10).unwrap();
    assert!(bounds.x >= 30, "{bounds:?}");
    assert!(bounds.x + bounds.width <= 40 + 720 + 10, "{bounds:?}");
}

#[cfg(windows)]
#[test]
fn chinese_text_renders_with_system_fonts() {
    // Windows 自带微软雅黑；字体名写错时也会回退到有中文字的字体
    let renderer = SvgRenderer::new(&[]);
    for font in ["Microsoft YaHei", "不存在的字体"] {
        let title = "夏季新款纯棉短袖T恤，宽松百搭，透气不闷汗，男女同款情侣装，多色可选";
        let svg = renderer.fill(TEMPLATE, &vars(font, title)).unwrap();
        assert_eq!(tspans(&svg), 2, "{svg}");
        let image = renderer.render(TEMPLATE, &vars(font, title)).unwrap();
        // 设置 PRISM_DUMP_DIR 时把渲染结果存下来，人工看效果
        if let Some(dir) = std::env::var_os("PRISM_DUMP_DIR") {
            let name = if font.is_ascii() {
                "yahei.png"
            } else {
                "fallback.png"
            };
            image.save(std::path::Path::new(&dir).join(name)).unwrap();
        }
        let bounds = bounds_on_white(&image, 10).expect("应该画出了文字");
        // 两行字：高度大于一行
        assert!(bounds.height > 60, "{font}: {bounds:?}");
        assert!(
            bounds.x + bounds.width <= 40 + 720 + 10,
            "{font}: {bounds:?}"
        );
    }
}
