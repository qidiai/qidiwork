use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::acp::{AcpBridge, BridgeEvent};
use crate::transport::{AgentTransport, ExitInfo, TransportError};
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};

struct RecordingTransport {
    incoming: mpsc::Sender<String>,
    receiver: Mutex<Option<mpsc::Receiver<String>>>,
    exit: broadcast::Sender<ExitInfo>,
    sent: Mutex<Vec<Value>>,
    fail_cancel: AtomicBool,
    fail_prompt: AtomicBool,
    bridge: Mutex<std::sync::Weak<AcpBridge>>,
}

impl RecordingTransport {
    fn new() -> Arc<Self> {
        let (incoming, receiver) = mpsc::channel(128);
        let (exit, _) = broadcast::channel(16);
        Arc::new(Self {
            incoming,
            receiver: Mutex::new(Some(receiver)),
            exit,
            sent: Mutex::new(Vec::new()),
            fail_cancel: AtomicBool::new(false),
            fail_prompt: AtomicBool::new(false),
            bridge: Mutex::new(std::sync::Weak::new()),
        })
    }

    fn inject(&self, frame: Value) {
        self.incoming.try_send(frame.to_string()).unwrap();
    }

    fn frames(&self) -> Vec<Value> {
        self.sent.lock().unwrap().clone()
    }

    fn replies(&self, id: u64) -> Vec<Value> {
        self.frames()
            .into_iter()
            .filter(|v| v["id"] == id && v.get("method").is_none())
            .collect()
    }

    fn prompt_id(&self, session: &str) -> Value {
        self.frames()
            .iter()
            .rev()
            .find(|v| v["method"] == "session/prompt" && v["params"]["sessionId"] == session)
            .unwrap()["id"]
            .clone()
    }
}

impl AgentTransport for RecordingTransport {
    fn send(&self, line: String) -> Result<(), TransportError> {
        let frame: Value = serde_json::from_str(&line).unwrap();
        if let Some(bridge) = self.bridge.lock().unwrap().upgrade() {
            // try_lock assertions inside this helper catch sending under either lock.
            bridge.permission_wait_counts();
        }
        if (frame["method"] == "session/cancel" && self.fail_cancel.load(Ordering::Relaxed))
            || (frame["method"] == "session/prompt" && self.fail_prompt.load(Ordering::Relaxed))
        {
            return Err(TransportError::Backpressure);
        }
        self.sent.lock().unwrap().push(frame.clone());
        if frame["method"] == "session/load" {
            self.inject(json!({"jsonrpc":"2.0", "id":frame["id"], "result":{}}));
        }
        Ok(())
    }

    fn take_line_receiver(&self) -> Result<mpsc::Receiver<String>, TransportError> {
        self.receiver
            .lock()
            .unwrap()
            .take()
            .ok_or(TransportError::Closed)
    }

    fn subscribe_exit(&self) -> broadcast::Receiver<ExitInfo> {
        self.exit.subscribe()
    }
    fn is_running(&self) -> bool {
        true
    }
    fn terminate(&self) {
        self.exit
            .send(ExitInfo {
                code: Some(0),
                success: true,
            })
            .unwrap();
    }
    fn shutdown(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async {
            self.terminate();
        })
    }
}

struct Fixture {
    transport: Arc<RecordingTransport>,
    bridge: Arc<AcpBridge>,
    events: broadcast::Receiver<BridgeEvent>,
}

