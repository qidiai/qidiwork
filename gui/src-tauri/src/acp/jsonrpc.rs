//! 最小 JSON-RPC 2.0 帧层(ACP 运行在 newline-delimited JSON-RPC 上)。
//!
//! 只实现桥需要的面:构造请求/通知/应答,解析入站三态(请求/通知/应答)。
//! 不引入完整 JSON-RPC 库——协议面仅 5 个方法(k3 审计可对照
//! agent-client-protocol 规范)。

use serde_json::{Value, json};

/// ACP 协议版本(agent-client-protocol V1)。
pub const ACP_PROTOCOL_VERSION: u32 = 1;
pub const ERR_METHOD_NOT_FOUND: i64 = -32601;

/// 构造出站请求(id 由桥分配)。
pub fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}

/// 构造出站通知。
pub fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","method":method,"params":params})
}

/// 构造成功应答。
pub fn success_response(id: u64, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

/// 构造错误应答。
pub fn error_response(id: u64, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

/// 入站消息三态。
#[derive(Debug, Clone)]
pub enum Incoming {
    /// agent 发来的请求(需要我方应答)。
    Request {
        id: u64,
        method: String,
        params: Value,
    },
    /// agent 发来的通知。
    Notification { method: String, params: Value },
    /// agent 对我方请求的应答。Err = (code, message)。
    Response {
        id: u64,
        result: Result<Value, (i64, String)>,
    },
}

/// 解析一行 JSON-RPC;不符合任何已知形态时返回 None(记日志后忽略,
/// 不使路由循环崩溃)。
pub fn parse_incoming(line: &str) -> Option<Incoming> {
    let v: Value = serde_json::from_str(line).ok()?;
    let id = v.get("id").cloned();
    let method = v.get("method").and_then(|m| m.as_str()).map(String::from);

    match (id, method) {
        (Some(id), Some(method)) => {
            let id = id.as_u64()?;
            Some(Incoming::Request {
                id,
                method,
                params: v.get("params").cloned().unwrap_or(Value::Null),
            })
        }
        (None, Some(method)) => Some(Incoming::Notification {
            method,
            params: v.get("params").cloned().unwrap_or(Value::Null),
        }),
        (Some(id), None) => {
            let id = id.as_u64()?;
            let result = match v.get("error") {
                Some(err) => Err((
                    err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1),
                    err.get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                )),
                None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
            };
            Some(Incoming::Response { id, result })
        }
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn request_roundtrip_shape() {
        let msg = request(1, "initialize", json!({"protocolVersion":1}));
        let s = msg.to_string();
        assert!(s.contains("\"id\":1") && s.contains("\"method\":\"initialize\""));
    }

    #[test]
    fn parse_all_three_shapes() {
        let req = parse_incoming(
            r#"{"jsonrpc":"2.0","id":7,"method":"session/request_permission","params":{"a":1}}"#,
        )
        .unwrap();
        assert!(matches!(req, Incoming::Request { id: 7, .. }));

        let note = parse_incoming(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s"}}"#,
        )
        .unwrap();
        assert!(matches!(note, Incoming::Notification { .. }));

        let ok = parse_incoming(r#"{"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}"#)
            .unwrap();
        match ok {
            Incoming::Response { id: 1, result } => {
                assert!(result.unwrap()["stopReason"] == "end_turn");
            }
            _ => panic!("应为 Response"),
        }

        let err =
            parse_incoming(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32601,"message":"no"}}"#)
                .unwrap();
        match err {
            Incoming::Response { id: 2, result } => {
                assert_eq!(result.unwrap_err(), (-32601, "no".to_string()));
            }
            _ => panic!("应为 Response(err)"),
        }
    }

    #[test]
    fn garbage_line_is_none() {
        assert!(parse_incoming("not json").is_none());
        assert!(parse_incoming(r#"{"foo":1}"#).is_none());
    }
}
