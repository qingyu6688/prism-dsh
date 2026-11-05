//! 启动真正的 prism-service 可执行文件，检查参数、目录锁和退出码。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_prism-service");
const WAIT: Duration = Duration::from_secs(20);

fn spawn(data_dir: &Path) -> Child {
    Command::new(BIN)
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--log-level")
        .arg("warn")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// 等进程退出，返回退出码；超时就杀掉并让测试失败。
fn wait_code(child: &mut Child) -> i32 {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code().unwrap();
        }
        if start.elapsed() > WAIT {
            child.kill().ok();
            panic!("prism-service 没有在 {WAIT:?} 内退出");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn ping(child: &mut Child) -> Value {
    let stdin = child.stdin.as_mut().unwrap();
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"service.ping\"}\n")
        .unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn creates_data_dir_answers_ping_and_exits_when_stdin_closes() {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().join("新建的数据目录");
    let mut child = spawn(&data_dir);

    assert_eq!(ping(&mut child)["result"]["pong"], true);
    assert!(data_dir.join("prism.db").exists());
    assert!(data_dir.join("prism-service.lock").exists());

    drop(child.stdin.take());
    assert_eq!(wait_code(&mut child), 0);
}

#[test]
fn second_instance_on_same_dir_exits_with_3() {
    let dir = TempDir::new().unwrap();
    let mut first = spawn(dir.path());
    // 先确认第一个已经拿到锁
    assert_eq!(ping(&mut first)["result"]["pong"], true);

    let mut second = spawn(dir.path());
    assert_eq!(wait_code(&mut second), 3);

    drop(first.stdin.take());
    assert_eq!(wait_code(&mut first), 0);

    // 第一个退出后锁释放，新进程可以正常启动
    let mut third = spawn(dir.path());
    assert_eq!(ping(&mut third)["result"]["pong"], true);
    drop(third.stdin.take());
    assert_eq!(wait_code(&mut third), 0);
}

#[test]
fn missing_data_dir_argument_exits_with_2() {
    let status = Command::new(BIN)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(2));
}

#[test]
fn unusable_data_dir_exits_with_4() {
    let dir = TempDir::new().unwrap();
    // 数据目录的位置被一个普通文件占了，无法创建目录
    let blocked = dir.path().join("file");
    std::fs::write(&blocked, b"x").unwrap();
    let mut child = spawn(&blocked.join("data"));
    assert_eq!(wait_code(&mut child), 4);
}