async fn settle() {
    // Only yield runnable tasks: never let the paused runtime auto-advance time.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

async fn advance(seconds: u64) {
    tokio::time::advance(Duration::from_secs(seconds)).await;
    settle().await;
}

impl Fixture {
    async fn new() -> Self {
        let transport = RecordingTransport::new();
        let bridge = AcpBridge::attach(transport.clone()).unwrap();
        *transport.bridge.lock().unwrap() = Arc::downgrade(&bridge);
        for session in ["a", "b"] {
            bridge
                .load_session(session, PathBuf::from("G:/qidiwork"), None)
                .await
                .unwrap();
        }
        let events = bridge.subscribe();
        Self {
            transport,
            bridge,
            events,
        }
    }

    fn drain(&mut self) -> Vec<BridgeEvent> {
        let mut result = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(event) => result.push(event),
                Err(broadcast::error::TryRecvError::Empty) => return result,
                Err(error) => panic!("event stream lost: {error}"),
            }
        }
    }

    async fn prompt(&self, session: &str) {
        self.bridge
            .prompt(session, "wait for permission", &[])
            .unwrap();
        settle().await;
    }

    async fn permission(&mut self, session: &str, id: u64, reject: bool) {
        let mut options = vec![json!({"optionId":"allow", "kind":"allow_once", "name":"Allow"})];
        if reject {
            options.push(json!({"optionId":"reject", "kind":"reject_once", "name":"Reject"}));
        }
        self.transport.inject(json!({
            "jsonrpc":"2.0", "id":id, "method":"session/request_permission",
            "params":{"sessionId":session, "toolCall":{"title":"approval"}, "options":options}
        }));
        settle().await;
        let events = self.drain();
        assert!(
            matches!(events.as_slice(), [BridgeEvent::PermissionRequest { request_id, session_id, .. }]
            if *request_id == id && session_id == session),
            "permission registration: {events:?}"
        );
    }

    fn quiet(&mut self, ids: &[u64]) {
        for id in ids {
            assert!(
                self.transport.replies(*id).is_empty(),
                "permission {id} must wait without an automatic reply: {:?}",
                self.transport.replies(*id)
            );
        }
        let events = self.drain();
        assert!(
            events.is_empty(),
            "waiting must not cancel or finish a turn: {events:?}"
        );
    }

    fn selected_once(&self, id: u64, option: &str) {
        assert_eq!(
            self.transport.replies(id),
            vec![json!({
                "jsonrpc":"2.0", "id":id, "result":{"outcome":{"outcome":"selected", "optionId":option}}
            })]
        );
        assert!(self.bridge.resolve_permission(id, option).is_err());
        assert!(self.bridge.cancel_permission(id).is_err());
        assert_eq!(self.transport.replies(id).len(), 1);
    }

    fn cancelled_once(&self, id: u64) {
        assert_eq!(
            self.transport.replies(id),
            vec![json!({
                "jsonrpc":"2.0", "id":id, "result":{"outcome":{"outcome":"cancelled"}}
            })]
        );
        assert!(self.bridge.resolve_permission(id, "allow").is_err());
        assert!(self.bridge.cancel_permission(id).is_err());
        assert_eq!(self.transport.replies(id).len(), 1);
    }

    fn timeout(&mut self, session: &str) {
        let events = self.drain();
        assert!(
            matches!(events.as_slice(), [BridgeEvent::TurnCompleted { session_id, stop_reason }]
            if session_id == session && stop_reason == "error: timeout/disconnected"),
            "timeout: {events:?}"
        );
        assert!(
            !self
                .transport
                .frames()
                .iter()
                .any(|v| v["method"] == "session/cancel"),
            "budget expiry must not send kernel cancel"
        );
    }
}

async fn long_wait(option: &str, reject: bool) {
    let mut f = Fixture::new().await;
    f.prompt("a").await;
    f.permission("a", 700, reject).await;
    for seconds in [120, 1800 - 120, 86400 - 1800] {
        advance(seconds).await;
        f.quiet(&[700]);
    }
    f.bridge.resolve_permission(700, option).unwrap();
    f.selected_once(700, option);
    f.transport.inject(json!({"jsonrpc":"2.0", "id":f.transport.prompt_id("a"), "result":{"stopReason":"end_turn"}}));
    settle().await;
    let events = f.drain();
    assert!(
        matches!(events.as_slice(), [BridgeEvent::TurnCompleted { stop_reason, .. }] if stop_reason == "end_turn"),
        "{events:?}"
    );
    advance(86400).await;
    f.selected_once(700, option);
    assert!(f.drain().is_empty());
}

// A1, A4: default public attach, real routed permission and outbound response.
#[tokio::test(start_paused = true)]
async fn default_attach_waits_a_day_then_allows_once() {
    long_wait("allow", true).await;
}

#[tokio::test(start_paused = true)]
async fn default_attach_waits_a_day_then_rejects_once() {
    long_wait("reject", true).await;
}

#[tokio::test(start_paused = true)]
async fn no_reject_option_still_waits_a_day() {
    long_wait("allow", false).await;
}

// A2: unchanged runtime budget when no permission is pending.
#[tokio::test(start_paused = true)]
async fn prompt_without_permission_times_out_at_thirty_minutes() {
    let mut f = Fixture::new().await;
    f.prompt("a").await;
    advance(1799).await;
    f.quiet(&[]);
    advance(1).await;
    f.timeout("a");
}

#[tokio::test(start_paused = true)]
async fn approval_resumes_remaining_budget_not_a_fresh_budget() {
    let mut f = Fixture::new().await;
    f.prompt("a").await;
    advance(300).await;
    f.permission("a", 710, true).await;
    advance(86400).await;
    f.quiet(&[710]);
    f.bridge.resolve_permission(710, "allow").unwrap();
    advance(1499).await;
    assert!(f.drain().is_empty());
    advance(1).await;
    f.timeout("a");
    f.selected_once(710, "allow");
}

