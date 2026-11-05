//! JSON-RPC 2.0 消息的解析和序列化，规则见 `docs/prism/service-api.md` 第 2 节。

use prism_core::{ApiError, ApiErrorCode};
use serde::Serialize;
use serde_json::{Map, Value, json};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
/// 业务错误，`data.code` 是 [`ApiErrorCode`]。
pub const BUSINESS_ERROR: i64 = -32001;

/// 响应里的 `error` 对象。`message` 会展示给用户，写成中文说明。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn parse_error() -> Self {
        Self::new(PARSE_ERROR, "消息不是合法的 JSON")
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(INVALID_REQUEST, message)
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::new(METHOD_NOT_FOUND, format!("没有这个方法：{method}"))
    }

    /// 参数不对。`data` 和业务错误保持同样的形状，Host 侧可以统一处理。
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            data: Some(json!({ "code": ApiErrorCode::InvalidInput, "detail": null })),
            ..Self::new(INVALID_PARAMS, message)
        }
    }
}

impl From<ApiError> for RpcError {
    fn from(error: ApiError) -> Self {
        let code = if error.code == ApiErrorCode::Internal {
            INTERNAL_ERROR
        } else {
            BUSINESS_ERROR
        };
        Self {
            code,
            message: error.message,
            data: Some(json!({ "code": error.code, "detail": error.detail })),
        }
    }
}

/// 一条合法的请求。`id` 为 `None` 时是通知，处理后不回响应。
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub id: Option<Value>,
    pub method: String,
    pub params: Value,
}

/// 解析一行消息的结果。
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Request(Request),
    /// 消息有问题，直接回这个错误；拿不到合法 id 时 `id` 是 `null`。
    Invalid {
        id: Value,
        error: RpcError,
    },
}

/// 解析一行消息（不含结尾的换行）。
pub fn parse(line: &[u8]) -> Incoming {
    let value: Value = match serde_json::from_slice(line) {
        Ok(value) => value,
        Err(_) => return invalid(Value::Null, RpcError::parse_error()),
    };
    let Value::Object(mut message) = value else {
        return invalid(
            Value::Null,
            RpcError::invalid_request("请求必须是 JSON 对象，不支持批量请求"),
        );
    };

    let id = match message.remove("id") {
        None => None,
        Some(id @ (Value::Number(_) | Value::String(_) | Value::Null)) => Some(id),
        Some(_) => {
            return invalid(
                Value::Null,
                RpcError::invalid_request("id 只能是数字或字符串"),
            );
        }
    };
    let reply_id = id.clone().unwrap_or(Value::Null);

    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return invalid(
            reply_id,
            RpcError::invalid_request("jsonrpc 字段必须是 \"2.0\""),
        );
    }
    let method = match message.remove("method") {
        Some(Value::String(method)) => method,
        _ => return invalid(reply_id, RpcError::invalid_request("缺少 method 字段")),
    };
    // 只支持按名称传参；没有 params 或传 null 都当作空对象
    let params = match message.remove("params") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(params @ Value::Object(_)) => params,
        Some(_) => return invalid(reply_id, RpcError::invalid_params("params 必须是对象")),
    };

    Incoming::Request(Request { id, method, params })
}

fn invalid(id: Value, error: RpcError) -> Incoming {
    Incoming::Invalid { id, error }
}

/// 成功响应，一行，不带结尾换行。
pub fn success(id: &Value, result: &Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

/// 失败响应，一行，不带结尾换行。
pub fn failure(id: &Value, error: &RpcError) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": error }).to_string()
}

/// 服务主动发给 Host 的通知，一行，不带结尾换行。
pub fn notification(method: &str, params: &Value) -> String {
    json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(line: &str) -> Request {
        match parse(line.as_bytes()) {
            Incoming::Request(request) => request,
            Incoming::Invalid { error, .. } => panic!("应该是合法请求：{error:?}"),
        }
    }

    fn rejected(line: &str) -> (Value, i64) {
        match parse(line.as_bytes()) {
            Incoming::Invalid { id, error } => (id, error.code),
            Incoming::Request(request) => panic!("应该被拒绝：{request:?}"),
        }
    }

    #[test]
    fn parses_request_and_defaults_params_to_empty_object() {
        let r = request(r#"{"jsonrpc":"2.0","id":7,"method":"service.ping"}"#);
        assert_eq!(r.id, Some(json!(7)));
        assert_eq!(r.method, "service.ping");
        assert_eq!(r.params, json!({}));

        let r = request(r#"{"jsonrpc":"2.0","id":"a","method":"m","params":null}"#);
        assert_eq!(r.id, Some(json!("a")));
        assert_eq!(r.params, json!({}));
    }

    #[test]
    fn request_without_id_is_notification() {
        let r = request(r#"{"jsonrpc":"2.0","method":"m","params":{"x":1}}"#);
        assert_eq!(r.id, None);
        assert_eq!(r.params, json!({"x": 1}));
    }

    #[test]
    fn rejects_bad_messages_with_right_codes() {
        assert_eq!(rejected("not json"), (Value::Null, PARSE_ERROR));
        assert_eq!(rejected("[1,2]"), (Value::Null, INVALID_REQUEST));
        assert_eq!(
            rejected(r#"{"jsonrpc":"2.0","id":{},"method":"m"}"#),
            (Value::Null, INVALID_REQUEST)
        );
        assert_eq!(
            rejected(r#"{"jsonrpc":"1.0","id":1,"method":"m"}"#),
            (json!(1), INVALID_REQUEST)
        );
        assert_eq!(
            rejected(r#"{"jsonrpc":"2.0","id":2}"#),
            (json!(2), INVALID_REQUEST)
        );
        assert_eq!(
            rejected(r#"{"jsonrpc":"2.0","id":3,"method":"m","params":[1]}"#),
            (json!(3), INVALID_PARAMS)
        );
    }

    #[test]
    fn business_and_internal_errors_map_to_codes() {
        let e = RpcError::from(ApiError::new(ApiErrorCode::NotFound, "商品不存在"));
        assert_eq!(e.code, BUSINESS_ERROR);
        assert_eq!(e.message, "商品不存在");
        assert_eq!(e.data, Some(json!({"code": "not_found", "detail": null})));

        let e = RpcError::from(ApiError::internal("出错了"));
        assert_eq!(e.code, INTERNAL_ERROR);
    }

    #[test]
    fn serialized_messages_are_single_line() {
        let line = success(&json!(1), &json!({"text": "第一行\n第二行"}));
        assert!(!line.contains('\n'));
        let line = failure(&Value::Null, &RpcError::parse_error());
        assert_eq!(
            serde_json::from_str::<Value>(&line).ok(),
            Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"消息不是合法的 JSON"}})
            )
        );
        let line = notification("image.batchProgress", &json!({"total": 1}));
        assert!(!line.contains("\"id\""));
    }
}
