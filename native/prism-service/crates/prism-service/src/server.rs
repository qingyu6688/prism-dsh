//! 服务循环：一个读协程按行收请求，每个请求一个任务并发处理，响应经过一个写协程按行写回。

use std::io;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::frame::{DEFAULT_MAX_MESSAGE_BYTES, Frame, read_frame};
use crate::methods::{self, Context, SHUTDOWN};
use crate::protocol::{self, Incoming, Request, RpcError};

/// 待写出的响应队列长度。写得比处理慢时，处理任务在发送处排队，不会无限占内存。
const OUTBOX_CAPACITY: usize = 256;

#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// 单条消息上限。
    pub max_message_bytes: usize,
    /// 退出时等进行中请求的最长时间，超时的请求直接放弃。
    pub drain_timeout: Duration,
}

impl Default for ServeOptions {
    fn default() -> Self {
        Self {
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
            drain_timeout: Duration::from_secs(10),
        }
    }
}

/// 处理请求，直到输入关闭或收到 `service.shutdown`；返回前等进行中的请求结束并写完响应。
pub async fn serve<R, W>(
    reader: R,
    writer: W,
    ctx: Arc<Context>,
    options: ServeOptions,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (outbox, rx) = mpsc::channel::<String>(OUTBOX_CAPACITY);
    let writer_task = tokio::spawn(write_loop(writer, rx));
    let mut reader = BufReader::new(reader);
    let mut shutdown = ctx.shutdown_signal();
    let mut tasks = JoinSet::new();
    let mut read_result = Ok(());

    loop {
        let frame = tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => {
                tracing::info!("收到 service.shutdown，停止接收新请求");
                break;
            }
            frame = read_frame(&mut reader, options.max_message_bytes) => frame,
        };
        match frame {
            Ok(None) => {
                tracing::info!("输入已关闭，准备退出");
                break;
            }
            Err(e) => {
                tracing::error!(error = %e, "读取输入失败");
                read_result = Err(e);
                break;
            }
            Ok(Some(Frame::TooLarge)) => {
                let error = RpcError::invalid_request(format!(
                    "消息超过 {} MB 上限",
                    options.max_message_bytes / (1024 * 1024)
                ));
                send(&outbox, protocol::failure(&Value::Null, &error)).await;
            }
            Ok(Some(Frame::Line(line))) => {
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                match protocol::parse(&line) {
                    Incoming::Invalid { id, error } => {
                        tracing::warn!(code = error.code, message = %error.message, "收到无效消息");
                        send(&outbox, protocol::failure(&id, &error)).await;
                    }
                    Incoming::Request(request) => {
                        tasks.spawn(handle(ctx.clone(), outbox.clone(), request));
                    }
                }
            }
        }
        // 回收已经结束的任务，长时间运行时 JoinSet 不会越攒越多
        while let Some(result) = tasks.try_join_next() {
            log_task_result(result);
        }
    }

    let drained = tokio::time::timeout(options.drain_timeout, async {
        while let Some(result) = tasks.join_next().await {
            log_task_result(result);
        }
    })
    .await;
    if drained.is_err() {
        tracing::warn!(remaining = tasks.len(), "等待进行中的请求超时，直接放弃");
        tasks.shutdown().await;
    }

    drop(outbox);
    let write_result = match writer_task.await {
        Ok(result) => result,
        Err(e) => Err(io::Error::other(e)),
    };
    read_result.and(write_result)
}

async fn handle(ctx: Arc<Context>, outbox: mpsc::Sender<String>, request: Request) {
    let Request { id, method, params } = request;
    let outcome = methods::dispatch(&ctx, &method, params).await;
    if let Err(error) = &outcome {
        tracing::debug!(%method, code = error.code, message = %error.message, "请求失败");
    }
    if let Some(id) = id {
        let line = match &outcome {
            Ok(result) => protocol::success(&id, result),
            Err(error) => protocol::failure(&id, error),
        };
        send(&outbox, line).await;
    }
    // 响应已经排进写队列，服务循环退出前会写完
    if method == SHUTDOWN && outcome.is_ok() {
        ctx.request_shutdown();
    }
}

async fn send(outbox: &mpsc::Sender<String>, line: String) {
    if outbox.send(line).await.is_err() {
        tracing::warn!("输出已关闭，响应没有写出");
    }
}

fn log_task_result(result: Result<(), tokio::task::JoinError>) {
    if let Err(e) = result
        && e.is_panic()
    {
        tracing::error!(error = %e, "处理请求时程序出错");
    }
}

async fn write_loop<W>(mut writer: W, mut rx: mpsc::Receiver<String>) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    while let Some(line) = rx.recv().await {
        writer.write_all(line.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        // 队列里还有就先不 flush，连续多条响应时少几次系统调用
        if rx.is_empty() {
            writer.flush().await?;
        }
    }
    writer.flush().await
}
