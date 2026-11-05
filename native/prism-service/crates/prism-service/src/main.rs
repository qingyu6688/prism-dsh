//! prism-service 入口：解析参数、锁定数据目录、打开数据库，然后在 stdin / stdout 上提供 JSON-RPC。

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use prism_service::{Context, DB_FILE, ServeOptions, exit_code, lock, serve};
use prism_store::Store;

#[derive(Debug, Parser)]
#[command(
    name = "prism-service",
    version,
    about = "棱镜业务服务：通过 stdin / stdout 上的 JSON-RPC 提供商品库、规则包、图片处理"
)]
struct Args {
    /// 数据目录：数据库、素材文件、用户规则包都放在这里，不存在时自动创建
    #[arg(long)]
    data_dir: PathBuf,

    /// 日志级别，日志写到 stderr
    #[arg(long, value_enum, default_value_t = LogLevel::Info)]
    log_level: LogLevel,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    fn as_filter(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }
}

fn main() {
    // 参数错误时 clap 自己打印说明并以退出码 2 退出
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_target(false)
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            args.log_level.as_filter(),
        ))
        .init();

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            tracing::error!(error = %e, "启动异步运行时失败");
            std::process::exit(exit_code::GENERAL);
        }
    };
    let code = runtime.block_on(run(args));
    // tokio 读 stdin 用的是阻塞线程，正常关闭运行时会一直等那次读取返回，所以直接放手
    runtime.shutdown_background();
    std::process::exit(code);
}

async fn run(args: Args) -> i32 {
    let data_dir = match std::path::absolute(&args.data_dir) {
        Ok(dir) => dir,
        Err(e) => {
            tracing::error!(error = %e, "数据目录路径无效");
            return exit_code::USAGE;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        tracing::error!(error = %e, dir = %data_dir.display(), "无法创建数据目录");
        return exit_code::STORAGE;
    }

    let _lock = match lock::acquire(&data_dir) {
        Ok(file) => file,
        Err(lock::LockError::Held) => {
            tracing::error!(dir = %data_dir.display(), "数据目录已被另一个 prism-service 占用");
            return exit_code::DATA_DIR_LOCKED;
        }
        Err(e) => {
            tracing::error!(error = %e, dir = %data_dir.display(), "无法锁定数据目录");
            return exit_code::STORAGE;
        }
    };

    let store = match Store::open(&data_dir.join(DB_FILE)).await {
        Ok(store) => store,
        Err(e) => {
            tracing::error!(error = %e, "打开数据库失败");
            return exit_code::STORAGE;
        }
    };

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        dir = %data_dir.display(),
        "prism-service 已启动"
    );
    let ctx = Context::new(store.clone(), data_dir);
    let result = serve(
        tokio::io::stdin(),
        tokio::io::stdout(),
        ctx,
        ServeOptions::default(),
    )
    .await;
    store.close().await;

    match result {
        Ok(()) => {
            tracing::info!("prism-service 已退出");
            exit_code::OK
        }
        Err(e) => {
            tracing::error!(error = %e, "读写 stdin / stdout 失败");
            exit_code::GENERAL
        }
    }
}
