//! 文字自动排版（详细设计 6.6）：按宽度换行，排不下就缩小字号，最小字号还排不下就截断加"…"。
//!
//! 和字体无关：字宽由调用方给出（100 号字下的宽度），这样能用假的测量函数单独测排版逻辑。

use std::collections::HashMap;

/// 测量用的参考字号，实际宽度按字号等比换算。
pub const MEASURE_SIZE: f32 = 100.0;
const ELLIPSIS: &str = "…";

/// 排版要求。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitSpec {
    pub max_width: f32,
    pub max_lines: usize,
    pub font_size: f32,
    pub min_font_size: f32,
    /// 每次缩小多少号
    pub step: f32,
}

/// 排版结果。
#[derive(Debug, Clone, PartialEq)]
pub struct FittedText {
    pub font_size: f32,
    pub lines: Vec<String>,
    /// 最小字号还排不下，截断了
    pub truncated: bool,
}

/// 断行的最小单位：一个中日韩文字、一个英文单词、或者一段空白。
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    Space,
}

fn is_cjk(c: char) -> bool {
    matches!(
        u32::from(c),
        0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x3FFFD
    )
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    let flush = |word: &mut String, tokens: &mut Vec<Token>| {
        if !word.is_empty() {
            tokens.push(Token::Word(std::mem::take(word)));
        }
    };
    for c in text.chars() {
        if c.is_whitespace() {
            flush(&mut word, &mut tokens);
            if tokens.last().is_some_and(|t| *t != Token::Space) {
                tokens.push(Token::Space);
            }
        } else if is_cjk(c) {
            flush(&mut word, &mut tokens);
            tokens.push(Token::Word(c.to_string()));
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut tokens);
    if tokens.last() == Some(&Token::Space) {
        tokens.pop();
    }
    tokens
}

/// 带缓存的测量：同一个片段只测一次。
struct Measurer<'a> {
    measure: &'a mut dyn FnMut(&str) -> f32,
    cache: HashMap<String, f32>,
}

impl Measurer<'_> {
    fn width(&mut self, text: &str, size: f32) -> f32 {
        let base = match self.cache.get(text) {
            Some(width) => *width,
            None => {
                let width = (self.measure)(text);
                self.cache.insert(text.to_owned(), width);
                width
            }
        };
        base * size / MEASURE_SIZE
    }

    /// 逐字相加，截断时用。
    fn chars_width(&mut self, text: &str, size: f32) -> f32 {
        text.chars().map(|c| self.width(&c.to_string(), size)).sum()
    }
}

/// 按宽度贪心换行。单个单词比一行还宽时拆成单个字符。
fn wrap(tokens: &[Token], size: f32, max_width: f32, measurer: &mut Measurer<'_>) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut width = 0.0;
    let mut pieces: Vec<String> = Vec::new();
    for token in tokens {
        match token {
            Token::Space => pieces.push(" ".into()),
            Token::Word(word) if measurer.width(word, size) > max_width => {
                pieces.extend(word.chars().map(|c| c.to_string()));
            }
            Token::Word(word) => pieces.push(word.clone()),
        }
    }
    for piece in pieces {
        let piece_width = measurer.width(&piece, size);
        if piece == " " {
            if line.is_empty() {
                continue;
            }
            if width + piece_width > max_width {
                lines.push(std::mem::take(&mut line));
                width = 0.0;
            } else {
                line.push(' ');
                width += piece_width;
            }
            continue;
        }
        if !line.is_empty() && width + piece_width > max_width {
            lines.push(line.trim_end().to_owned());
            line.clear();
            width = 0.0;
        }
        line.push_str(&piece);
        width += piece_width;
    }
    let last = line.trim_end();
    if !last.is_empty() {
        lines.push(last.to_owned());
    }
    lines
}

