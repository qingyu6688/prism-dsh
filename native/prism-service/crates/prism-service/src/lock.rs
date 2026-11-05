//! 数据目录锁：同一个数据目录只能有一个服务进程。
//!
//! 两个进程同时写一个 SQLite 库虽然不会损坏数据，但生图批次的调度、素材文件的整理都假设
//! 只有自己在动这个目录。Host 重启服务时，旧进程还没退干净的话新进程会因为拿不到锁而退出（退出码 3），
//! Host 隔一秒再试即可。

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;

/// 锁文件名，放在数据目录下。
pub const LOCK_FILE: &str = "prism-service.lock";

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("数据目录已被另一个 prism-service 占用")]
    Held,
    #[error("无法创建锁文件：{0}")]
    Io(#[from] io::Error),
}

/// 独占数据目录。返回的文件要一直持有，文件关闭（进程退出）时锁自动释放。
pub fn acquire(data_dir: &Path) -> Result<File, LockError> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data_dir.join(LOCK_FILE))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(LockError::Held),
        Err(TryLockError::Error(e)) => Err(LockError::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_fails_until_first_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire(dir.path()).unwrap();
        assert!(matches!(acquire(dir.path()), Err(LockError::Held)));
        drop(first);
        assert!(acquire(dir.path()).is_ok());
    }
}
