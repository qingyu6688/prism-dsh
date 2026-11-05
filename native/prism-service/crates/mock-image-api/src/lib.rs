//! 模拟生图服务。
//!
//! 接口格式和 OpenAI 一致，压测和测试用它代替真实接口，不花钱也不需要 Key。
//! 可以作为命令行程序单独运行，也可以在测试里用 [`MockServer::start`] 在随机端口上启动。
//! 接口和参数见 `docs/api.md` 第 6 节。

mod config;
mod handlers;
mod state;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub use config::{MockConfig, parse_latency};
pub use handlers::JOB_ID_HEADER;
use state::AppState;
pub use state::Stats;

/// 参考图可能有十几 MB，放宽 axum 默认的 2 MB 请求体限制。
const MAX_BODY_BYTES: usize = 50 * 1024 * 1024;

/// 构造路由。
fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(handlers::health))
        .route("/v1/models", get(handlers::models))
        .route("/v1/images/generations", post(handlers::generations))
        .route("/v1/images/edits", post(handlers::edits))
        .route("/v1/chat/completions", post(handlers::chat_completions))
        .route("/v1/async/images", post(handlers::async_submit))
        .route("/v1/async/tasks/{task_id}", get(handlers::async_poll))
        .route("/stats", get(handlers::stats))
        .route("/stats/reset", post(handlers::reset_stats))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

/// 在指定的监听器上运行，直到 `shutdown` 完成。命令行入口用。
pub async fn serve(
    listener: TcpListener,
    config: MockConfig,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    config.validate().map_err(invalid_input)?;
    let state = Arc::new(AppState::new(config));
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
}

/// 在测试进程里运行的模拟服务，离开作用域时自动停止。
pub struct MockServer {
    addr: SocketAddr,
    state: Arc<AppState>,
    handle: JoinHandle<()>,
}

impl MockServer {
    /// 在 `127.0.0.1` 的随机空闲端口上启动。
    pub async fn start(config: MockConfig) -> std::io::Result<Self> {
        config.validate().map_err(invalid_input)?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(AppState::new(config));
        let app = router(Arc::clone(&state));
        let handle = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app).await {
                tracing::error!("模拟服务异常退出：{error}");
            }
        });
        Ok(Self {
            addr,
            state,
            handle,
        })
    }

    /// 形如 `http://127.0.0.1:12345/v1` 的接口地址，直接填进 API 配置的 Base URL。
    pub fn base_url(&self) -> String {
        format!("http://{}/v1", self.addr)
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// 当前统计，不用走 HTTP。
    pub fn stats(&self) -> Stats {
        self.state.stats()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

fn invalid_input(message: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
}
