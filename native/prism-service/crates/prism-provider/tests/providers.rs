//! 适配层的集成测试：大部分请求打到模拟服务；需要检查"实际发了什么"的用例，
//! 打到测试里临时起的记录服务。

// 测试里的辅助函数出错就该直接失败，用 unwrap 最直接
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use mock_image_api::{MockConfig, MockServer};
use prism_core::UpstreamErrorCode;
use prism_provider::{
    ApiKey, ChatMessage, ChatRequest, ImageInput, ImageProvider, ImageRequest, OpenAiChat,
    OpenAiImages, ProviderConfig, SubmitOutcome, TextProvider, ToolDefinition, list_models,
};
use serde_json::{Value, json};

const PNG_SIGNATURE: &[u8] = &[0x89, b'P', b'N', b'G'];

fn config(base_url: String) -> ProviderConfig {
    ProviderConfig::new(
        base_url,
        ApiKey::new("sk-test-key-123456"),
        "gpt-image-2.5-sunburst",
    )
}

fn reference() -> ImageInput {
    ImageInput {
        bytes: vec![1, 2, 3, 4],
        mime: "image/png".into(),
        file_name: "ref.png".into(),
    }
}

fn images(outcome: SubmitOutcome) -> Vec<prism_provider::GeneratedImage> {
    match outcome {
        SubmitOutcome::Done(images) => images,
        SubmitOutcome::Pending { .. } => panic!("同步接口不应该返回 Pending"),
    }
}

async fn mock(config: MockConfig) -> MockServer {
    MockServer::start(config).await.unwrap()
}

async fn submit_error(
    server: &MockServer,
    timeout: Option<Duration>,
) -> prism_provider::ProviderError {
    let mut cfg = config(server.base_url());
    if let Some(timeout) = timeout {
        cfg.timeout = timeout;
    }
    let provider = OpenAiImages::new(cfg).unwrap();
    provider.submit(&ImageRequest::new("x")).await.unwrap_err()
}

// ---------- 记录服务：保存收到的请求体，返回预设响应 ----------

#[derive(Clone, Default)]
struct Recorder {
    bodies: Arc<Mutex<Vec<Value>>>,
    base: Arc<Mutex<String>>,
}

async fn record_generation(
    State(recorder): State<Recorder>,
    body: axum::Json<Value>,
) -> axum::Json<Value> {
    recorder.bodies.lock().unwrap().push(body.0);
    let base = recorder.base.lock().unwrap().clone();
    axum::Json(json!({ "data": [{ "url": format!("{base}/files/result.png") }] }))
}

async fn png_file() -> ([(&'static str, &'static str); 1], Vec<u8>) {
    let mut bytes = PNG_SIGNATURE.to_vec();
    bytes.extend_from_slice(b"fake-png-body");
    ([("content-type", "image/png")], bytes)
}

async fn unauthorized() -> (StatusCode, axum::Json<Value>) {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(
            json!({ "error": { "message": "Incorrect API key provided", "code": "invalid_api_key" } }),
        ),
    )
}

