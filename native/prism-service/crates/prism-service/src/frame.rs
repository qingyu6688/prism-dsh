//! 按行读消息，限制单条消息的大小。
//!
//! 不用 `read_line`：它会把整行读进内存，对端发来一条没有换行的超大消息时内存没有上限。

use std::io;

use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// 单条消息默认上限 16 MB（`docs/prism/service-api.md` 第 2 节）。
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// 一行内容，不含结尾的 `\n` 和 `\r`。
    Line(Vec<u8>),
    /// 这一行超过了上限，内容已经丢弃。
    TooLarge,
}

/// 读下一行。对端关闭且没有剩余数据时返回 `None`；最后一行没有换行也照常返回。
pub async fn read_frame<R>(reader: &mut R, max_bytes: usize) -> io::Result<Option<Frame>>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
    let mut too_large = false;
    let mut got_any = false;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if !got_any {
                return Ok(None);
            }
            break;
        }
        got_any = true;
        let (chunk, found_newline) = match available.iter().position(|b| *b == b'\n') {
            Some(pos) => (&available[..pos], true),
            None => (available, false),
        };
        let consumed = chunk.len() + usize::from(found_newline);
        if !too_large {
            if line.len() + chunk.len() > max_bytes {
                too_large = true;
                line = Vec::new();
            } else {
                line.extend_from_slice(chunk);
            }
        }
        reader.consume(consumed);
        if found_newline {
            break;
        }
    }
    if too_large {
        return Ok(Some(Frame::TooLarge));
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    Ok(Some(Frame::Line(line)))
}

#[cfg(test)]
mod tests {
    use tokio::io::BufReader;

    use super::*;

    async fn frames(input: &[u8], max: usize, buf_size: usize) -> Vec<Frame> {
        let mut reader = BufReader::with_capacity(buf_size, input);
        let mut out = Vec::new();
        while let Some(frame) = read_frame(&mut reader, max).await.unwrap() {
            out.push(frame);
        }
        out
    }

    fn line(s: &str) -> Frame {
        Frame::Line(s.as_bytes().to_vec())
    }

    #[tokio::test]
    async fn splits_lines_and_strips_crlf() {
        let got = frames(b"a\r\nbb\n\nccc", 100, 2).await;
        assert_eq!(got, vec![line("a"), line("bb"), line(""), line("ccc")]);
    }

    #[tokio::test]
    async fn oversized_line_is_dropped_and_next_line_still_read() {
        let got = frames(b"0123456789\nok\n", 5, 3).await;
        assert_eq!(got, vec![Frame::TooLarge, line("ok")]);
    }

    #[tokio::test]
    async fn line_exactly_at_limit_is_kept() {
        let got = frames(b"12345\n", 5, 2).await;
        assert_eq!(got, vec![line("12345")]);
    }

    #[tokio::test]
    async fn empty_input_ends_immediately() {
        assert!(frames(b"", 5, 2).await.is_empty());
    }
}
