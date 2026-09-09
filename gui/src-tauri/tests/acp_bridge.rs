#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! ACP 桥集成测试:真实应用二进制的 `--mock-agent` 迷你 ACP 服务器。
//!
//! 覆盖:initialize / session/new / 流式 prompt 回合 / 权限审批回路
//! (allow 与 deny)/ 断连事件 / reconnect + session/load 恢复语义。

use std::path::PathBuf;
use std::time::Duration;

use qidiwork_gui::acp::{AcpBridge, BridgeEvent, reconnect};
use qidiwork_gui::process::{AgentProcess, SpawnConfig};
use qidiwork_gui::transport::AgentTransport as _;

fn mock_cfg() -> SpawnConfig {
    SpawnConfig {
        program: env!("CARGO_BIN_EXE_qidiwork-gui").to_string(),
        args: vec!["--mock-agent".into()],
        cwd: None,
    }
}

async fn next_event(rx: &mut tokio::sync::broadcast::Receiver<BridgeEvent>) -> BridgeEvent {
    tokio::time::timeout(Duration::from_secs(15), rx.recv())
        .await
        .expect("等待桥事件超时")
        .unwrap()
}

/// 收集事件直到目标回合结束,返回期间的所有事件。
async fn collect_until_turn_completed(
    rx: &mut tokio::sync::broadcast::Receiver<BridgeEvent>,
    session_id: &str,
) -> Vec<BridgeEvent> {
    let mut events = Vec::new();
    loop {
        let ev = next_event(rx).await;
        let done = matches!(
            &ev,
            BridgeEvent::TurnCompleted { session_id: s, .. } if s == session_id
        );
        events.push(ev);
        if done {
            return events;
        }
    }
}

fn update_texts(events: &[BridgeEvent], session_id: &str) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            BridgeEvent::SessionUpdate {
                session_id: s,
                update,
            } if s == session_id => update
                .pointer("/content/text")
                .and_then(|t| t.as_str())
                .map(String::from),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn initialize_new_session_and_prompt_roundtrip() {
    let transport = AgentProcess::spawn(mock_cfg()).await.unwrap();
    let bridge = AcpBridge::attach(transport.clone()).unwrap();
    bridge.ensure_initialized().await.unwrap();
    let session = bridge
        .new_session(PathBuf::from("."))
        .await
        .expect("session/new 失败");
    assert_eq!(session, "sess-1");

    let mut rx = bridge.subscribe();
    bridge.prompt(&session, "你好").unwrap();
    let events = collect_until_turn_completed(&mut rx, &session).await;

    let texts = update_texts(&events, &session);
    assert!(
        texts.iter().any(|t| t.contains("done: 你好")),
        "texts={texts:?}"
    );
    let Some(BridgeEvent::TurnCompleted { stop_reason, .. }) = events
        .iter()
        .rfind(|e| matches!(e, BridgeEvent::TurnCompleted { .. }))
    else {
        panic!("应有 TurnCompleted");
    };
    assert_eq!(stop_reason, "end_turn");

    transport.shutdown().await;
}

#[tokio::test]
async fn permission_roundtrip_allow_and_deny() {
    let transport = AgentProcess::spawn(mock_cfg()).await.unwrap();
    let bridge = AcpBridge::attach(transport.clone()).unwrap();
    bridge.ensure_initialized().await.unwrap();
    let session = bridge.new_session(PathBuf::from(".")).await.unwrap();
    let mut rx = bridge.subscribe();

    // allow 路径
    bridge.prompt(&session, "需要权限:写文件").unwrap();
    let ev = next_event(&mut rx).await;
    let BridgeEvent::PermissionRequest { request_id, .. } = ev else {
        panic!("应先收到权限请求,实际 {ev:?}");
    };
    bridge.resolve_permission(request_id, "allow").unwrap();
    let events = collect_until_turn_completed(&mut rx, &session).await;
    let texts = update_texts(&events, &session);
    assert!(
        texts.iter().any(|t| t.contains("已授权:allow")),
        "texts={texts:?}"
    );
    // 重复响应应报错(幂等保护)
    assert!(bridge.resolve_permission(request_id, "allow").is_err());

    // deny 路径
    bridge.prompt(&session, "需要权限:删文件").unwrap();
    let ev = next_event(&mut rx).await;
    let BridgeEvent::PermissionRequest { request_id, .. } = ev else {
        panic!("第二次权限请求缺失");
    };
    bridge.resolve_permission(request_id, "deny").unwrap();
    let events = collect_until_turn_completed(&mut rx, &session).await;
    let Some(BridgeEvent::TurnCompleted { stop_reason, .. }) = events
        .iter()
        .rfind(|e| matches!(e, BridgeEvent::TurnCompleted { .. }))
    else {
        panic!("deny 路径应有 TurnCompleted");
    };
    assert_eq!(stop_reason, "rejected");

    transport.shutdown().await;
}

