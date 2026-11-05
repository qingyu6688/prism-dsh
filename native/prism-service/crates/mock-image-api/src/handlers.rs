//! 各接口的处理函数。响应格式尽量和 OpenAI 一致，接口列表见 `docs/api.md` 第 6 节。

use std::io::Cursor;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use image::{ImageFormat, Rgb, RgbImage};
use serde_json::{Value, json};

use crate::state::{AppState, AsyncTask, Outcome, lock};

/// 压测时用来统计"同一个任务是否被重复生成"的请求头。
pub const JOB_ID_HEADER: &str = "x-prism-job-id";
/// 规划请求里用这个标记告诉模拟服务要返回几条变化指令，没有时默认 4 条。
const VARIANT_COUNT_MARKER: &str = "variant_count=";
const DEFAULT_VARIANT_COUNT: usize = 4;
const AXES: [&str; 4] = ["element", "composition", "palette", "theme"];

type Shared = State<Arc<AppState>>;

pub async fn health() -> &'static str {
    "ok"
}

pub async fn models(State(state): Shared) -> Response {
    state.record_request("/v1/models");
    let ids = [
        "gpt-image-2.5-sunburst",
        "gpt-image-2.5-flare",
        "gpt-image-2",
        "gpt-image-1",
        "gpt-4.1",
        "gpt-4.1-mini",
        "text-embedding-3-large",
    ];
    let data: Vec<Value> = ids
        .iter()
        .map(|id| json!({ "id": id, "object": "model", "owned_by": "mock" }))
        .collect();
    finish(
        &state,
        Json(json!({ "object": "list", "data": data })).into_response(),
    )
}

/// `POST /v1/images/generations`，JSON 请求体。
pub async fn generations(
    State(state): Shared,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    state.record_request("/v1/images/generations");
    let prompt = body
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let n = body.get("n").and_then(Value::as_u64).unwrap_or(1);
    image_response(&state, &headers, &prompt, n).await
}

/// `POST /v1/images/edits`，multipart 请求体（参考图 + 参数）。
pub async fn edits(State(state): Shared, headers: HeaderMap, mut multipart: Multipart) -> Response {
    state.record_request("/v1/images/edits");
    let mut prompt = None;
    let mut n = 1;
    let mut images = 0;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => {
                return finish(&state, bad_request(&format!("multipart 解析失败：{error}")));
            }
        };
        let name = field.name().unwrap_or_default().to_owned();
        let bytes = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => {
                return finish(
                    &state,
                    bad_request(&format!("读取字段 {name} 失败：{error}")),
                );
            }
        };
        match name.as_str() {
            "prompt" => prompt = Some(String::from_utf8_lossy(&bytes).into_owned()),
            "n" => n = String::from_utf8_lossy(&bytes).trim().parse().unwrap_or(1),
            "image" | "image[]" => images += 1,
            _ => {}
        }
    }
    let Some(prompt) = prompt else {
        return finish(&state, bad_request("缺少 prompt 字段"));
    };
    if images == 0 {
        return finish(&state, bad_request("缺少参考图 image 字段"));
    }
    image_response(&state, &headers, &prompt, n).await
}

/// `POST /v1/chat/completions`。按请求内容返回几种回复：
/// - 工具里有 `plan_steps`、最后一条用户消息里有 ```json 代码块：用代码块当参数调用 `plan_steps`
///   （Agent 验收测试用，测试在消息里写好要提交的计划）
/// - 带 `tools` 且最后一条是用户消息：调用第一个工具（Agent 测试用）
/// - 要求 JSON 输出或带 `variant_count=` 标记：返回规划结果
/// - 其他：一句普通回复
pub async fn chat_completions(State(state): Shared, Json(body): Json<Value>) -> Response {
    state.record_request("/v1/chat/completions");
    let outcome = state.decide();
    if outcome != Outcome::Success {
        return finish(&state, failure(&state, outcome).await);
    }
    tokio::time::sleep(state.latency()).await;

    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("mock-model");
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let last_role = messages
        .last()
        .and_then(|m| m.get("role"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let first_tool = body
        .get("tools")
        .and_then(Value::as_array)
        .and_then(|tools| tools.first())
        .and_then(|tool| tool.pointer("/function/name"))
        .and_then(Value::as_str);
    let text = message_text(&messages);
    let wants_json = body
        .pointer("/response_format/type")
        .and_then(Value::as_str)
        == Some("json_object");

    let has_plan_tool = body
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| {
            tools.iter().any(|tool| {
                tool.pointer("/function/name").and_then(Value::as_str) == Some("plan_steps")
            })
        });
    let scripted_plan = (has_plan_tool && last_role == "user")
        .then(|| {
            messages
                .last()
                .map(|m| message_text(std::slice::from_ref(m)))
        })
        .flatten()
        .and_then(|last| fenced_json(&last));

    let (message, finish_reason) = match (first_tool, last_role) {
        _ if scripted_plan.is_some() => (
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": format!("call_{}", uuid::Uuid::now_v7().simple()),
                    "type": "function",
                    "function": { "name": "plan_steps", "arguments": scripted_plan.unwrap_or_default() }
                }]
            }),
            "tool_calls",
        ),
        (Some(tool), "user") => (
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": format!("call_{}", uuid::Uuid::now_v7().simple()),
                    "type": "function",
                    "function": { "name": tool, "arguments": "{}" }
                }]
            }),
            "tool_calls",
        ),
        _ if wants_json || text.contains(VARIANT_COUNT_MARKER) => (
            json!({ "role": "assistant", "content": plan_json(variant_count(&text)).to_string() }),
            "stop",
        ),
        _ => (
            json!({ "role": "assistant", "content": "模拟回复" }),
            "stop",
        ),
    };

    let response = json!({
        "id": format!("chatcmpl-{}", uuid::Uuid::now_v7().simple()),
        "object": "chat.completion",
        "created": unix_seconds(),
        "model": model,
        "choices": [{ "index": 0, "message": message, "finish_reason": finish_reason }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 10, "total_tokens": 20 }
    });
    finish(&state, Json(response).into_response())
}

