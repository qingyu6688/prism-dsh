//! 棱镜业务服务。
//!
//! 由 dsh 的 Host 插件 `@prism/dsh-host-service` 作为子进程启动，通过 stdin / stdout 上
//! 一行一条的 JSON-RPC 2.0 调用。接口约定见 `docs/prism/service-api.md`。

pub mod frame;
pub mod lock;
pub mod methods;
pub mod protocol;
pub mod server;

pub use methods::Context;
pub use server::{ServeOptions, serve};

/// 数据目录里的数据库文件名。
pub const DB_FILE: &str = "prism.db";

/// 进程退出码，和 `docs/prism/service-api.md` 第 1 节一致。Host 插件按这些值判断要不要重启。
pub mod exit_code {
    /// 正常退出（stdin 关闭或收到 `service.shutdown`）。
    pub const OK: i32 = 0;
    /// 其他错误。
    pub const GENERAL: i32 = 1;
    /// 启动参数错误。clap 解析失败时自己以 2 退出，这里只是写明。
    pub const USAGE: i32 = 2;
    /// 数据目录已被另一个服务进程占用。
    pub const DATA_DIR_LOCKED: i32 = 3;
    /// 数据目录无法创建，或数据库打不开（含迁移失败）。
    pub const STORAGE: i32 = 4;
}