#[tokio::test(start_paused = true)]
async fn overlapping_permissions_resume_only_after_last_removal() {
    let mut f = Fixture::new().await;
    f.prompt("a").await;
    advance(300).await;
    f.permission("a", 720, true).await;
    advance(90).await;
    f.permission("a", 721, true).await;
    assert!(f.bridge.resolve_permission(721, "not-an-option").is_err());
    f.bridge.resolve_permission(720, "reject").unwrap();
    advance(86400).await;
    f.quiet(&[721]);
    f.selected_once(720, "reject");
    f.bridge.cancel_permission(721).unwrap();
    f.cancelled_once(721);
    advance(1499).await;
    assert!(f.drain().is_empty());
    advance(1).await;
    f.timeout("a");
}

#[tokio::test(start_paused = true)]
async fn permission_pauses_only_its_own_session() {
    let mut f = Fixture::new().await;
    f.prompt("a").await;
    f.prompt("b").await;
    advance(300).await;
    f.permission("a", 730, true).await;
    advance(1500).await;
    f.timeout("b");
    f.quiet(&[730]);
    f.bridge.resolve_permission(730, "allow").unwrap();
    advance(1499).await;
    assert!(f.drain().is_empty());
    advance(1).await;
    f.timeout("a");
}

// A3: successful cancellation clears reverse requests, not the prompt waiter.
#[tokio::test(start_paused = true)]
async fn cancel_clears_only_its_session_and_preserves_prompt_budget() {
    let mut f = Fixture::new().await;
    f.prompt("a").await;
    f.prompt("b").await;
    advance(300).await;
    f.permission("a", 740, true).await;
    f.permission("a", 741, false).await;
    f.permission("b", 742, true).await;
    advance(86400).await;
    f.quiet(&[740, 741, 742]);
    f.bridge.cancel("a").unwrap();
    assert_eq!(f.bridge.permission_wait_counts(), (1, 2, 2));
    f.cancelled_once(740);
    f.cancelled_once(741);
    let events = f.drain();
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|e| matches!(e, BridgeEvent::PermissionCancelled { session_id, request_id: None, .. } if session_id == "a")));
    f.quiet(&[742]);
    advance(1499).await;
    assert!(f.drain().is_empty());
    advance(1).await;
    let events = f.drain();
    assert!(
        matches!(events.as_slice(), [BridgeEvent::TurnCompleted { session_id, stop_reason }]
        if session_id == "a" && stop_reason == "error: timeout/disconnected"),
        "{events:?}"
    );
    f.quiet(&[742]);
    f.bridge.resolve_permission(742, "allow").unwrap();
}

#[tokio::test(start_paused = true)]
async fn failed_cancel_keeps_permission_and_paused_budget() {
    let mut f = Fixture::new().await;
    f.prompt("a").await;
    advance(300).await;
    f.permission("a", 750, true).await;
    f.transport.fail_cancel.store(true, Ordering::Relaxed);
    assert_eq!(
        f.bridge.cancel("a").unwrap_err(),
        "发送 session/cancel 失败: agent 写入队列已满(疑似假死)"
    );
    assert_eq!(f.bridge.permission_wait_counts(), (1, 1, 1));
    advance(86400).await;
    f.quiet(&[750]);
    f.bridge.resolve_permission(750, "allow").unwrap();
    advance(1499).await;
    assert!(f.drain().is_empty());
    advance(1).await;
    f.timeout("a");
}

#[tokio::test(start_paused = true)]
async fn forget_clears_waiter_and_rejects_late_answers_without_touching_sibling() {
    let mut f = Fixture::new().await;
    f.prompt("a").await;
    f.prompt("b").await;
    f.permission("a", 760, true).await;
    f.permission("b", 761, true).await;
    assert!(f.bridge.forget_session("a"));
    assert!(!f.bridge.forget_session("a"));
    f.cancelled_once(760);
    let events = f.drain();
    assert!(
        matches!(events.as_slice(), [BridgeEvent::PermissionCancelled { session_id, request_id: None, reason }]
        if session_id == "a" && reason == "session_removed"),
        "{events:?}"
    );
    f.transport.inject(json!({"jsonrpc":"2.0", "id":f.transport.prompt_id("a"), "result":{"stopReason":"end_turn"}}));
    advance(86400).await;
    f.quiet(&[761]);
    f.bridge.resolve_permission(761, "allow").unwrap();
}

