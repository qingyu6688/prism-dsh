use std::path::Path;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::Result;

/// 连接池大小。SQLite 同一时间只有一个写入者，多出来的连接用于并发读（WAL 模式下读写不互相阻塞）。
const MAX_CONNECTIONS: u32 = 8;
/// 数据库被锁时等待的时间。引擎高并发写状态时会短暂排队，超过这个时间才报错。
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// 数据库入口。克隆成本很低（内部是连接池的引用），可以在各协程之间传递。
#[derive(Debug, Clone)]
pub struct Store {
    pub(crate) pool: SqlitePool,
}

impl Store {
    /// 打开（不存在则创建）数据库文件，并执行未执行过的迁移。
    pub async fn open(path: &Path) -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            // WAL 模式下 Normal 已经能保证崩溃后数据库不损坏，只可能丢最后一个事务
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT);

        let pool = SqlitePoolOptions::new()
            .max_connections(MAX_CONNECTIONS)
            .connect_with(options)
            .await?;

        let mut conn = pool.acquire().await?;
        crate::migrate::run(&mut conn, prism_core::now_ms()).await?;
        drop(conn);
        Ok(Self { pool })
    }

    /// 程序里最新的迁移版本。
    pub const LATEST_SCHEMA_VERSION: i64 =
        crate::migrate::MIGRATIONS[crate::migrate::MIGRATIONS.len() - 1].version;

    /// 数据库已经执行到的迁移版本，没有执行过任何迁移时是 0。
    pub async fn schema_version(&self) -> Result<i64> {
        let version: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM schema_migration")
            .fetch_one(&self.pool)
            .await?;
        Ok(version.unwrap_or(0))
    }

    /// 关闭连接池，等正在进行的查询结束。应用退出前调用。
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// 写事务：一开始就拿写锁（`BEGIN IMMEDIATE`）。
    ///
    /// 默认的 `BEGIN` 是先读后写，WAL 模式下如果别的连接在这期间提交了写入，
    /// 读事务升级成写事务时 SQLite 会直接报 "database is locked"，不会按 busy_timeout 等待。
    /// 一开始就拿写锁，冲突时会正常排队等待。
    pub(crate) async fn begin_write(&self) -> Result<Transaction<'static, Sqlite>> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }
}
