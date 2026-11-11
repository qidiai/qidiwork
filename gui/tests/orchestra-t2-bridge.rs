use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use crate::acp::{AcpBridge, BridgeEvent};
use crate::transport::{AgentTransport, ExitInfo, TransportError};
use tokio::sync::{broadcast, mpsc};

struct TestTransport {
    incoming: mpsc::Sender<String>,
    receiver: Mutex<Option<mpsc::Receiver<String>>>,
    exit: broadcast::Sender<ExitInfo>,
    sent: Mutex<Vec<serde_json::Value>>,
    next: AtomicU64,
    hold_model: AtomicBool,
}
impl TestTransport {
    fn new() -> Arc<Self> {
        let (incoming, receiver) = mpsc::channel(128);
        let (exit, _) = broadcast::channel(16);
        Arc::new(Self { incoming, receiver: Mutex::new(Some(receiver)), exit, sent: Mutex::new(Vec::new()), next: AtomicU64::new(1), hold_model: AtomicBool::new(false) })
    }
    fn inject(&self, value: serde_json::Value) { self.incoming.try_send(value.to_string()).unwrap(); }
    fn calls(&self) -> Vec<serde_json::Value> { self.sent.lock().unwrap().clone() }
}
impl AgentTransport for TestTransport {
    fn send(&self, line: String) -> Result<(), TransportError> {
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        self.sent.lock().unwrap().push(value.clone());
        let result = match value["method"].as_str() {
            Some("initialize") => Some(serde_json::json!({"protocolVersion": 1, "agentCapabilities": {}})),
            Some("session/new") => Some(serde_json::json!({"sessionId": format!("case-{}", self.next.fetch_add(1, Ordering::Relaxed)), "models": {"currentModelId": "test", "availableModels": []}})),
            Some("session/load") => Some(serde_json::json!({"models": {"currentModelId": "restored", "availableModels": []}})),
            Some("session/set_model") if !self.hold_model.load(Ordering::Relaxed) => Some(serde_json::json!({})),
            _ => None,
        };
        if let Some(result) = result { self.inject(serde_json::json!({"jsonrpc":"2.0", "id":value["id"], "result":result})); }
        Ok(())
    }
    fn take_line_receiver(&self) -> Result<mpsc::Receiver<String>, TransportError> {
        self.receiver.lock().unwrap().take().ok_or(TransportError::Closed)
    }
    fn subscribe_exit(&self) -> broadcast::Receiver<ExitInfo> { self.exit.subscribe() }
    fn is_running(&self) -> bool { true }
    fn terminate(&self) { let _ = self.exit.send(ExitInfo { code: Some(0), success: true }); }
    fn shutdown(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output=()> + Send + '_>> { Box::pin(async { self.terminate(); }) }
}

async fn setup() -> (Arc<TestTransport>, Arc<AcpBridge>, String, String) {
    let transport = TestTransport::new();
    let bridge = AcpBridge::attach(transport.clone()).unwrap();
    bridge.ensure_initialized().await.unwrap();
    let a = bridge.new_session(PathBuf::from("C:/a"), Some("a".into())).await.unwrap();
    let b = bridge.new_session(PathBuf::from("C:/b"), Some("b".into())).await.unwrap();
    (transport, bridge, a, b)
}

#[tokio::test]
async fn forgetting_removes_recovery_and_model_state_without_touching_other_session() {
    let (transport, bridge, a, b) = setup().await;
    assert!(bridge.model_state(&a).is_some());
    assert!(bridge.forget_session(&a));
    assert!(!bridge.forget_session(&a));
    assert!(!bridge.forget_session("never-created"));
    assert!(!bridge.sessions().iter().any(|(id, _, _)| id == &a));
    assert!(bridge.model_state(&a).is_none());
    assert!(bridge.model_state(&b).is_some());
    assert_eq!(bridge.sessions().len(), 1);
    let count = transport.calls().len();
    assert!(bridge.prompt(&a, "不能再发", &[]).is_err());
    assert!(bridge.set_model(&a, "test", None).await.is_err());
    assert_eq!(transport.calls().len(), count, "已移除会话不得再下发请求");
    assert!(bridge.prompt(&b, "另一会话仍能工作", &[]).is_ok());
    transport.terminate();
}

#[tokio::test]
async fn forgotten_session_drops_late_updates_permissions_and_completion() {
    let (transport, bridge, a, b) = setup().await;
    let mut events = bridge.subscribe();
    bridge.prompt(&a, "待完成", &[]).unwrap();
    let prompt_id = transport.calls().iter().rev().find(|v| v["method"] == "session/prompt").unwrap()["id"].clone();
    transport.inject(serde_json::json!({"jsonrpc":"2.0","id":900,"method":"session/request_permission","params":{"sessionId":a,"toolCall":{"title":"等待确认"},"options":[{"optionId":"ok","name":"允许","kind":"allow_once"}]}}));
    let permission = tokio::time::timeout(Duration::from_secs(2), events.recv()).await.unwrap().unwrap();
    assert!(matches!(permission, BridgeEvent::PermissionRequest { request_id:900,.. }));
    assert!(bridge.forget_session(&a));
    assert!(bridge.resolve_permission(900,"ok").is_err(), "已删除会话的授权应失效");
    while events.try_recv().is_ok() {}
    bridge.notify_session_restored(&a);
    transport.inject(serde_json::json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":a,"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"晚到的旧数据"}}}}));
    transport.inject(serde_json::json!({"jsonrpc":"2.0","id":901,"method":"session/request_permission","params":{"sessionId":a,"toolCall":{"title":"晚到审批"},"options":[{"optionId":"ok","name":"允许"}]}}));
    transport.inject(serde_json::json!({"jsonrpc":"2.0","id":prompt_id,"result":{"stopReason":"end_turn","_meta":{"usage":{"inputTokens":10,"outputTokens":5}}}}));
    transport.inject(serde_json::json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":b,"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"屏障"}}}}));
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv()).await.unwrap().unwrap();
        match event {
            BridgeEvent::SessionUpdate { session_id, .. } if session_id == b => break,
            other => panic!("移除后不应再显示旧会话事件: {other:?}"),
        }
    }
    assert!(tokio::time::timeout(Duration::from_millis(100), events.recv()).await.is_err(), "晚到完成/费用事件不得复活会话");
    assert!(bridge.resolve_permission(901,"ok").is_err());
    transport.terminate();
}