#[tokio::test(start_paused = true)]
async fn exit_cancels_permissions_and_finishes_waiters_once() {
    let mut f = Fixture::new().await;
    f.prompt("a").await;
    f.prompt("b").await;
    f.permission("a", 770, true).await;
    f.permission("b", 771, false).await;
    f.transport.terminate();
    settle().await;
    f.cancelled_once(770);
    f.cancelled_once(771);
    let events = f.drain();
    assert_eq!(events.iter().filter(|e| matches!(e, BridgeEvent::PermissionCancelled { request_id: None, reason, .. } if reason == "bridge_restart")).count(), 2);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, BridgeEvent::Disconnected { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, BridgeEvent::TurnCompleted { .. }))
            .count(),
        2
    );
    advance(86400).await;
    assert!(f.drain().is_empty());
}

#[tokio::test(start_paused = true)]
async fn turn_response_cleans_permissions_inside_bridge_idempotently() {
    let mut f = Fixture::new().await;
    f.prompt("a").await;
    f.prompt("b").await;
    f.permission("a", 780, true).await;
    f.permission("b", 781, true).await;
    f.transport.inject(json!({"jsonrpc":"2.0", "id":f.transport.prompt_id("a"), "result":{"stopReason":"end_turn", "_meta":{"usage":{}}}}));
    settle().await;
    assert!(f.bridge.resolve_permission(780, "allow").is_err());
    assert!(f.bridge.cancel_permission(780).is_err());
    assert_eq!(
        f.bridge.expire_permissions_for_session("a", "turn_ended"),
        0
    );
    assert!(
        f.transport.replies(780).is_empty(),
        "ended turn must not receive a duplicate reverse reply"
    );
    let events = f.drain();
    assert!(
        matches!(events.as_slice(), [
        BridgeEvent::PermissionCancelled { session_id, request_id: None, reason },
        BridgeEvent::TurnUsage { .. }, BridgeEvent::TurnCompleted { stop_reason, .. }
    ] if session_id == "a" && reason == "turn_ended" && stop_reason == "end_turn"),
        "{events:?}"
    );
    advance(86400).await;
    f.quiet(&[781]);
    f.bridge.resolve_permission(781, "reject").unwrap();
}

#[tokio::test(start_paused = true)]
async fn failed_prompt_send_leaves_no_later_completion() {
    let mut f = Fixture::new().await;
    f.transport.fail_prompt.store(true, Ordering::Relaxed);
    assert!(f.bridge.prompt("a", "not sent", &[]).is_err());
    advance(86400).await;
    f.quiet(&[]);
    f.transport.fail_prompt.store(false, Ordering::Relaxed);
    f.prompt("a").await;
    advance(1800).await;
    f.timeout("a");
}

// A4: ordinary RPC still expires even while that session awaits permission.
#[tokio::test(start_paused = true)]
async fn ordinary_rpc_still_times_out_at_sixty_seconds() {
    let mut f = Fixture::new().await;
    f.permission("a", 790, true).await;
    let bridge = f.bridge.clone();
    let task = tokio::spawn(async move { bridge.set_model("a", "test", None).await });
    settle().await;
    advance(59).await;
    assert!(!task.is_finished());
    advance(1).await;
    assert!(task.is_finished());
    assert_eq!(
        task.await.unwrap().unwrap_err(),
        "session/set_model 超时(60s)"
    );
    f.quiet(&[790]);
}

// Advance the paused clock without yielding this task to the prompt waiter.
// The Instant assertion verifies that the single poll actually moved time.
fn advance_without_polling(seconds: u64) {
    use std::future::Future;
    let before = tokio::time::Instant::now();
    let mut advance = std::pin::pin!(tokio::time::advance(Duration::from_secs(seconds)));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    let _ = advance.as_mut().poll(&mut context);
    assert_eq!(
        tokio::time::Instant::now() - before,
        Duration::from_secs(seconds)
    );
}

fn synchronous_permission(f: &mut Fixture, id: u64) {
    f.bridge.permission_without_polling(id, "a");
    let events = f.drain();
    assert!(
        matches!(events.as_slice(), [BridgeEvent::PermissionRequest { request_id, .. }] if *request_id == id),
        "{events:?}"
    );
}

