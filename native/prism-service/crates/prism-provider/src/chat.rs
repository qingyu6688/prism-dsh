//! 文本 / 看图模型：Chat Completions 接口。看图规划和 Agent 都用它。

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Map, Value, json};

use crate::http::{authorize, build_client, send_json};
use crate::{ProviderConfig, ProviderError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

/// 消息里的一段内容：文字或图片。
#[derive(Clone, PartialEq, Eq)]
pub enum ContentPart {
    Text(String),
    Image { mime: String, bytes: Vec<u8> },
}

impl fmt::Debug for ContentPart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => f.debug_tuple("Text").field(text).finish(),
            Self::Image { mime, bytes } => write!(f, "Image({mime}, {} 字节)", bytes.len()),
        }
    }
}

/// 模型请求调用的工具。`arguments` 是 JSON 字符串，由调用方解析校验。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// 一条对话消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: Vec<ContentPart>,
    /// 助手消息里的工具调用
    pub tool_calls: Vec<ToolCall>,
    /// 工具结果消息对应的调用 ID
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    fn new(role: Role, content: Vec<ContentPart>) -> Self {
        Self {
            role,
            content,
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self::new(Role::System, vec![ContentPart::Text(text.into())])
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self::new(Role::User, vec![ContentPart::Text(text.into())])
    }

    /// 带图片的用户消息，看图规划用。
    pub fn user_with_images(text: impl Into<String>, images: Vec<(String, Vec<u8>)>) -> Self {
        let mut content = vec![ContentPart::Text(text.into())];
        content.extend(
            images
                .into_iter()
                .map(|(mime, bytes)| ContentPart::Image { mime, bytes }),
        );
        Self::new(Role::User, content)
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self::new(Role::Assistant, vec![ContentPart::Text(text.into())])
    }

    /// 助手发起的工具调用，要原样放回对话历史。
    pub fn assistant_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            tool_calls,
            ..Self::new(Role::Assistant, Vec::new())
        }
    }

    /// 工具执行结果。
    pub fn tool_result(tool_call_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            tool_call_id: Some(tool_call_id.into()),
            ..Self::new(Role::Tool, vec![ContentPart::Text(text.into())])
        }
    }

    fn to_json(&self) -> Value {
        let mut message = Map::new();
        message.insert("role".into(), json!(self.role.as_str()));
        message.insert("content".into(), self.content_json());
        if !self.tool_calls.is_empty() {
            let calls: Vec<Value> = self
                .tool_calls
                .iter()
                .map(|call| {
                    json!({
                        "id": call.id,
                        "type": "function",
                        "function": { "name": call.name, "arguments": call.arguments }
                    })
                })
                .collect();
            message.insert("tool_calls".into(), Value::Array(calls));
        }
        if let Some(id) = &self.tool_call_id {
            message.insert("tool_call_id".into(), json!(id));
        }
        Value::Object(message)
    }

    /// 只有一段文字时发字符串，兼容性最好；有图片时发数组。
    fn content_json(&self) -> Value {
        match self.content.as_slice() {
            [] => Value::Null,
            [ContentPart::Text(text)] => json!(text),
            parts => Value::Array(
                parts
                    .iter()
                    .map(|part| match part {
                        ContentPart::Text(text) => json!({ "type": "text", "text": text }),
                        ContentPart::Image { mime, bytes } => json!({
                            "type": "image_url",
                            "image_url": { "url": format!("data:{mime};base64,{}", BASE64.encode(bytes)) }
                        }),
                    })
                    .collect(),
            ),
        }
    }
}

/// 给模型的工具定义。`parameters` 是 JSON Schema。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
    /// 要求模型输出 JSON。接口不支持时不发这个参数，靠提示词约束
    pub json_output: bool,
    pub temperature: Option<f32>,
    pub job_id: Option<String>,
}

impl ChatRequest {
    pub fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            messages,
            tools: Vec::new(),
            json_output: false,
            temperature: None,
            job_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
}

/// 文本供应商。
#[async_trait]
pub trait TextProvider: Send + Sync {
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError>;

    /// 应用可按会话选择文本配置；普通供应商沿用无上下文调用。
    async fn chat_for(
        &self,
        request: &ChatRequest,
        _conversation_id: &str,
    ) -> Result<ChatResponse, ProviderError> {
        self.chat(request).await
    }
}

/// OpenAI Chat Completions 接口，中转站兼容得最好。
#[derive(Debug)]
pub struct OpenAiChat {
    config: ProviderConfig,
    client: reqwest::Client,
    /// 带工具时要发 `reasoning_effort: "none"`。有的推理模型在 Chat Completions 里
    /// 不接受"推理 + 函数工具"，但老模型不认这个参数，所以不默认发，被拒一次后才打开
    tools_without_reasoning: AtomicBool,
}

/// 推理模型拒绝"推理 + 函数工具"时，上游说明里会提到这个参数名
const REASONING_PARAM: &str = "reasoning_effort";
const HTTP_BAD_REQUEST: u16 = 400;

impl OpenAiChat {
    pub fn new(config: ProviderConfig) -> Result<Self, ProviderError> {
        let client = build_client(&config)?;
        Ok(Self {
            config,
            client,
            tools_without_reasoning: AtomicBool::new(false),
        })
    }

    async fn send(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        let http = self
            .client
            .post(self.config.endpoint("chat/completions"))
            .json(&self.body(request));
        let body = send_json(
            authorize(http, &self.config, request.job_id.as_deref()),
            "chat/completions",
        )
        .await?;
        parse_chat_response(&body)
    }

