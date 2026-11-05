//! 在进程内跑服务循环，用内存管道收发 JSON-RPC 消息。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use prism_service::{Context, DB_FILE, ServeOptions, serve};
use prism_store::Store;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};
use tokio::task::JoinHandle;

struct Harness {
    to_service: WriteHalf<DuplexStream>,
    from_service: BufReader<ReadHalf<DuplexStream>>,
    service: JoinHandle<std::io::Result<()>>,
    ctx: Arc<Context>,
    _dir: TempDir,
}

async fn start(options: ServeOptions) -> Harness {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join(DB_FILE)).await.unwrap();
    let ctx = Context::new(store, dir.path().to_path_buf());
    let (client, server) = tokio::io::duplex(64 * 1024);
    let (server_read, server_write) = tokio::io::split(server);
    let service = tokio::spawn(serve(server_read, server_write, ctx.clone(), options));
    let (client_read, client_write) = tokio::io::split(client);
    Harness {
        to_service: client_write,
        from_service: BufReader::new(client_read),
        service,
        ctx,
        _dir: dir,
    }
}

impl Harness {
    async fn send(&mut self, line: &str) {
        self.to_service.write_all(line.as_bytes()).await.unwrap();
        self.to_service.write_all(b"\n").await.unwrap();
    }

    async fn recv(&mut self) -> Value {
        let mut line = String::new();
        tokio::time::timeout(
            Duration::from_secs(5),
            self.from_service.read_line(&mut line),
        )
        .await
        .expect("5 秒内没有收到响应")
        .unwrap();
        serde_json::from_str(&line).unwrap()
    }

    async fn call(&mut self, id: i64, method: &str) -> Value {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method}).to_string())
            .await;
        self.recv().await
    }
}

#[tokio::test]
async fn ping_and_version() {
    let mut h = start(ServeOptions::default()).await;

    assert_eq!(
        h.call(1, "service.ping").await,
        json!({"jsonrpc": "2.0", "id": 1, "result": {"pong": true}})
    );

    let reply = h.call(2, "service.version").await;
    let result = &reply["result"];
    assert_eq!(result["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(result["schemaVersion"], Store::LATEST_SCHEMA_VERSION);
    assert_eq!(
        result["dataDir"],
        h.ctx.data_dir().display().to_string().as_str()
    );
}

#[tokio::test]
async fn errors_use_jsonrpc_codes() {
    let mut h = start(ServeOptions::default()).await;

    let reply = h.call(1, "catalog.nope").await;
    assert_eq!(reply["id"], 1);
    assert_eq!(reply["error"]["code"], -32601);

    h.send("{not json").await;
    let reply = h.recv().await;
    assert_eq!(reply["id"], Value::Null);
    assert_eq!(reply["error"]["code"], -32700);

    h.send(r#"{"jsonrpc":"2.0","id":5,"method":"service.ping","params":[1]}"#)
        .await;
    let reply = h.recv().await;
    assert_eq!(reply["id"], 5);
    assert_eq!(reply["error"]["code"], -32602);
    assert_eq!(reply["error"]["data"]["code"], "invalid_input");

    // 空行忽略，服务继续工作
    h.send("").await;
    assert_eq!(h.call(6, "service.ping").await["result"]["pong"], true);
}

#[tokio::test]
async fn notification_gets_no_reply() {
    let mut h = start(ServeOptions::default()).await;
    h.send(r#"{"jsonrpc":"2.0","method":"service.ping"}"#).await;
    // 如果通知也回了响应，下面收到的第一条就不是 id 9
    assert_eq!(h.call(9, "service.ping").await["id"], 9);
}

#[tokio::test]
async fn oversized_message_is_rejected_and_service_continues() {
    let mut h = start(ServeOptions {
        max_message_bytes: 1024,
        ..ServeOptions::default()
    })
    .await;
    let big = json!({"jsonrpc": "2.0", "id": 1, "method": "service.ping", "params": {"pad": "x".repeat(2048)}});
    h.send(&big.to_string()).await;
    let reply = h.recv().await;
    assert_eq!(reply["id"], Value::Null);
    assert_eq!(reply["error"]["code"], -32600);

    assert_eq!(h.call(2, "service.ping").await["result"]["pong"], true);
}

#[tokio::test]
async fn many_requests_all_get_matching_replies() {
    let mut h = start(ServeOptions::default()).await;
    for id in 0..50 {
        let method = if id % 2 == 0 {
            "service.ping"
        } else {
            "service.version"
        };
        h.send(&json!({"jsonrpc": "2.0", "id": id, "method": method}).to_string())
            .await;
    }
    let mut seen = Vec::new();
    for _ in 0..50 {
        let reply = h.recv().await;
        assert!(reply.get("result").is_some(), "{reply}");
        seen.push(reply["id"].as_i64().unwrap());
    }
    seen.sort_unstable();
    assert_eq!(seen, (0..50).collect::<Vec<_>>());
}

#[tokio::test]
async fn shutdown_replies_then_stops() {
    let mut h = start(ServeOptions::default()).await;
    assert_eq!(
        h.call(1, "service.shutdown").await,
        json!({"jsonrpc": "2.0", "id": 1, "result": {}})
    );
    let result = tokio::time::timeout(Duration::from_secs(5), h.service)
        .await
        .expect("shutdown 后 5 秒内没有退出")
        .unwrap();
    assert!(result.is_ok());
}

#[tokio::test]
async fn closing_input_stops_service() {
    let mut h = start(ServeOptions::default()).await;
    h.send(r#"{"jsonrpc":"2.0","id":1,"method":"service.ping"}"#)
        .await;
    h.to_service.shutdown().await.unwrap();
    drop(h.to_service);
    // 关闭输入前发出的请求仍然要有响应
    let mut line = String::new();
    h.from_service.read_line(&mut line).await.unwrap();
    assert!(line.contains("\"pong\":true"), "{line}");
    let result = tokio::time::timeout(Duration::from_secs(5), h.service)
        .await
        .expect("输入关闭后 5 秒内没有退出")
        .unwrap();
    assert!(result.is_ok());
}
