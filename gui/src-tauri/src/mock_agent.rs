//! 内置 mock agent(`qidiwork-gui.exe --mock-agent`)。
//!
//! M3 起为**迷你 ACP 服务器**:在 newline-delimited JSON-RPC 上实现桥
//! 所需的协议面(initialize / session/new / session/load / session/prompt /
//! session/cancel / session/update / session/request_permission),使桥的
//! 集成测试完全离线、不依赖 qidi.exe。仅用于测试与手动联调——生产
//! release 构建经 `#[cfg(any(test, debug_assertions))]` 门控,无此分支。
//!
//! 行为:
//! - `initialize` → `{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}`
//! - `session/new` → 递增 `sess-N`,并回传 `models` 状态(默认 mock-model-a)
//! - `session/set_model` → 确认应答 + 广播 `model_changed` 回显通知
//! - `session/load` → sessionId=="lost" 报 404 错误,其余原样确认(回传 mock-model-b)
//! - `session/prompt` → 先后发两条 `session/update`(agent_message_chunk:
//!   "处理中…" 与 "done: <原文>"),再以 `end_turn` 结束;
//!   文本含"需要权限"时先发 `session/request_permission`(allow/deny),
//!   内联等待应答后按选择继续(拒绝则以 `rejected` 结束)
//! - `session/cancel` 通知 → 忽略(无应答)
//! - `--sleep` → 睡 300s(Job/退出路径测试)

use std::io::{BufRead, Write};

use serde_json::{Value, json};

fn send(out: &mut impl Write, msg: &Value) -> bool {
    if writeln!(out, "{msg}").is_err() || out.flush().is_err() {
        return false; // stdout 断开(父进程死亡)
    }
    true
}

fn update(session_id: &str, text: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session_id,
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": text}
            }
        }
    })
}

/// 内核 session/new|load 应答的 models 字段形状(SessionModelState 线上格式)。
fn mock_models(current: &str) -> Value {
    json!({
        "currentModelId": current,
        "availableModels": [
            {"modelId": "mock-model-a", "name": "模型 A"},
            {"modelId": "mock-model-b", "name": "模型 B"}
        ]
    })
}

pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--sleep") {
        std::thread::sleep(std::time::Duration::from_secs(300));
        return 0;
    }
    // 可注入错误协议版本(桥的版本协商失败路径测试)
    let protocol_version: u64 = args
        .iter()
        .position(|a| a == "--protocol-version")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);

    let mut out = std::io::stdout();
    let mut session_counter: u64 = 0;
    let mut permission_counter: u64 = 9000;

    // 单一锁句柄贯穿全程:权限内联等待要再读 stdin,
    // 外层 for+lines() 会持锁导致内层 lock() 死锁(本项目踩过的坑)。
    let mut lock = std::io::stdin().lock();
    loop {
        let mut line = String::new();
        if lock.read_line(&mut line).unwrap_or(0) == 0 {
            break; // EOF
        }
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let id = v.get("id").cloned();
        let method = v.get("method").and_then(|m| m.as_str()).map(String::from);
        let params = v.get("params").cloned().unwrap_or(Value::Null);

        match (method.as_deref(), id) {
            (Some("initialize"), Some(id)) => {
                if !send(
                    &mut out,
                    &json!({
                        "jsonrpc":"2.0","id":id,
                        "result":{"protocolVersion":protocol_version,"agentCapabilities":{"loadSession":true},"authMethods":[]}
                    }),
                ) {
                    return 0;
                }
            }
            (Some("session/new"), Some(id)) => {
                session_counter += 1;
                let sid = format!("sess-{session_counter}");
                if !send(
                    &mut out,
                    &json!({"jsonrpc":"2.0","id":id,
                            "result":{"sessionId":sid,"models":mock_models("mock-model-a")}}),
                ) {
                    return 0;
                }
            }
            (Some("session/set_model"), Some(id)) => {
                let sid = params
                    .get("sessionId")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                let model = params
                    .get("modelId")
                    .and_then(|m| m.as_str())
                    .unwrap_or("")
                    .to_string();
                if !send(&mut out, &json!({"jsonrpc":"2.0","id":id,"result":{}})) {
                    return 0;
                }
                // 真实内核切换成功后会广播 model_changed 回显(Leader fan-out),
                // mock 照抄线上格式以覆盖桥的回显解析路径。
                if !send(
                    &mut out,
                    &json!({
                        "jsonrpc":"2.0",
                        "method":"x.ai/session_notification",
                        "params":{
                            "sessionId": sid,
                            "update": {"sessionUpdate":"model_changed","model_id": model}
                        }
                    }),
                ) {
                    return 0;
                }
            }
            (Some("session/load"), Some(id)) => {
                let sid = params
                    .get("sessionId")
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                let msg = if sid == "lost" {
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":404,"message":"unknown session"}})
                } else {
                    json!({"jsonrpc":"2.0","id":id,
                           "result":{"sessionId":sid,"models":mock_models("mock-model-b")}})
                };
                if !send(&mut out, &msg) {
                    return 0;
                }
            }
            (Some("session/prompt"), Some(id)) => {
                let session_id = params
                    .get("sessionId")
                    .and_then(|s| s.as_str())
                    .unwrap_or("sess-0")
                    .to_string();
                let text = params
                    .pointer("/prompt/0/text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();

                let mut denied = false;
                if text.contains("需要权限") {
                    permission_counter += 1;
                    let perm_id = permission_counter;
                    let req = json!({
                        "jsonrpc":"2.0","id":perm_id,
                        "method":"session/request_permission",
                        "params":{
                            "sessionId": session_id,
                            "toolCall": {"title": "模拟写文件"},
                            "options": [
                                {"id":"allow","name":"允许","kind":"allow_once"},
                                {"id":"deny","name":"拒绝","kind":"reject_once"}
                            ]
                        }
                    });
                    if !send(&mut out, &req) {
                        return 0;
                    }
                    // 内联等待权限应答(期间其他消息忽略——mock 无并发);
                    // 复用同一个锁句柄,不再 lock()(见上文说明)。
                    let outcome: Option<String> = loop {
                        let mut inner = String::new();
                        if lock.read_line(&mut inner).unwrap_or(0) == 0 {
                            return 0; // EOF
                        }
                        let Ok(r) = serde_json::from_str::<Value>(inner.trim()) else {
                            continue;
                        };
                        if r.get("id").and_then(|i| i.as_u64()) == Some(perm_id)
                            && r.get("method").is_none()
                        {
                            break r
                                .pointer("/result/outcome/optionId")
                                .and_then(|o| o.as_str())
                                .map(String::from);
                        }
                    };
                    match outcome.as_deref() {
                        Some("deny")
                            if {
                                denied = true;
                                true
                            } =>
                        {
                            if !send(&mut out, &update(&session_id, "已拒绝")) {
                                return 0;
                            }
                        }
                        Some(other) => {
                            if !send(&mut out, &update(&session_id, &format!("已授权:{other}")))
                            {
                                return 0;
                            }
                        }
                        None => return 0,
                    }
                }

                if !send(&mut out, &update(&session_id, "处理中…")) {
                    return 0;
                }
                let done = if denied {
                    "done(拒绝)".to_string()
                } else {
                    format!("done: {text}")
                };
                if !send(&mut out, &update(&session_id, &done)) {
                    return 0;
                }
                let stop = if denied { "rejected" } else { "end_turn" };
                if !send(
                    &mut out,
                    &json!({"jsonrpc":"2.0","id":id,"result":{"stopReason":stop}}),
                ) {
                    return 0;
                }
            }
            (Some("session/cancel"), None) => { /* 通知:忽略 */ }
            (Some(m), Some(id))
                if !send(
                    &mut out,
                    &json!({
                        "jsonrpc":"2.0","id":id,
                        "error":{"code":-32601,"message":format!("mock 未实现: {m}")}
                    }),
                ) =>
            {
                return 0;
            }
            _ => {}
        }
    }
    0
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn update_shape_matches_acp() {
        let u = update("sess-1", "hi");
        assert_eq!(u["method"], "session/update");
        assert_eq!(
            u["params"]["update"]["sessionUpdate"],
            "agent_message_chunk"
        );
        assert_eq!(u["params"]["update"]["content"]["type"], "text");
    }
}