/// `POST /v1/async/images`：提交异步生图任务，立刻返回任务 ID。
pub async fn async_submit(
    State(state): Shared,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    state.record_request("/v1/async/images");
    let outcome = state.decide();
    if outcome != Outcome::Success {
        return finish(&state, failure(&state, outcome).await);
    }
    let task_id = format!("task_{}", uuid::Uuid::now_v7().simple());
    let task = AsyncTask {
        ready_at: std::time::Instant::now() + state.latency(),
        images: body
            .get("n")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(1),
        prompt: body
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        job_id: job_id(&headers),
        counted: false,
    };
    lock(&state.tasks).insert(task_id.clone(), task);
    finish(
        &state,
        Json(json!({ "data": [{ "task_id": task_id }] })).into_response(),
    )
}

/// `GET /v1/async/tasks/{id}`：查询异步任务。完成后可以反复查询，只在第一次完成时计数。
pub async fn async_poll(State(state): Shared, Path(task_id): Path<String>) -> Response {
    state.record_request("/v1/async/tasks");
    let task = {
        let mut tasks = lock(&state.tasks);
        let Some(task) = tasks.get_mut(&task_id) else {
            return finish(
                &state,
                error_body(StatusCode::NOT_FOUND, "任务不存在", "not_found", None),
            );
        };
        if task.ready_at > std::time::Instant::now() {
            return finish(
                &state,
                Json(json!({ "data": { "status": "processing" } })).into_response(),
            );
        }
        let first_time = !task.counted;
        task.counted = true;
        (task.clone(), first_time)
    };
    let (task, first_time) = task;
    let images = match render_images(&state, task.images) {
        Ok(images) => images,
        Err(error) => return finish(&state, render_failed(&error)),
    };
    if first_time {
        state.record_generation(task.job_id.as_deref(), task.images);
    }
    let result: Vec<Value> = images
        .into_iter()
        .map(|b64| json!({ "b64_json": b64 }))
        .collect();
    let body = json!({
        "data": {
            "status": "completed",
            "result": { "images": result, "revised_prompt": format!("[mock] {}", task.prompt) }
        }
    });
    finish(&state, Json(body).into_response())
}

pub async fn stats(State(state): Shared) -> Response {
    Json(state.stats()).into_response()
}

pub async fn reset_stats(State(state): Shared) -> StatusCode {
    state.reset_stats();
    StatusCode::NO_CONTENT
}

/// 生图接口的公共部分：错误注入 → 延迟 → 返回 n 张纯色图。
async fn image_response(state: &AppState, headers: &HeaderMap, prompt: &str, n: u64) -> Response {
    let outcome = state.decide();
    if outcome != Outcome::Success {
        return finish(state, failure(state, outcome).await);
    }
    tokio::time::sleep(state.latency()).await;

    let count = u32::try_from(n.clamp(1, 10)).unwrap_or(1);
    let images = match render_images(state, count) {
        Ok(images) => images,
        Err(error) => return finish(state, render_failed(&error)),
    };
    state.record_generation(job_id(headers).as_deref(), count);
    let data: Vec<Value> = images
        .into_iter()
        .map(|b64| json!({ "b64_json": b64, "revised_prompt": format!("[mock] {prompt}") }))
        .collect();
    finish(
        state,
        Json(json!({ "created": unix_seconds(), "data": data })).into_response(),
    )
}

