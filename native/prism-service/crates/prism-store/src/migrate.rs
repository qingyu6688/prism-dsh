//! 数据库迁移程序。
//!
//! 没用 `sqlx::migrate!`：sqlx 0.8 的 SQLite 迁移总是把文件包在一个事务里，忽略文件开头的
//! `-- no-transaction` 标记。而 SQLite 改列约束只能重建表，重建表必须在事务外关掉外键，
//! 否则删旧表会级联删除子表数据（迁移 0002 就是这种情况）。所以自己写一个，规则：
//!
//! - 迁移文件在仓库根目录的 `migrations/`，按版本号顺序执行，执行过的记在 `schema_migration` 表
//! - 文件以 `-- no-transaction` 开头：先关外键，原样执行（文件自己写 `BEGIN` / `COMMIT`），
//!   然后检查外键完整性，再打开外键
//! - 其他文件：放在一个 `BEGIN IMMEDIATE` 事务里执行
//! - 记录每个文件的 SHA-256，已执行的文件被改动过就拒绝启动（已发布的迁移不能修改）
//! - 兼容旧库：早期开发版用 sqlx 迁移，`_sqlx_migrations` 里的记录会导入进来

use sha2::{Digest, Sha256};
use sqlx::{Connection, SqliteConnection};

use crate::{Result, StoreError};

/// 一个迁移文件。新增迁移时在 [`MIGRATIONS`] 末尾加一行。
pub(crate) struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../../../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "catalog",
        sql: include_str!("../../../migrations/0002_catalog.sql"),
    },
    Migration {
        version: 3,
        name: "import_mapping",
        sql: include_str!("../../../migrations/0003_import_mapping.sql"),
    },
    Migration {
        version: 4,
        name: "agent_plan",
        sql: include_str!("../../../migrations/0004_agent_plan.sql"),
    },
    Migration {
        version: 5,
        name: "notification",
        sql: include_str!("../../../migrations/0005_notification.sql"),
    },
    Migration {
        version: 6,
        name: "shared_api_credentials",
        sql: include_str!("../../../migrations/0006_shared_api_credentials.sql"),
    },
];

const NO_TRANSACTION: &str = "-- no-transaction";

/// 执行所有未执行的迁移。
pub(crate) async fn run(conn: &mut SqliteConnection, now: i64) -> Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS schema_migration (
             version    INTEGER PRIMARY KEY,
             name       TEXT    NOT NULL,
             checksum   TEXT    NOT NULL,
             applied_at INTEGER NOT NULL
         )",
    )
    .execute(&mut *conn)
    .await?;
    import_sqlx_history(conn, now).await?;

    for migration in MIGRATIONS {
        let checksum = checksum(migration.sql);
        let recorded: Option<String> =
            sqlx::query_scalar("SELECT checksum FROM schema_migration WHERE version = ?")
                .bind(migration.version)
                .fetch_optional(&mut *conn)
                .await?;
        match recorded.as_deref() {
            None => apply(conn, migration, &checksum, now).await?,
            // 从 sqlx 导入的记录没有校验和，补上当前的
            Some("") => {
                sqlx::query("UPDATE schema_migration SET checksum = ? WHERE version = ?")
                    .bind(&checksum)
                    .bind(migration.version)
                    .execute(&mut *conn)
                    .await?;
            }
            Some(existing) if existing == checksum => {}
            Some(_) => {
                return Err(StoreError::Migration(format!(
                    "迁移 {:04}_{} 已经执行过，但文件内容被改动了。已发布的迁移不能修改，请新增迁移文件",
                    migration.version, migration.name
                )));
            }
        }
    }
    Ok(())
}

async fn apply(
    conn: &mut SqliteConnection,
    migration: &Migration,
    checksum: &str,
    now: i64,
) -> Result<()> {
    let failed = |error: sqlx::Error| {
        StoreError::Migration(format!(
            "执行迁移 {:04}_{} 失败：{error}",
            migration.version, migration.name
        ))
    };

    if migration.sql.starts_with(NO_TRANSACTION) {
        sqlx::raw_sql("PRAGMA foreign_keys = OFF")
            .execute(&mut *conn)
            .await?;
        let result = sqlx::raw_sql(migration.sql).execute(&mut *conn).await;
        if result.is_err() {
            // 文件自己开的事务可能还没结束，回滚掉；没有进行中的事务时这里会报错，不影响后续
            if let Err(rollback_error) = sqlx::raw_sql("ROLLBACK").execute(&mut *conn).await {
                drop(rollback_error);
            }
        }
        let violations: Vec<(String, i64, String)> =
            sqlx::query_as("SELECT \"table\", rowid, parent FROM pragma_foreign_key_check")
                .fetch_all(&mut *conn)
                .await?;
        sqlx::raw_sql("PRAGMA foreign_keys = ON")
            .execute(&mut *conn)
            .await?;
        result.map_err(failed)?;
        if let Some((table, rowid, parent)) = violations.first() {
            return Err(StoreError::Migration(format!(
                "迁移 {:04}_{} 之后外键检查不通过：{table} 表第 {rowid} 行引用的 {parent} 不存在（共 {} 处）",
                migration.version,
                migration.name,
                violations.len()
            )));
        }
        record(conn, migration, checksum, now).await
    } else {
        let mut tx = conn.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::raw_sql(migration.sql)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
        record(&mut tx, migration, checksum, now).await?;
        tx.commit().await?;
        Ok(())
    }
}

async fn record(
    conn: &mut SqliteConnection,
    migration: &Migration,
    checksum: &str,
    now: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO schema_migration (version, name, checksum, applied_at) VALUES (?, ?, ?, ?)",
    )
    .bind(migration.version)
    .bind(migration.name)
    .bind(checksum)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// 早期开发版用 sqlx 迁移，把它记录的已执行版本导入进来，避免重复执行。