    fn body(&self, request: &ChatRequest) -> Value {
        let mut body = Map::new();
        body.insert("model".into(), json!(self.config.model));
        body.insert(
            "messages".into(),
            Value::Array(request.messages.iter().map(ChatMessage::to_json).collect()),
        );
        if !request.tools.is_empty() {
            let tools: Vec<Value> = request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters
                        }
                    })
                })
                .collect();
            body.insert("tools".into(), Value::Array(tools));
            if self.tools_without_reasoning.load(Ordering::Relaxed) {
                body.insert(REASONING_PARAM.into(), json!("none"));
            }
        }
        if request.json_output && self.config.capabilities.supports_json_mode {
            body.insert("response_format".into(), json!({ "type": "json_object" }));
        }
        if let Some(temperature) = request.temperature {
            body.insert("temperature".into(), json!(temperature));
        }
        Value::Object(body)
    }
}

#[async_trait]
impl TextProvider for OpenAiChat {
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        match self.send(request).await {
            // 400 不计费，关掉推理后重试一次；之后这个实例带工具的请求都直接关掉
            Err(error)
                if error.status == Some(HTTP_BAD_REQUEST)
                    && error.message.contains(REASONING_PARAM)
                    && !request.tools.is_empty()
                    && !self.tools_without_reasoning.swap(true, Ordering::Relaxed) =>
            {
                tracing::info!(model = %self.config.model, "模型不支持带工具推理，改为不推理后重试");
                self.send(request).await.map_err(|retry| {
                    // 关掉推理也被拒：这个模型在 Chat Completions 里没法带工具
                    if retry.status == Some(HTTP_BAD_REQUEST) && retry.message.contains(REASONING_PARAM) {
                        ProviderError {
                            message: format!(
                                "模型 {} 在 Chat Completions 接口里带工具时要求关掉推理，但它又不支持关掉推理，Agent 没法用它。请换一个模型（以后支持 Responses 接口后可以再用）",
                                self.config.model
                            ),
                            ..retry
                        }
                    } else {
                        retry
                    }
                })
            }
            outcome => outcome,
        }
    }
}

fn parse_chat_response(body: &Value) -> Result<ChatResponse, ProviderError> {
    let choice = body
        .pointer("/choices/0")
        .ok_or_else(|| ProviderError::bad_response("缺少 choices"))?;
    let message = choice
        .get("message")
        .ok_or_else(|| ProviderError::bad_response("缺少 message"))?;
    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    Some(ToolCall {
                        id: call.get("id")?.as_str()?.to_owned(),
                        name: call.pointer("/function/name")?.as_str()?.to_owned(),
                        arguments: call
                            .pointer("/function/arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}")
                            .to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ChatResponse {
        content: message
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_owned),
        tool_calls,
        finish_reason: choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ApiKey;

    fn chat(json_mode: bool) -> OpenAiChat {
        let mut config = ProviderConfig::new("http://x/v1", ApiKey::new("k"), "gpt-4.1");
        config.capabilities.supports_json_mode = json_mode;
        OpenAiChat::new(config).unwrap()
    }

    #[test]
    fn single_text_content_is_sent_as_string() {
        let json = ChatMessage::user("你好").to_json();
        assert_eq!(json["content"], "你好");
    }

    #[test]
    fn images_are_sent_as_data_urls() {
        let message =
            ChatMessage::user_with_images("看图", vec![("image/png".into(), vec![1, 2, 3])]);
        let json = message.to_json();
        assert_eq!(json["content"][1]["type"], "image_url");
        assert_eq!(
            json["content"][1]["image_url"]["url"],
            "data:image/png;base64,AQID"
        );
    }

    #[test]
    fn tool_messages_carry_ids() {
        let call = ToolCall {
            id: "call_1".into(),
            name: "fission".into(),
            arguments: "{}".into(),
        };
        let assistant = ChatMessage::assistant_tool_calls(vec![call]).to_json();
        assert!(assistant["content"].is_null());
        assert_eq!(assistant["tool_calls"][0]["function"]["name"], "fission");
        let result = ChatMessage::tool_result("call_1", "已开始").to_json();
        assert_eq!(result["tool_call_id"], "call_1");
        assert_eq!(result["role"], "tool");
    }

    #[test]
    fn json_mode_is_skipped_when_unsupported() {
        // UT-P05：不支持的参数不发
        let mut request = ChatRequest::new(vec![ChatMessage::user("x")]);
        request.json_output = true;
        assert!(chat(true).body(&request).get("response_format").is_some());
        assert!(chat(false).body(&request).get("response_format").is_none());
    }

    #[test]
    fn debug_output_hides_image_bytes() {
        let message =
            ChatMessage::user_with_images("看图", vec![("image/png".into(), vec![9; 500])]);
        let debug = format!("{message:?}");
        assert!(debug.contains("500 字节"));
        assert!(!debug.contains("9, 9"));
    }

    #[test]
    fn parses_tool_calls_and_content() {
        let body = json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": null,
                    "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "fission", "arguments": "{\"count\":4}" } }]
                }
            }]
        });
        let response = parse_chat_response(&body).unwrap();
        assert_eq!(response.content, None);
        assert_eq!(response.tool_calls[0].arguments, "{\"count\":4}");
        assert_eq!(response.finish_reason.as_deref(), Some("tool_calls"));
        assert!(parse_chat_response(&json!({})).is_err());
    }
}
