//! 模拟生图服务的命令行入口。用法见 `docs/api.md` 第 6 节。

use std::net::SocketAddr;
use std::time::Duration;

use clap::Parser;
use mock_image_api::{MockConfig, parse_latency};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "mock-image-api", version, about = "棱镜的模拟生图服务")]
struct Args {
    /// 监听地址，默认只监听本机
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// 监听端口
    #[arg(long, default_value_t = 18080)]
    port: u16,
    /// 成功响应前的延迟（毫秒），写成区间 2000-8000 或固定值 500
    #[arg(long, default_value = "0", value_parser = parse_latency)]
    latency: (u64, u64),
    /// 每分钟允许的请求数，超过返回 429；不填不限
    #[arg(long)]
    rate_limit_per_min: Option<u32>,
    /// 随机返回 429 的比例
    #[arg(long, default_value_t = 0.0)]
    error_429: f64,
    /// 随机返回 500 的比例
    #[arg(long, default_value_t = 0.0)]
    error_500: f64,
    /// 随机挂起不响应的比例
    #[arg(long, default_value_t = 0.0)]
    timeout: f64,
    /// 挂起多少秒后才返回 504
    #[arg(long, default_value_t = 120)]
    hang_secs: u64,
    /// 随机返回坏 JSON 的比例
    #[arg(long, default_value_t = 0.0)]
    bad_json: f64,
    /// 随机返回内容审核拒绝的比例
    #[arg(long, default_value_t = 0.0)]
    content_rejected: f64,
    /// 返回图片的边长（像素）
    #[arg(long, default_value_t = 64)]
    image_size: u32,
    /// 随机数种子，固定后错误注入的顺序可复现
    #[arg(long, default_value_t = 42)]
    seed: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let config = MockConfig {
        latency_ms: args.latency,
        rate_limit_per_min: args.rate_limit_per_min,
        error_429: args.error_429,
        error_500: args.error_500,
        timeout: args.timeout,
        bad_json: args.bad_json,
        content_rejected: args.content_rejected,
        hang: Duration::from_secs(args.hang_secs),
        image_size: args.image_size,
        seed: args.seed,
    };
    config.validate().map_err(anyhow::Error::msg)?;

    let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("模拟生图服务已启动：http://{addr}/v1");
    mock_image_api::serve(listener, config, shutdown_signal()).await?;
    Ok(())
}

async fn shutdown_signal() {
    // 收不到 Ctrl+C 信号时（比如在某些容器里）就一直运行，由外部结束进程
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
    tracing::info!("收到退出信号，正在关闭");
}