#[tokio::test]
async fn late_model_reply_and_notification_cannot_recreate_forgotten_model_cache() {
    let (transport, bridge, a, b) = setup().await;
    transport.hold_model.store(true, Ordering::Relaxed);
    let pending_bridge = bridge.clone();
    let pending_id = a.clone();
    let pending = tokio::spawn(async move { pending_bridge.set_model(&pending_id, "new-model", None).await });
    let id = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(call) = transport.calls().iter().find(|c| c["method"] == "session/set_model") {
                break call["id"].clone();
            }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    assert!(bridge.forget_session(&a));
    transport.inject(serde_json::json!({"jsonrpc":"2.0","id":id,"result":{}}));
    assert!(tokio::time::timeout(Duration::from_secs(2), pending).await.unwrap().unwrap().is_err());
    let mut events = bridge.subscribe();
    transport.inject(serde_json::json!({"jsonrpc":"2.0","method":"x.ai/session_notification","params":{"sessionId":a,"update":{"sessionUpdate":"model_changed","model_id":"late-model"}}}));
    transport.inject(serde_json::json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":b,"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"屏障"}}}}));
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv()).await.unwrap().unwrap();
    assert!(matches!(event, BridgeEvent::SessionUpdate { session_id, .. } if session_id == b));
    assert!(bridge.model_state(&a).is_none());
    assert!(bridge.model_state(&b).is_some());
    transport.terminate();
}