/// 排版：从原字号开始，排不下就缩小，最小字号还排不下就截断。
pub fn fit_text(text: &str, spec: &FitSpec, measure: &mut dyn FnMut(&str) -> f32) -> FittedText {
    let mut measurer = Measurer {
        measure,
        cache: HashMap::new(),
    };
    let tokens = tokenize(text);
    let max_lines = spec.max_lines.max(1);
    let min_size = spec.min_font_size.min(spec.font_size).max(1.0);
    let step = spec.step.max(0.5);

    let mut size = spec.font_size;
    loop {
        let lines = wrap(&tokens, size, spec.max_width, &mut measurer);
        if lines.len() <= max_lines {
            return FittedText {
                font_size: size,
                lines,
                truncated: false,
            };
        }
        if size - step < min_size {
            break;
        }
        size -= step;
    }

    let size = min_size;
    let mut lines = wrap(&tokens, size, spec.max_width, &mut measurer);
    lines.truncate(max_lines);
    if let Some(last) = lines.last_mut() {
        let ellipsis = measurer.width(ELLIPSIS, size);
        while !last.is_empty() && measurer.chars_width(last, size) + ellipsis > spec.max_width {
            last.pop();
        }
        let trimmed = last.trim_end().to_owned();
        *last = trimmed + ELLIPSIS;
    }
    FittedText {
        font_size: size,
        lines,
        truncated: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 假的字宽：100 号字下中文 100、英文字母 50、空格 25。
    fn fake(text: &str) -> f32 {
        text.chars()
            .map(|c| {
                if is_cjk(c) {
                    100.0
                } else if c == ' ' {
                    25.0
                } else {
                    50.0
                }
            })
            .sum()
    }

    fn spec(max_width: f32, max_lines: usize) -> FitSpec {
        FitSpec {
            max_width,
            max_lines,
            font_size: 40.0,
            min_font_size: 20.0,
            step: 2.0,
        }
    }

    fn fit(text: &str, spec: &FitSpec) -> FittedText {
        fit_text(text, spec, &mut fake)
    }

    #[test]
    fn short_text_keeps_its_size() {
        let fitted = fit("纯棉T恤", &spec(400.0, 1));
        assert_eq!(
            (fitted.font_size, fitted.lines.as_slice()),
            (40.0, ["纯棉T恤".to_owned()].as_slice())
        );
        assert!(!fitted.truncated);
    }

    #[test]
    fn chinese_breaks_between_characters() {
        // 40 号中文每字 40，一行 160 放 4 个字
        let fitted = fit("夏季清凉透气短袖", &spec(160.0, 2));
        assert_eq!(fitted.lines, ["夏季清凉", "透气短袖"]);
        assert_eq!(fitted.font_size, 40.0);
    }

    #[test]
    fn english_breaks_between_words() {
        // 40 号字母 20，空格 10
        let fitted = fit("Soft cotton tee", &spec(220.0, 2));
        assert_eq!(fitted.lines, ["Soft cotton", "tee"]);
    }

    #[test]
    fn shrinks_before_adding_lines() {
        // 8 个字在 40 号要 320，一行只能 300：缩到 36 号刚好 288
        let fitted = fit("夏季清凉透气短袖", &spec(300.0, 1));
        assert_eq!(fitted.lines, ["夏季清凉透气短袖"]);
        assert_eq!(fitted.font_size, 36.0);
    }

    #[test]
    fn truncates_at_the_minimum_size() {
        let fitted = fit(&"字".repeat(40), &spec(200.0, 2));
        assert!(fitted.truncated);
        assert_eq!(fitted.font_size, 20.0);
        assert_eq!(fitted.lines.len(), 2);
        assert_eq!(fitted.lines[0], "字".repeat(10));
        assert!(fitted.lines[1].ends_with('…'));
        assert!(fitted.lines[1].chars().count() <= 10);
    }

    #[test]
    fn very_long_words_are_split() {
        let fitted = fit("Supercalifragilistic", &spec(200.0, 3));
        assert!(fitted.lines.len() > 1);
        assert_eq!(fitted.lines.concat(), "Supercalifragilistic");
    }

    #[test]
    fn measures_each_piece_once() {
        let mut calls = 0;
        let mut counting = |text: &str| {
            calls += 1;
            fake(text)
        };
        fit_text(&"字".repeat(30), &spec(100.0, 1), &mut counting);
        // 同一个字只测一次（外加省略号和截断时逐字测，也都走缓存）
        assert!(calls <= 3, "{calls}");
    }
}