/// 起一个记录服务，返回 (Base URL, 记录器)。
async fn recorder() -> (String, Recorder) {
    let recorder = Recorder::default();
    let app = Router::new()
        .route("/v1/images/generations", post(record_generation))
        .route("/files/result.png", get(png_file))
        .route("/v1/chat/completions", post(unauthorized))
        .with_state(recorder.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    *recorder.base.lock().unwrap() = format!("http://{addr}");
    (format!("http://{addr}/v1"), recorder)
}

// ---------- 生图 ----------

#[tokio::test]
async fn edits_with_reference_images_return_pngs() {
    let server = mock(MockConfig::default()).await;
    let provider = OpenAiImages::new(config(server.base_url())).unwrap();
    let mut request = ImageRequest::new("换成圣诞主题");
    request.references = vec![reference()];
    request.n = 2;

    let result = images(provider.submit(&request).await.unwrap());
    assert_eq!(result.len(), 2);
    assert!(result[0].bytes.starts_with(PNG_SIGNATURE));
    assert_eq!(
        result[0].revised_prompt.as_deref(),
        Some("[mock] 换成圣诞主题")
    );
    assert_eq!(server.stats().requests_by_path["/v1/images/edits"], 1);
}

#[tokio::test]
async fn requests_without_references_use_generations() {
    let server = mock(MockConfig::default()).await;
    let provider = OpenAiImages::new(config(server.base_url())).unwrap();
    let result = images(provider.submit(&ImageRequest::new("一只猫")).await.unwrap());
    assert_eq!(result.len(), 1);
    assert_eq!(server.stats().requests_by_path["/v1/images/generations"], 1);
}

#[tokio::test]
async fn job_header_is_sent_only_when_enabled() {
    let server = mock(MockConfig::default()).await;
    let mut request = ImageRequest::new("x");
    request.job_id = Some("job-42".into());

    let provider = OpenAiImages::new(config(server.base_url())).unwrap();
    provider.submit(&request).await.unwrap();
    assert!(server.stats().job_successes.is_empty());

    let mut cfg = config(server.base_url());
    cfg.send_job_header = true;
    OpenAiImages::new(cfg)
        .unwrap()
        .submit(&request)
        .await
        .unwrap();
    assert_eq!(server.stats().job_successes["job-42"], 1);
}

#[tokio::test]
async fn downloads_url_results_and_omits_unsupported_fields() {
    // UT-P03（url 形式）+ UT-P05（不支持的字段不发）
    let (base_url, recorder) = recorder().await;
    let mut cfg = config(base_url);
    cfg.capabilities.supports_n = false;
    cfg.capabilities.supports_background = false;
    let provider = OpenAiImages::new(cfg).unwrap();

    let mut request = ImageRequest::new("透明背景的猫");
    request.size = Some("1024x1024".into());
    request.background = Some("transparent".into());
    request.output_format = Some("png".into());
    let result = images(provider.submit(&request).await.unwrap());
    assert!(result[0].bytes.starts_with(PNG_SIGNATURE));

    let body = recorder.bodies.lock().unwrap()[0].clone();
    assert_eq!(body["size"], "1024x1024");
    assert_eq!(body["output_format"], "png");
    assert!(body.get("n").is_none());
    assert!(body.get("background").is_none());
}

#[tokio::test]
async fn too_many_references_are_rejected_before_sending() {
    let server = mock(MockConfig::default()).await;
    let mut cfg = config(server.base_url());
    cfg.capabilities.max_reference_images = 1;
    let provider = OpenAiImages::new(cfg).unwrap();
    let mut request = ImageRequest::new("x");
    request.references = vec![reference(), reference()];

    let error = provider.submit(&request).await.unwrap_err();
    assert_eq!(error.code, UpstreamErrorCode::BadRequest);
    assert_eq!(server.stats().requests_total, 0);
}

// ---------- 错误归类 ----------

#[tokio::test]
async fn classifies_server_errors() {
    let server = mock(MockConfig {
        error_500: 1.0,
        ..MockConfig::default()
    })
    .await;
    let error = submit_error(&server, None).await;
    assert_eq!(error.code, UpstreamErrorCode::UpstreamError);
    assert_eq!(error.status, Some(500));
}

#[tokio::test]
async fn classifies_rate_limits_with_retry_after() {
    let server = mock(MockConfig {
        error_429: 1.0,
        ..MockConfig::default()
    })
    .await;
    let error = submit_error(&server, None).await;
    assert_eq!(error.code, UpstreamErrorCode::RateLimited);
    assert_eq!(error.retry_after, Some(Duration::from_secs(1)));
}

#[tokio::test]
async fn classifies_content_rejection() {
    let server = mock(MockConfig {
        content_rejected: 1.0,
        ..MockConfig::default()
    })
    .await;
    assert_eq!(
        submit_error(&server, None).await.code,
        UpstreamErrorCode::ContentRejected
    );
}

#[tokio::test]
async fn classifies_malformed_responses() {
    let server = mock(MockConfig {
        bad_json: 1.0,
        ..MockConfig::default()
    })
    .await;
    assert_eq!(
        submit_error(&server, None).await.code,
        UpstreamErrorCode::BadResponse
    );
}

#[tokio::test]
async fn classifies_timeouts() {
    let server = mock(MockConfig {
        timeout: 1.0,
        hang: Duration::from_secs(5),
        ..MockConfig::default()
    })
    .await;
    let error = submit_error(&server, Some(Duration::from_millis(300))).await;
    assert_eq!(error.code, UpstreamErrorCode::Timeout);
}

#[tokio::test]
async fn classifies_connection_failures() {
    // 先拿到一个空闲端口再释放，保证没人监听
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let provider = OpenAiImages::new(config(format!("http://{addr}/v1"))).unwrap();
    let error = provider.submit(&ImageRequest::new("x")).await.unwrap_err();
    assert_eq!(error.code, UpstreamErrorCode::Network);
}

#[tokio::test]
async fn classifies_bad_keys() {
    let (base_url, _) = recorder().await;
    let chat = OpenAiChat::new(config(base_url)).unwrap();
    let error = chat
        .chat(&ChatRequest::new(vec![ChatMessage::user("x")]))
        .await
        .unwrap_err();
    assert_eq!(error.code, UpstreamErrorCode::Auth);
    assert!(error.message.contains("API Key"));
    assert!(!error.message.contains("sk-test-key"));
}

// ---------- 文本 ----------

#[tokio::test]
async fn chat_returns_plan_json_for_image_analysis() {
    let server = mock(MockConfig::default()).await;
    let chat = OpenAiChat::new(config(server.base_url())).unwrap();
    let mut request = ChatRequest::new(vec![
        ChatMessage::system("你是电商美工"),
        ChatMessage::user_with_images(
            "规划裂变 variant_count=3",
            vec![("image/png".into(), vec![1, 2, 3])],
        ),
    ]);
    request.json_output = true;

    let response = chat.chat(&request).await.unwrap();
    let plan: Value = serde_json::from_str(response.content.as_deref().unwrap()).unwrap();
    assert_eq!(plan["variants"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn chat_returns_tool_calls() {
    let server = mock(MockConfig::default()).await;
    let chat = OpenAiChat::new(config(server.base_url())).unwrap();
    let mut request = ChatRequest::new(vec![ChatMessage::user("裂变这张图")]);
    request.tools = vec![ToolDefinition {
        name: "fission".into(),
        description: "裂变图片".into(),
        parameters: json!({ "type": "object", "properties": {} }),
    }];

    let response = chat.chat(&request).await.unwrap();
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].name, "fission");
    assert_eq!(response.finish_reason.as_deref(), Some("tool_calls"));
}

/// 模拟某些推理模型：带函数工具却没把推理关掉时返回 400，说明里提到 reasoning_effort。
async fn reasoning_only_chat(
    State(bodies): State<Arc<Mutex<Vec<Value>>>>,
    axum::Json(body): axum::Json<Value>,
) -> (StatusCode, axum::Json<Value>) {
    bodies.lock().unwrap().push(body.clone());
    if body.get("tools").is_some() && body.get("reasoning_effort") != Some(&json!("none")) {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({ "error": {
                "message": "Function tools with reasoning_effort are not supported for this model in /v1/chat/completions. To use function tools, use /v1/responses or set reasoning_effort to 'none'.",
                "type": "invalid_request_error",
                "code": "unsupported_parameter"
            }})),
        );
    }
    (
        StatusCode::OK,
        axum::Json(json!({ "choices": [{
            "message": { "role": "assistant", "content": "好的" },
            "finish_reason": "stop"
        }]})),
    )
}