#[tokio::test]
async fn disconnect_emits_and_breaks_pending() {
    let transport = AgentProcess::spawn(mock_cfg()).await.unwrap();
    let bridge = AcpBridge::attach(transport.clone()).unwrap();
    bridge.ensure_initialized().await.unwrap();
    let mut rx = bridge.subscribe();

    transport.shutdown().await;
    let ev = next_event(&mut rx).await;
    assert!(
        matches!(ev, BridgeEvent::Disconnected { .. }),
        "断开应广播 Disconnected,实际 {ev:?}"
    );
}

#[tokio::test]
async fn reconnect_restores_sessions_and_reports_lost() {
    // 第一段:建会话后强杀
    let transport = AgentProcess::spawn(mock_cfg()).await.unwrap();
    let bridge = AcpBridge::attach(transport.clone()).unwrap();
    bridge.ensure_initialized().await.unwrap();
    let mut rx = bridge.subscribe();
    let session = bridge.new_session(PathBuf::from(".")).await.unwrap();
    transport.shutdown().await;
    let ev = next_event(&mut rx).await;
    assert!(matches!(ev, BridgeEvent::Disconnected { .. }));

    // 第二段:reconnect 恢复 sess-1;"lost" 会话应报恢复失败但不拖垮整体
    let transport2 = AgentProcess::spawn(mock_cfg()).await.unwrap();
    let (bridge2, restored, failed) = reconnect(
        transport2,
        vec![
            (session.clone(), PathBuf::from(".")),
            ("lost".to_string(), PathBuf::from(".")),
        ],
    )
    .await
    .expect("reconnect 失败");
    assert_eq!(restored, vec![session.clone()], "lost 不应入恢复清单");
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].0, "lost");
    assert_eq!(bridge2.sessions().len(), 1, "桥内登记同样只含成功会话");
}

#[tokio::test]
async fn initialize_rejects_protocol_version_mismatch() {
    let cfg = SpawnConfig {
        program: env!("CARGO_BIN_EXE_qidiwork-gui").to_string(),
        args: vec![
            "--mock-agent".into(),
            "--protocol-version".into(),
            "2".into(),
        ],
        cwd: None,
    };
    let transport = AgentProcess::spawn(cfg).await.unwrap();
    let bridge = AcpBridge::attach(transport.clone()).unwrap();
    let err = bridge
        .ensure_initialized()
        .await
        .expect_err("版本不匹配必须失败");
    assert!(err.contains("协议版本不匹配"), "err={err}");
    // 不置位:重试仍会再次握手(而非假成功)
    assert!(bridge.ensure_initialized().await.is_err());
    transport.shutdown().await;
}

#[tokio::test]
async fn resolve_rejects_unknown_option_but_keeps_entry() {
    let transport = AgentProcess::spawn(mock_cfg()).await.unwrap();
    let bridge = AcpBridge::attach(transport.clone()).unwrap();
    bridge.ensure_initialized().await.unwrap();
    let session = bridge.new_session(PathBuf::from(".")).await.unwrap();
    let mut rx = bridge.subscribe();

    bridge.prompt(&session, "需要权限:写文件").unwrap();
    let ev = next_event(&mut rx).await;
    let BridgeEvent::PermissionRequest { request_id, .. } = ev else {
        panic!("应收到权限请求");
    };
    // 非法选项:拒绝且条目保留(可重试)
    assert!(bridge.resolve_permission(request_id, "bogus").is_err());
    // 合法选项随后成功
    bridge.resolve_permission(request_id, "allow").unwrap();
    let events = collect_until_turn_completed(&mut rx, &session).await;
    let texts = update_texts(&events, &session);
    assert!(
        texts.iter().any(|t| t.contains("已授权:allow")),
        "texts={texts:?}"
    );
    transport.shutdown().await;
}

#[tokio::test]
async fn cancel_permission_completes_roundtrip_once() {
    let transport = AgentProcess::spawn(mock_cfg()).await.unwrap();
    let bridge = AcpBridge::attach(transport.clone()).unwrap();
    bridge.ensure_initialized().await.unwrap();
    let session = bridge.new_session(PathBuf::from(".")).await.unwrap();
    let mut rx = bridge.subscribe();

    bridge.prompt(&session, "需要权限:操作").unwrap();
    let ev = next_event(&mut rx).await;
    let BridgeEvent::PermissionRequest { request_id, .. } = ev else {
        panic!("应收到权限请求");
    };
    bridge.cancel_permission(request_id).unwrap();
    assert!(
        bridge.cancel_permission(request_id).is_err(),
        "取消必须幂等保护"
    );
    // mock 对 cancelled 走非 deny 分支:回合照常收尾
    let events = collect_until_turn_completed(&mut rx, &session).await;
    assert!(!events.is_empty());
    transport.shutdown().await;
}