async fn import_sqlx_history(conn: &mut SqliteConnection, now: i64) -> Result<()> {
    let has_sqlx_table: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_optional(&mut *conn)
    .await?;
    if has_sqlx_table.is_none() {
        return Ok(());
    }
    sqlx::query(
        "INSERT OR IGNORE INTO schema_migration (version, name, checksum, applied_at)
         SELECT version, description, '', ? FROM _sqlx_migrations WHERE success = 1",
    )
    .bind(now)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

fn checksum(sql: &str) -> String {
    // 统一换行符，避免 Windows 上检出成 CRLF 时校验和变化
    let normalized = sql.replace("\r\n", "\n");
    Sha256::digest(normalized.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_migration_file_is_registered_in_order() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations");
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| name.ends_with(".sql"))
            .collect();
        files.sort();
        assert_eq!(
            files.len(),
            MIGRATIONS.len(),
            "migrations/ 里有文件没登记到 MIGRATIONS"
        );
        for (file, migration) in files.iter().zip(MIGRATIONS) {
            assert_eq!(
                file,
                &format!("{:04}_{}.sql", migration.version, migration.name)
            );
        }
    }

    #[test]
    fn versions_are_increasing() {
        for pair in MIGRATIONS.windows(2) {
            assert!(pair[0].version < pair[1].version);
        }
    }

    /// 用 sqlx 迁移过、只有 0001 且已有数据的旧库，升级后数据完整、外键指向正确。
    #[tokio::test]
    async fn upgrades_a_database_migrated_by_sqlx() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("old.db");
        let mut conn = SqliteConnection::connect_with(
            &sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .foreign_keys(true),
        )
        .await
        .unwrap();
        sqlx::raw_sql(MIGRATIONS[0].sql)
            .execute(&mut conn)
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE _sqlx_migrations (
                 version BIGINT PRIMARY KEY, description TEXT NOT NULL,
                 installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                 success BOOLEAN NOT NULL, checksum BLOB NOT NULL, execution_time BIGINT NOT NULL);
             INSERT INTO _sqlx_migrations VALUES (1, 'init', CURRENT_TIMESTAMP, 1, x'00', 1);
             INSERT INTO batch (id, seq, mode, variants_per_source, params, status, created_at, updated_at)
                 VALUES ('b1', 1, 'theme', 4, '{}', 'running', 1, 1);
             INSERT INTO source_image (id, batch_id, seq, file_path, sha256, width, height, created_at, updated_at)
                 VALUES ('s1', 'b1', 1, 'a.png', 'aa', 10, 10, 1, 1);
             INSERT INTO job (id, batch_id, batch_seq, source_id, source_seq, kind, variant_index, status, created_at, updated_at)
                 VALUES ('j1', 'b1', 1, 's1', 1, 'generate', 1, 'succeeded', 1, 1);
             INSERT INTO image (id, job_id, batch_id, source_id, variant_index, file_path, format, width, height, sha256, created_at, updated_at)
                 VALUES ('i1', 'j1', 'b1', 's1', 1, 'r.png', 'png', 10, 10, 'bb', 1, 1);",
        )
        .execute(&mut conn)
        .await
        .unwrap();
        conn.close().await.unwrap();

        let store = crate::Store::open(&path).await.unwrap();
        let mut conn = store.pool.acquire().await.unwrap();

        // 重建 batch、job 时外键是关着的，结果图和原图都还在
        let images: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM image")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
        assert_eq!(images, 1);
        let (module, mode): (String, String) =
            sqlx::query_as("SELECT module, mode FROM batch WHERE id = 'b1'")
                .fetch_one(&mut *conn)
                .await
                .unwrap();
        assert_eq!((module.as_str(), mode.as_str()), ("image", "theme"));
        let (source, task_type): (Option<String>, String) =
            sqlx::query_as("SELECT source_id, task_type FROM job WHERE id = 'j1'")
                .fetch_one(&mut *conn)
                .await
                .unwrap();
        assert_eq!(source.as_deref(), Some("s1"));
        assert_eq!(task_type, "image.fission");

        // 改名没有把外键改写到临时表上
        let targets: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT \"table\" FROM pragma_foreign_key_list('image') ORDER BY 1",
        )
        .fetch_all(&mut *conn)
        .await
        .unwrap();
        assert_eq!(targets, ["batch", "job", "source_image"]);
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
        assert_eq!(foreign_keys, 1);

        // sqlx 的记录导入后补上了校验和
        let recorded: Vec<(i64, String)> =
            sqlx::query_as("SELECT version, checksum FROM schema_migration ORDER BY version")
                .fetch_all(&mut *conn)
                .await
                .unwrap();
        assert_eq!(recorded.len(), MIGRATIONS.len());
        assert!(recorded.iter().all(|(_, sum)| sum.len() == 64));

        // 已执行的迁移被改动过，拒绝启动
        sqlx::query("UPDATE schema_migration SET checksum = 'changed' WHERE version = 2")
            .execute(&mut *conn)
            .await
            .unwrap();
        drop(conn);
        store.close().await;
        let error = crate::Store::open(&path).await.unwrap_err();
        assert!(matches!(error, StoreError::Migration(_)), "{error}");
    }

    #[test]
    fn checksum_ignores_line_endings() {
        assert_eq!(checksum("a\r\nb"), checksum("a\nb"));
        assert_ne!(checksum("a"), checksum("b"));
        assert_eq!(checksum("").len(), 64);
    }
}
