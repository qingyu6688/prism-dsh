//! 棱镜命令行。
//!
//! 目前有 `bench`（压测引擎）。`run` / `status` / `profile` 随裂变编排一起实现，用法见 `docs/api.md` 第 5 节。

mod bench;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

/// 棱镜（Prism）命令行：不开界面跑裂变、查进度、压测引擎。
#[derive(Debug, Parser)]
#[command(name = "prism-cli", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 对模拟服务（或任意 OpenAI 兼容接口）压测调度引擎
    Bench(bench::BenchArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .init();

    match Cli::parse().command {
        Command::Bench(args) => bench::run(args).await,
    }
}