#[tokio::test]
async fn chat_turns_off_reasoning_when_model_rejects_tools_with_reasoning() {
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/v1/chat/completions", post(reasoning_only_chat))
        .with_state(bodies.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let chat = OpenAiChat::new(config(format!("http://{addr}/v1"))).unwrap();
    let mut request = ChatRequest::new(vec![ChatMessage::user("你好")]);
    request.tools = vec![ToolDefinition {
        name: "catalog__search_products".into(),
        description: "查商品".into(),
        parameters: json!({ "type": "object", "properties": {} }),
    }];

    // 第一次被拒后自动关掉推理重试，成功
    let response = chat.chat(&request).await.unwrap();
    assert_eq!(response.content.as_deref(), Some("好的"));
    // 之后带工具的请求直接关掉推理，不再先失败一次
    chat.chat(&request).await.unwrap();
    // 不带工具的请求不发这个参数，老模型不认它
    chat.chat(&ChatRequest::new(vec![ChatMessage::user("你好")]))
        .await
        .unwrap();

    let sent = bodies.lock().unwrap();
    let efforts: Vec<Option<&Value>> = sent.iter().map(|b| b.get("reasoning_effort")).collect();
    assert_eq!(
        efforts,
        vec![None, Some(&json!("none")), Some(&json!("none")), None]
    );
}

/// 模拟必须推理的模型：带工具时不关推理被拒，关掉推理也被拒。
async fn reasoning_required_chat(
    axum::Json(body): axum::Json<Value>,
) -> (StatusCode, axum::Json<Value>) {
    let message = if body.get("reasoning_effort").is_some() {
        "Unsupported value: 'reasoning_effort' does not support 'none' with this model."
    } else {
        "Function tools with reasoning_effort are not supported for this model in /v1/chat/completions."
    };
    (
        StatusCode::BAD_REQUEST,
        axum::Json(json!({ "error": { "message": message, "type": "invalid_request_error" } })),
    )
}

#[tokio::test]
async fn chat_explains_models_that_cannot_use_tools_on_chat_completions() {
    let app = Router::new().route("/v1/chat/completions", post(reasoning_required_chat));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let chat = OpenAiChat::new(config(format!("http://{addr}/v1"))).unwrap();
    let mut request = ChatRequest::new(vec![ChatMessage::user("你好")]);
    request.tools = vec![ToolDefinition {
        name: "ping".into(),
        description: "测试".into(),
        parameters: json!({ "type": "object", "properties": {} }),
    }];
    let error = chat.chat(&request).await.unwrap_err();
    assert_eq!(error.code, UpstreamErrorCode::BadRequest);
    assert!(
        error.message.contains("Agent 没法用它"),
        "{}",
        error.message
    );
}

// ---------- 模型列表 ----------

#[tokio::test]
async fn lists_models_with_default_first() {
    let server = mock(MockConfig::default()).await;
    let list = list_models(&config(server.base_url())).await.unwrap();
    assert!(list.supported);
    assert_eq!(list.default_image_model(), Some("gpt-image-2.5-sunburst"));
    assert!(list.other_models.contains(&"gpt-4.1".to_owned()));
}

#[tokio::test]
async fn missing_models_endpoint_means_unsupported() {
    // 记录服务没有 /v1/models，会返回 404
    let (base_url, _) = recorder().await;
    let list = list_models(&config(base_url)).await.unwrap();
    assert!(!list.supported);
    assert!(list.image_models.is_empty());
}

// ---------- 异步任务型 ----------

#[tokio::test]
async fn async_provider_submits_then_polls_until_done() {
    use prism_provider::{AsyncTaskProvider, AsyncTaskSpec, PollOutcome};

    let server = mock(MockConfig {
        latency_ms: (200, 200),
        ..MockConfig::default()
    })
    .await;
    let mut cfg = config(server.base_url());
    cfg.send_job_header = true;
    let provider = AsyncTaskProvider::new(cfg, AsyncTaskSpec::default()).unwrap();
    let mut request = ImageRequest::new("异步生成");
    request.references = vec![reference()];
    request.job_id = Some("job-async".into());

    let SubmitOutcome::Pending { remote_task_id } = provider.submit(&request).await.unwrap() else {
        panic!("异步接口应该返回 Pending");
    };
    assert_eq!(
        provider.poll(&remote_task_id).await.unwrap(),
        PollOutcome::Pending
    );

    tokio::time::sleep(Duration::from_millis(250)).await;
    let PollOutcome::Done(result) = provider.poll(&remote_task_id).await.unwrap() else {
        panic!("延迟过后应该完成");
    };
    assert!(result[0].bytes.starts_with(PNG_SIGNATURE));
    assert_eq!(server.stats().job_successes["job-async"], 1);

    let error = provider.poll("task_does_not_exist").await.unwrap_err();
    assert_eq!(error.status, Some(404));
}
