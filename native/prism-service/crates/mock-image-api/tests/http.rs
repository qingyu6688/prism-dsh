//! 通过 HTTP 测模拟服务的各个接口和错误注入。

// 测试里的辅助函数出错就该直接失败，用 unwrap 最直接
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use mock_image_api::{JOB_ID_HEADER, MockConfig, MockServer};
use reqwest::multipart::{Form, Part};
use serde_json::{Value, json};

const PNG_SIGNATURE: &[u8] = &[0x89, b'P', b'N', b'G'];

async fn start(config: MockConfig) -> (MockServer, reqwest::Client) {
    let server = MockServer::start(config).await.unwrap();
    (server, reqwest::Client::new())
}

fn edit_form(prompt: &str) -> Form {
    Form::new()
        .text("model", "gpt-image-2.5-sunburst")
        .text("prompt", prompt.to_owned())
        .text("n", "2")
        .part(
            "image[]",
            Part::bytes(vec![1, 2, 3])
                .file_name("ref.png")
                .mime_str("image/png")
                .unwrap(),
        )
}

fn decode_png(b64: &str) -> Vec<u8> {
    BASE64.decode(b64).unwrap()
}

#[tokio::test]
async fn lists_models_including_the_default_image_model() {
    let (server, client) = start(MockConfig::default()).await;
    let body: Value = client
        .get(format!("{}/models", server.base_url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<&str> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    assert!(ids.contains(&"gpt-image-2.5-sunburst"));
}

#[tokio::test]
async fn generations_return_png_images() {
    let (server, client) = start(MockConfig::default()).await;
    let body: Value = client
        .post(format!("{}/images/generations", server.base_url()))
        .json(&json!({ "model": "gpt-image-2.5-sunburst", "prompt": "一只猫", "n": 3 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let data = body["data"].as_array().unwrap();
    assert_eq!(data.len(), 3);
    assert!(decode_png(data[0]["b64_json"].as_str().unwrap()).starts_with(PNG_SIGNATURE));
    assert_eq!(data[0]["revised_prompt"], "[mock] 一只猫");
    assert_eq!(server.stats().images_generated, 3);
}

#[tokio::test]
async fn edits_accept_multipart_with_reference_image() {
    let (server, client) = start(MockConfig::default()).await;
    let response = client
        .post(format!("{}/images/edits", server.base_url()))
        .multipart(edit_form("换成圣诞主题"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["data"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn edits_without_reference_image_are_rejected() {
    let (server, client) = start(MockConfig::default()).await;
    let form = Form::new().text("prompt", "没有参考图");
    let response = client
        .post(format!("{}/images/edits", server.base_url()))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn chat_returns_a_plan_with_requested_variant_count() {
    let (server, client) = start(MockConfig::default()).await;
    let body: Value = client
        .post(format!("{}/chat/completions", server.base_url()))
        .json(&json!({
            "model": "gpt-4.1",
            "response_format": { "type": "json_object" },
            "messages": [{ "role": "user", "content": [
                { "type": "text", "text": "规划裂变 variant_count=5" },
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } }
            ]}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let content = body["choices"][0]["message"]["content"].as_str().unwrap();
    let plan: Value = serde_json::from_str(content).unwrap();
    assert_eq!(plan["variants"].as_array().unwrap().len(), 5);
    assert!(plan["analysis"]["subject"].is_string());
}

#[tokio::test]
async fn chat_calls_the_first_tool_when_tools_are_given() {
    let (server, client) = start(MockConfig::default()).await;
    let body: Value = client
        .post(format!("{}/chat/completions", server.base_url()))
        .json(&json!({
            "model": "gpt-4.1",
            "messages": [{ "role": "user", "content": "把这张图裂变 4 张" }],
            "tools": [{ "type": "function", "function": { "name": "fission", "parameters": {} } }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let choice = &body["choices"][0];
    assert_eq!(choice["finish_reason"], "tool_calls");
    assert_eq!(
        choice["message"]["tool_calls"][0]["function"]["name"],
        "fission"
    );
}

#[tokio::test]
async fn injects_server_errors() {
    let (server, client) = start(MockConfig {
        error_500: 1.0,
        ..MockConfig::default()
    })
    .await;
    let response = client
        .post(format!("{}/images/generations", server.base_url()))
        .json(&json!({ "prompt": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 500);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "server_error");
    assert_eq!(server.stats().images_generated, 0);
}

#[tokio::test]
async fn injects_rate_limits_with_retry_after() {
    let (server, client) = start(MockConfig {
        error_429: 1.0,
        ..MockConfig::default()
    })
    .await;
    let response = client
        .post(format!("{}/images/generations", server.base_url()))
        .json(&json!({ "prompt": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 429);
    assert_eq!(response.headers()["retry-after"], "1");
}

#[tokio::test]
async fn enforces_per_minute_limit() {
    let (server, client) = start(MockConfig {
        rate_limit_per_min: Some(2),
        ..MockConfig::default()
    })
    .await;
    let mut statuses = Vec::new();
    for _ in 0..3 {
        let response = client
            .post(format!("{}/images/generations", server.base_url()))
            .json(&json!({ "prompt": "x" }))
            .send()
            .await
            .unwrap();
        statuses.push(response.status().as_u16());
    }
    assert_eq!(statuses, vec![200, 200, 429]);
}

#[tokio::test]
async fn injects_content_rejection() {
    let (server, client) = start(MockConfig {
        content_rejected: 1.0,
        ..MockConfig::default()
    })
    .await;
    let response = client
        .post(format!("{}/images/edits", server.base_url()))
        .multipart(edit_form("x"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "moderation_blocked");
}

#[tokio::test]
async fn injects_malformed_json() {
    let (server, client) = start(MockConfig {
        bad_json: 1.0,
        ..MockConfig::default()
    })
    .await;
    let response = client
        .post(format!("{}/images/generations", server.base_url()))
        .json(&json!({ "prompt": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.json::<Value>().await.is_err());
}

#[tokio::test]
async fn hangs_long_enough_for_clients_to_time_out() {
    let (server, _) = start(MockConfig {
        timeout: 1.0,
        hang: Duration::from_secs(5),
        ..MockConfig::default()
    })
    .await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(300))
        .build()
        .unwrap();
    let error = client
        .post(format!("{}/images/generations", server.base_url()))
        .json(&json!({ "prompt": "x" }))
        .send()
        .await
        .unwrap_err();
    assert!(error.is_timeout());
}

#[tokio::test]
async fn applies_latency() {
    let (server, client) = start(MockConfig {
        latency_ms: (150, 150),
        ..MockConfig::default()
    })
    .await;
    let started = Instant::now();
    client
        .post(format!("{}/images/generations", server.base_url()))
        .json(&json!({ "prompt": "x" }))
        .send()
        .await
        .unwrap();
    assert!(started.elapsed() >= Duration::from_millis(150));
}

#[tokio::test]
async fn async_tasks_complete_after_latency_and_count_once() {
    let (server, client) = start(MockConfig {
        latency_ms: (200, 200),
        ..MockConfig::default()
    })
    .await;
    let submitted: Value = client
        .post(format!("{}/async/images", server.base_url()))
        .header(JOB_ID_HEADER, "job-async")
        .json(&json!({ "prompt": "异步", "n": 1 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let task_id = submitted["data"][0]["task_id"].as_str().unwrap().to_owned();
    let poll_url = format!("{}/async/tasks/{task_id}", server.base_url());

    let first: Value = client
        .get(&poll_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(first["data"]["status"], "processing");

    tokio::time::sleep(Duration::from_millis(250)).await;
    for _ in 0..2 {
        let done: Value = client
            .get(&poll_url)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(done["data"]["status"], "completed");
        let b64 = done["data"]["result"]["images"][0]["b64_json"]
            .as_str()
            .unwrap();
        assert!(decode_png(b64).starts_with(PNG_SIGNATURE));
    }
    let stats = server.stats();
    assert_eq!(stats.job_successes["job-async"], 1);
    assert_eq!(stats.duplicate_jobs, 0);
}

#[tokio::test]
async fn counts_duplicate_generations_per_job_id() {
    let (server, client) = start(MockConfig::default()).await;
    for _ in 0..2 {
        client
            .post(format!("{}/images/generations", server.base_url()))
            .header(JOB_ID_HEADER, "job-1")
            .json(&json!({ "prompt": "x" }))
            .send()
            .await
            .unwrap();
    }
    let stats: Value = client
        .get(format!("http://{}/stats", server.addr()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(stats["duplicate_jobs"], 1);
    assert_eq!(stats["responses_by_status"]["200"], 2);

    let reset = client
        .post(format!("http://{}/stats/reset", server.addr()))
        .send()
        .await
        .unwrap();
    assert_eq!(reset.status(), 204);
    assert_eq!(server.stats().requests_total, 0);
}

#[tokio::test]
async fn invalid_config_is_rejected_at_start() {
    let result = MockServer::start(MockConfig {
        error_500: 2.0,
        ..MockConfig::default()
    })
    .await;
    assert!(result.is_err());
}