/// 按注入的错误类型构造响应。
async fn failure(state: &AppState, outcome: Outcome) -> Response {
    match outcome {
        Outcome::Success => StatusCode::OK.into_response(),
        Outcome::RateLimited { retry_after_secs } => {
            let mut response = error_body(
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit reached for requests",
                "rate_limit_exceeded",
                Some("requests"),
            );
            let headers = response.headers_mut();
            headers.insert(header::RETRY_AFTER, HeaderValue::from(retry_after_secs));
            headers.insert(
                "x-ratelimit-remaining-requests",
                HeaderValue::from_static("0"),
            );
            response
        }
        Outcome::ServerError => error_body(
            StatusCode::INTERNAL_SERVER_ERROR,
            "The server had an error while processing your request.",
            "server_error",
            Some("server_error"),
        ),
        Outcome::Hang => {
            tokio::time::sleep(state.config.hang).await;
            StatusCode::GATEWAY_TIMEOUT.into_response()
        }
        Outcome::BadJson => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            "{\"data\": [{\"b64_json\": ",
        )
            .into_response(),
        Outcome::ContentRejected => error_body(
            StatusCode::BAD_REQUEST,
            "Your request was rejected as a result of our safety system.",
            "moderation_blocked",
            Some("image_generation_user_error"),
        ),
    }
}

fn error_body(status: StatusCode, message: &str, code: &str, kind: Option<&str>) -> Response {
    let body = json!({ "error": { "message": message, "type": kind, "code": code } });
    (status, Json(body)).into_response()
}

fn bad_request(message: &str) -> Response {
    error_body(
        StatusCode::BAD_REQUEST,
        message,
        "invalid_request",
        Some("invalid_request_error"),
    )
}

fn finish(state: &AppState, response: Response) -> Response {
    state.record_status(response.status().as_u16());
    response
}

/// 生成 count 张纯色 PNG，返回 base64。颜色随机，方便肉眼区分。
fn render_images(state: &AppState, count: u32) -> Result<Vec<String>, image::ImageError> {
    let size = state.config.image_size;
    (0..count)
        .map(|_| {
            let [r, g, b, ..] = state.next_seed().to_le_bytes();
            let image = RgbImage::from_pixel(size, size, Rgb([r, g, b]));
            let mut buffer = Cursor::new(Vec::new());
            image.write_to(&mut buffer, ImageFormat::Png)?;
            Ok(BASE64.encode(buffer.into_inner()))
        })
        .collect()
}

fn render_failed(error: &image::ImageError) -> Response {
    error_body(
        StatusCode::INTERNAL_SERVER_ERROR,
        &format!("生成模拟图片失败：{error}"),
        "mock_render_failed",
        None,
    )
}

fn job_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get(JOB_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

/// 把所有消息里的文字拼起来；内容可能是字符串，也可能是 `[{type: text}, {type: image_url}]` 数组。
fn message_text(messages: &[Value]) -> String {
    let mut text = String::new();
    for message in messages {
        match message.get("content") {
            Some(Value::String(content)) => text.push_str(content),
            Some(Value::Array(parts)) => {
                for part in parts {
                    if let Some(part_text) = part.get("text").and_then(Value::as_str) {
                        text.push_str(part_text);
                    }
                }
            }
            _ => {}
        }
        text.push('\n');
    }
    text
}

/// 文字里第一个 ```json 代码块的内容。
fn fenced_json(text: &str) -> Option<String> {
    let start = text.find("```json")? + "```json".len();
    let end = text[start..].find("```")? + start;
    Some(text[start..end].trim().to_owned())
}

fn variant_count(text: &str) -> usize {
    text.find(VARIANT_COUNT_MARKER)
        .map(|start| &text[start + VARIANT_COUNT_MARKER.len()..])
        .and_then(|rest| {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
        .unwrap_or(DEFAULT_VARIANT_COUNT)
}

fn plan_json(count: usize) -> Value {
    let variants: Vec<Value> = (0..count)
        .map(|i| {
            json!({
                "axis": AXES[i % AXES.len()],
                "instruction": format!("模拟指令 {}", i + 1)
            })
        })
        .collect();
    json!({
        "analysis": {
            "subject": "模拟主体",
            "style": "扁平插画",
            "palette": ["#F4A261", "#264653"],
            "composition": "主体居中",
            "text": "",
            "background": "纯白"
        },
        "variants": variants
    })
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_variant_count_marker() {
        assert_eq!(variant_count("请规划 variant_count=7 条"), 7);
        assert_eq!(variant_count("没有标记"), DEFAULT_VARIANT_COUNT);
    }

    #[test]
    fn fenced_json_is_extracted() {
        assert_eq!(
            fenced_json(
                "提交这个计划
```json
{\"steps\":[]}
```
谢谢"
            )
            .as_deref(),
            Some("{\"steps\":[]}")
        );
        assert_eq!(fenced_json("没有代码块"), None);
    }

    #[test]
    fn plan_has_requested_number_of_variants() {
        let plan = plan_json(6);
        assert_eq!(plan["variants"].as_array().map(Vec::len), Some(6));
        assert_eq!(plan["variants"][4]["axis"], "element");
    }

    #[test]
    fn collects_text_from_multimodal_messages() {
        let messages = vec![json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "看这张图 variant_count=3" },
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } }
            ]
        })];
        assert!(message_text(&messages).contains("variant_count=3"));
    }
}