// A2: both edges precede the prompt task's very first poll.
#[tokio::test(start_paused = true)]
async fn short_approval_before_first_prompt_poll_preserves_elapsed_budget() {
    let mut f = Fixture::new().await;
    f.bridge.prompt("a", "not polled yet", &[]).unwrap();
    advance_without_polling(300);
    synchronous_permission(&mut f, 800);
    advance_without_polling(10);
    f.bridge.resolve_permission(800, "allow").unwrap();
    settle().await;
    advance(1499).await;
    assert!(f.drain().is_empty());
    advance(1).await;
    f.timeout("a");
}

// A2: watch may coalesce every pause/resume edge; only the ledger is authoritative.
#[tokio::test(start_paused = true)]
async fn coalesced_watch_changes_do_not_lose_paused_time() {
    let mut f = Fixture::new().await;
    f.prompt("a").await;
    advance_without_polling(300);
    synchronous_permission(&mut f, 810);
    advance_without_polling(900);
    f.bridge.resolve_permission(810, "allow").unwrap();
    advance_without_polling(200);
    synchronous_permission(&mut f, 811);
    advance_without_polling(600);
    f.bridge.resolve_permission(811, "allow").unwrap();
    settle().await;
    assert!(
        f.drain().is_empty(),
        "old timer must not expire a resumed budget"
    );
    advance(1299).await;
    assert!(f.drain().is_empty());
    advance(1).await;
    f.timeout("a");
}

#[tokio::test(start_paused = true)]
async fn expired_timer_rechecks_active_permission_before_timing_out() {
    let mut f = Fixture::new().await;
    f.prompt("a").await;
    advance_without_polling(1799);
    synchronous_permission(&mut f, 820);
    advance_without_polling(100);
    settle().await;
    f.quiet(&[820]);
    f.bridge.resolve_permission(820, "allow").unwrap();
    settle().await;
    assert!(f.drain().is_empty());
    advance(1).await;
    f.timeout("a");
}

#[tokio::test(start_paused = true)]
async fn permission_winning_deadline_lock_waits_even_with_zero_budget() {
    let mut f = Fixture::new().await;
    f.prompt("a").await;
    advance_without_polling(1800);
    synchronous_permission(&mut f, 830);
    settle().await;
    advance(86400).await;
    f.quiet(&[830]);
    f.bridge.resolve_permission(830, "allow").unwrap();
    settle().await;
    f.timeout("a");
    f.selected_once(830, "allow");
}

#[tokio::test(start_paused = true)]
async fn permission_before_prompt_registration_starts_with_full_budget() {
    let mut f = Fixture::new().await;
    f.permission("a", 840, true).await;
    advance(86400).await;
    f.prompt("a").await;
    advance(86400).await;
    f.quiet(&[840]);
    f.bridge.resolve_permission(840, "reject").unwrap();
    advance(1799).await;
    assert!(f.drain().is_empty());
    advance(1).await;
    f.timeout("a");
}

// A2/A3: empty maps alone are insufficient: check the task's Arc also goes away.
#[tokio::test(start_paused = true)]
async fn cleanup_releases_budgets_pending_entries_and_prompt_tasks() {
    for action in [
        "forget",
        "exit",
        "success",
        "error",
        "timeout",
        "send_failure",
    ] {
        let mut f = Fixture::new().await;
        let references = Arc::strong_count(&f.bridge);
        if action == "send_failure" {
            f.transport.fail_prompt.store(true, Ordering::Relaxed);
            assert!(f.bridge.prompt("a", "not sent", &[]).is_err());
        } else {
            f.prompt("a").await;
            assert_eq!(Arc::strong_count(&f.bridge), references + 1);
            if action != "timeout" {
                f.permission("a", 850, true).await;
                assert_eq!(f.bridge.permission_wait_counts(), (1, 1, 1));
            }
            match action {
                "forget" => { assert!(f.bridge.forget_session("a")); }
                "exit" => f.transport.terminate(),
                "success" => f.transport.inject(json!({"jsonrpc":"2.0", "id":f.transport.prompt_id("a"), "result":{"stopReason":"end_turn"}})),
                "error" => f.transport.inject(json!({"jsonrpc":"2.0", "id":f.transport.prompt_id("a"), "error":{"code":-1,"message":"turn failed"}})),
                "timeout" => advance(1800).await,
                other => panic!("unexpected action {other}"),
            }
        }
        settle().await;
        assert_eq!(f.bridge.permission_wait_counts(), (0, 0, 0), "{action}");
        let expected = if action == "exit" {
            references - 1
        } else {
            references
        };
        assert_eq!(
            Arc::strong_count(&f.bridge),
            expected,
            "prompt waiter retained bridge after {action}"
        );
        assert!(f.bridge.resolve_permission(850, "allow").is_err());
    }
}
