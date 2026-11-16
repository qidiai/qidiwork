//! ACP 会话桥:在 [`AgentTransport`] 之上实现 initialize / session/new /
//! session/prompt / session/cancel / session/load 与流式 `session/update`
//! 转发、权限审批回路、请求级超时(k3 M2 审计衔接缺口:超时不依赖传输层)。
//!
//! 事件模型:`BridgeEvent` 走 broadcast(展示流,多订阅者);
//! 协议入站行走 M2 定稿的可靠 mpsc(桥是唯一消费者,不丢帧)。
//!
//! 审计定稿(k3 M3):
//! - 权限回路的 optionId / sessionId 均做校验,未知会话的帧不入库不透传;
//! - 断连时对挂起权限回 `cancelled` 应答后再清理;
//! - initialize 校验 `result.protocolVersion`,不匹配即失败;
//! - 路由循环内不 await 写队列(防御性拒绝走 best-effort 后台任务);
//! - 权限请求带 per-request 截止时间(默认 120s):到期仍无人回填即按
//!   **拒绝**回应 agent,**绝不自动 allow**(缺口:见 `spawn_permission_timeout`);
//! - 注意 `PROMPT_TIMEOUT` 超时 ≠ 取消:超时只发 TurnCompleted 错误事件,
//!   agent 侧仍在跑,真正中断需配合 `session/cancel`(M4 编排)。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::Instant;

use crate::acp::jsonrpc::{self, ACP_PROTOCOL_VERSION, ERR_METHOD_NOT_FOUND, Incoming};
use crate::transport::{AgentTransport, ExitInfo, TransportError};

/// 单个 RPC 的默认超时。session/prompt 的回合可能跑很久,单独放宽。
const RPC_TIMEOUT: Duration = Duration::from_secs(60);
const PROMPT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// 单条权限请求的默认截止时间(per-request)。到期仍无人回填 → 按**拒绝**
/// 回应 agent 并广播 `PermissionCancelled{reason:"timeout"}`,**绝不自动 allow**。
/// 与 `PROMPT_TIMEOUT` 无关:后者只防前端无限等 `session/prompt` 应答。
const DEFAULT_PERMISSION_TIMEOUT: Duration = Duration::from_secs(120);
/// 展示流(桥 → 前端转发任务)的 broadcast 容量。256 在长回合密集
/// session/update 下易被追平 → Lagged 丢帧,最坏丢 TurnCompleted 使
/// 前端 busy 永久卡住(见 useAgent.ts 头部注释)。提到 1024 降低触发
/// 概率;残余丢帧仍由 QueueLagged 告警兜底。
const EVENT_CAPACITY: usize = 1024;

/// 桥事件(转发给前端 / 测试断言)。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeEvent {
    /// agent 的流式更新(session/update 的 update 字段原样透传,
    /// 前端按 update.sessionUpdate 分类渲染:M4)。
    SessionUpdate {
        session_id: String,
        update: serde_json::Value,
    },
    /// 权限审批请求。前端弹窗后经 `resolve_permission` / `cancel_permission`
    /// 回填。
    PermissionRequest {
        request_id: u64,
        session_id: String,
        /// 规范中的工具调用摘要(原样透传,供展示)。
        tool_call: serde_json::Value,
        options: serde_json::Value,
    },
    /// A pending permission request became void: either the bridge dropped its
    /// pending table (bridge restart / agent exit) or the turn that asked for
    /// it has ended. The frontend drops the matching permission queue entry and
    /// clears a stuck `busy` flag, so no zombie dialog survives.
    /// The old `request_id` is deliberately NOT re-sent: once the connection or
    /// the turn is gone, the kernel side id is dead too (see the
    /// pending-interaction semantics in the kernel).
    PermissionCancelled {
        session_id: String,
        reason: String,
    },
    /// agent 传输断开(崩溃/被杀)。恢复编排:`reconnect`。
    Disconnected {
        code: Option<i32>,
        success: bool,
    },
    /// 一个回合结束(session/prompt 的应答)。
    TurnCompleted {
        session_id: String,
        stop_reason: String,
    },
    /// 回合用量(session/prompt 应答的 `_meta` 原样透传:usage 为全回合
    /// token/成本汇总,modelUsage 键为实际使用的模型 id)。仅成功应答
    /// 携带。原样透传而非镜像成强类型:内核字段演进不破坏 GUI,前端
    /// 按 feature-detect 宽松解析(双 casing 兼容)。
    TurnUsage {
        session_id: String,
        meta: serde_json::Value,
    },
    /// 会话模型状态(session/new / session/load 应答 models 字段与
    /// set_model/model_changed 回显)。state 为内核 SessionModelState
    /// 原样透传:{ currentModelId, availableModels:[{modelId,name}] }。
    ModelState {
        session_id: String,
        state: serde_json::Value,
    },
    /// 会话恢复结果(重 spawn 后)。
    SessionRestored {
        session_id: String,
    },
    SessionRestoreFailed {
        session_id: String,
        error: String,
    },
    /// 展示流拥堵告警:桥 → 前端的转发任务订阅滞后超过 [`EVENT_CAPACITY`]
    /// 而丢帧(`RecvError::Lagged`)。由转发任务回灌进 broadcast 再透传给
    /// 前端;前端据此提示「任务状态可能不同步」,不做自动探活(ACP 无
    /// status RPC,方案审计裁决)。无 session_id:拥堵是桥级现象。
    QueueLagged {
        dropped: u64,
    },
    /// 回合中运行态用量(每次 model call 结束一帧,内核 prompt ledger
    /// 快照投影)。覆盖式语义:前端直接替换展示值,绝不累加;回合末权威
    /// `turn_usage` 到达后由前端清场。仅在线面板消费,不落盘不回放。
    UsageUpdate {
        session_id: String,
        input_tokens: u64,
        output_tokens: u64,
        model_calls: u64,
        num_turns: u64,
    },
}

/// `session/prompt` 图片块载荷(前端 composer 附件)。字段名走 camelCase,
/// 与 ACP `ImageContent` 线上格式一致(`mimeType`);`data` 为去前缀的 base64。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePayload {
    /// base64 编码(无 `data:<mime>;base64,` 前缀)。
    pub data: String,
    /// MIME 类型(image/png | image/jpeg | image/webp | image/gif)。
    pub mime_type: String,
}

#[derive(Debug, Clone)]
struct SessionRecord {
    id: String,
    cwd: PathBuf,
    /// 绑定的办公任务工作区名(session/new|load 透传的 `office_task`)。
    /// 留档用于 prompt 时前置产物登记硬指令(见 `decorate_prompt`)。
    office_task: Option<String>,
}

/// 在途权限请求的登记项。
#[derive(Debug, Clone)]
struct PermissionEntry {
    session_id: String,
    /// agent 给出的全部合法选项 id,回填时校验。
    option_ids: Vec<String>,
    /// 该请求里 `kind` 含 `reject` 的选项 id(`reject_once` / `reject_always`)。
    /// 超时按拒绝处理时用它回填;缺失则退化为 `cancelled`(绝不 allow)。
    reject_option: Option<String>,
    /// 截止时刻(落表时 = now + `permission_timeout`)。超时任务只有复核
    /// `deadline <= now` 才会动手,避免早醒误杀仍在有效期内的请求。
    deadline: Instant,
}

/// 在途 RPC 应答回传通道。
type PendingTx = oneshot::Sender<Result<serde_json::Value, (i64, String)>>;

pub struct AcpBridge {
    transport: Arc<dyn AgentTransport>,
    next_id: AtomicU64,
    initialized: AtomicBool,
    /// 我方在途请求:id → 应答回传。
    pending: Arc<Mutex<HashMap<u64, PendingTx>>>,
    /// 在途权限请求:id → 登记(会话 + 合法选项)。
    permission_pending: Arc<Mutex<HashMap<u64, PermissionEntry>>>,
    /// 单条权限请求的截止时间(默认 [`DEFAULT_PERMISSION_TIMEOUT`],测试可注入)。
    permission_timeout: Duration,
    event_tx: broadcast::Sender<BridgeEvent>,
    sessions: Mutex<Vec<SessionRecord>>,
    /// 每会话最近一次模型状态(内核应答原样缓存;查询走 model_state)。
    model_states: Mutex<HashMap<String, serde_json::Value>>,
}

/// 把办公任务绑定合入 params 的官方扩展通道 `_meta`(契约:`_meta.office_task`,
/// snake_case)。未绑定 → params 原样返回(不落键,与旧版逐字一致);已有
/// `_meta` 对象时**合入**而非覆盖(保留其它 `_meta` 键)。
/// 不放顶层:ACP 的 NewSessionRequest/LoadSessionRequest 为 camelCase +
/// `#[non_exhaustive]` 且无 `deny_unknown_fields`(agent-client-protocol-schema
/// 0.11.4 `agent.rs:905` / `:1078`),顶层未知键会被内核静默丢弃。
/// 取权限请求的合法选项 id 列表,供 [`AcpBridge::resolve_permission`] 校验。
///
/// 键必须是 ACP 线上格式的 `optionId`(agent-client-protocol-schema 0.11.4
/// `client.rs:584` 的 `PermissionOption` 为 `rename_all="camelCase"`,字段
/// `option_id`)。写成 `id` 会让本列表恒空,于是每次「允许」都被判为非法选项
/// ——错误只以中文提示一闪,界面上看像按钮失灵。
fn option_ids_of(params: &serde_json::Value) -> Vec<String> {
    params
        .get("options")
        .and_then(|o| o.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|o| o.get("optionId").and_then(|i| i.as_str()))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// 取权限请求里「拒绝」选项的 id,供超时按拒绝回填时使用。
///
/// 判定只看 `kind` 是否含 `reject`(`reject_once` / `reject_always`,以及
/// schema 后续可能新增的 reject 变体);与 [`option_ids_of`] 同样读线上
/// camelCase 的 `optionId`。没有 reject 选项 → `None`,超时改走 `cancelled`。
fn reject_option_of(params: &serde_json::Value) -> Option<String> {
    params
        .get("options")
        .and_then(|o| o.as_array())
        .and_then(|arr| {
            arr.iter().find_map(|o| {
                let kind = o.get("kind").and_then(|k| k.as_str())?;
                if !kind.to_ascii_lowercase().contains("reject") {
                    return None;
                }
                o.get("optionId").and_then(|i| i.as_str()).map(String::from)
            })
        })
}

/// 构造并发送一条权限结果应答(`result.outcome.{outcome,optionId}`)。
///
/// 抽成自由函数是因为超时任务只持有克隆出来的 `transport`(它借不到桥:
/// `handle_agent_request` 拿的是 `&self`,Arc 的强引用不能从 `&self` 复制)。
fn send_permission_outcome_on(
    transport: &dyn AgentTransport,
    request_id: u64,
    session_id: &str,
    outcome: &str,
    option_id: Option<&str>,
) -> Result<(), String> {
    let mut outcome_value = serde_json::json!({ "outcome": outcome });
    if let Some(oid) = option_id {
        outcome_value["optionId"] = serde_json::Value::String(oid.to_string());
    }
    let resp =
        jsonrpc::success_response(request_id, serde_json::json!({ "outcome": outcome_value }));
    transport
        .send(resp.to_string())
        .map_err(|e| format!("发送权限应答失败: {e}"))?;
    tracing::info!(request_id, %session_id, %outcome, ?option_id, "权限已回填");
    Ok(())
}

/// 解析 `_x.ai/usage_update` 扩展通知 params。线上键为 camelCase
/// (内核 emit_usage_update 投影),桥层转 snake_case 强类型元组供
/// [`BridgeEvent::UsageUpdate`] 使用;任一键缺失/类型不符 → `None`
/// (畸形帧不进前端,记日志丢弃)。
fn parse_usage_update(
    params: &serde_json::Value,
) -> Option<(String, u64, u64, u64, u64)> {
    let session_id = params.get("sessionId")?.as_str()?.to_string();
    let input_tokens = params.get("inputTokens")?.as_u64()?;
    let output_tokens = params.get("outputTokens")?.as_u64()?;
    let model_calls = params.get("modelCalls")?.as_u64()?;
    let num_turns = params.get("numTurns")?.as_u64()?;
    Some((
        session_id,
        input_tokens,
        output_tokens,
        model_calls,
        num_turns,
    ))
}

fn with_office_task_meta(
    mut params: serde_json::Value,
    office_task: Option<&str>,
) -> serde_json::Value {
    let Some(task) = office_task else {
        return params;
    };
    if task.trim().is_empty() {
        return params;
    }
    let Some(root) = params.as_object_mut() else {
        return params;
    };
    let meta = root.entry("_meta").or_insert_with(|| serde_json::json!({}));
    if !meta.is_object() {
        // 畸形 _meta(非对象):重建为空对象,避免静默丢绑定
        *meta = serde_json::json!({});
    }
    if let Some(meta_obj) = meta.as_object_mut() {
        meta_obj.insert(
            "office_task".to_string(),
            serde_json::Value::String(task.to_string()),
        );
    }
    params
}

/// `initialize` 请求参数(握手 + 客户端能力 + 扩展 `_meta`)。
///
/// `_meta.bufferingSettings`:内核仅在客户端 initialize 传入此项时才启用
/// chunk 合并——`acp_agent.rs` 读 `arguments.meta.bufferingSettings`(缺省即
/// "Buffering disabled: always send immediately")。wire 键名**必须 `_meta`**:
/// 第三方 schema crate 全局 `#[serde(rename = "_meta")]`,顶层未知字段会被
/// 静默丢弃(`session/new` 的 office_task 探针已实证此坑)。字段 camelCase
/// `maxItems`/`maxBytes`/`maxDurationMs`——对应
/// `update_chunk_merge::BufferingSettings`(`#[serde(rename_all = "camelCase")]`,
/// 默认 100/2048/10)。显式传值 = 与内核默认一致,激活合并同时固定行为。
/// 与 `clientCapabilities` 平级;未声明 fs / terminal 能力:agent 的文件
/// 操作走其自有工具(GUI 不经 ACP 提供客户端文件访问,缩小攻击面,方案 v2 §D4)。
fn initialize_params() -> serde_json::Value {
    serde_json::json!({
        "protocolVersion": ACP_PROTOCOL_VERSION,
        "clientCapabilities": {
            // 未声明 fs / terminal 能力(见上)
        },
        "_meta": {
            "bufferingSettings": {
                "maxItems": 100,
                "maxBytes": 2048,
                "maxDurationMs": 10
            }
        }
    })
}

impl AcpBridge {
    /// 挂到传输层:取走行接收端(唯一消费者)并启动路由循环。
    /// 返回的桥同时是事件源(`subscribe`)。
    pub fn attach(transport: Arc<dyn AgentTransport>) -> Result<Arc<Self>, TransportError> {
        Self::attach_with_timeout(transport, DEFAULT_PERMISSION_TIMEOUT)
    }

    /// 测试注入点:权限超时可控(公开 `attach` 签名保持不变)。
    #[cfg(test)]
    pub(crate) fn new_with_timeout(
        transport: Arc<dyn AgentTransport>,
        permission_timeout: Duration,
    ) -> Result<Arc<Self>, TransportError> {
        Self::attach_with_timeout(transport, permission_timeout)
    }

    fn attach_with_timeout(
        transport: Arc<dyn AgentTransport>,
        permission_timeout: Duration,
    ) -> Result<Arc<Self>, TransportError> {
        let line_rx = transport.take_line_receiver()?;
        let (event_tx, _) = broadcast::channel(EVENT_CAPACITY);
        let bridge = Arc::new(Self {
            transport,
            next_id: AtomicU64::new(1),
            initialized: AtomicBool::new(false),
            pending: Arc::new(Mutex::new(HashMap::new())),
            permission_pending: Arc::new(Mutex::new(HashMap::new())),
            permission_timeout,
            event_tx,
            sessions: Mutex::new(Vec::new()),
            model_states: Mutex::new(HashMap::new()),
        });

        let router = Arc::clone(&bridge);
        tokio::spawn(async move {
            router.route_loop(line_rx).await;
        });

        // 退出监控:agent 断开 → 通知订阅方 + 清理在途请求。
        let watcher = Arc::clone(&bridge);
        tokio::spawn(async move {
            let mut rx = watcher.transport.subscribe_exit();
            let info = loop {
                match rx.recv().await {
                    Ok(info) => break info,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            };
            watcher.on_exit(info);
        });

        Ok(bridge)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BridgeEvent> {
        self.event_tx.subscribe()
    }

    pub fn is_running(&self) -> bool {
        self.transport.is_running()
    }

    /// 注册时记录的会话(M3 恢复编排用)。
    /// 三元组含绑定的 office_task(reconnect 时原样带回 session/load)。
    pub fn sessions(&self) -> Vec<(String, PathBuf, Option<String>)> {
        self.sessions
            .lock()
            .map(|s| {
                s.iter()
                    .map(|r| (r.id.clone(), r.cwd.clone(), r.office_task.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn session_known(&self, session_id: &str) -> bool {
        self.sessions
            .lock()
            .map(|s| s.iter().any(|r| r.id == session_id))
            .unwrap_or(false)
    }

    /// 遗忘单个会话(未知/已删 → false):移除登记、模型缓存与待批权限,出锁后 best-effort 回 cancelled。
    pub fn forget_session(&self, session_id: &str) -> bool {
        let dropped = {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !sessions.iter().any(|r| r.id == session_id) {
                None
            } else {
                sessions.retain(|r| r.id != session_id);
                let _ = self
                    .model_states
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(session_id);
                let mut permissions = self
                    .permission_pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let ids: Vec<u64> = permissions
                    .iter()
                    .filter(|(_, entry)| entry.session_id == session_id)
                    .map(|(id, _)| *id)
                    .collect();
                for id in &ids {
                    let _ = permissions.remove(id);
                }
                Some(ids)
            }
        };
        let Some(ids) = dropped else {
            return false;
        };
        for id in ids {
            let resp = jsonrpc::success_response(
                id,
                serde_json::json!({"outcome": {"outcome": "cancelled"}}),
            );
            if let Err(e) = self.transport.send(resp.to_string()) {
                tracing::warn!(request_id = id, "已移除会话的权限取消应答发送失败(忽略): {e}");
            }
            let _ = self.event_tx.send(BridgeEvent::PermissionCancelled {
                session_id: session_id.to_string(),
                reason: "session_removed".to_string(),
            });
        }
        true
    }

    /// known 检查与发射同处一把 sessions 锁:forget 之后迟到的事件不再上屏。
    fn send_event_if_known(&self, session_id: &str, event: BridgeEvent) -> bool {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !sessions.iter().any(|r| r.id == session_id) {
            return false;
        }
        let _ = self.event_tx.send(event);
        true
    }

    /// 协议级断连收尾。顺序即契约:先收尾在途 RPC,再对挂起权限回
    /// `cancelled`,最后广播 Disconnected(k3 M3 审计 §2,防前端双结局)。
    /// 全同步(收尾只做 best-effort 发送),监控任务直接调用。
    fn on_exit(&self, info: ExitInfo) {
        // 1) 在途 RPC 全部按错误收尾
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for (_, tx) in pending.drain() {
            let _ = tx.send(Err((-1, "agent 已断开".into())));
        }
        drop(pending);
        // 2) 挂起权限逐项回 cancelled(best-effort:写失败即放弃)
        //    Trigger A (bridge restart): before the pending table is cleared,
        //    report every entry to the frontend as PermissionCancelled. Without
        //    it the frontend keeps stale queue entries whose ids no longer
        //    exist in the table, so every click fails silently (zombie dialog).
        //    The old request_id is NOT re-pushed (dead once disconnected).
        let mut permissions = self
            .permission_pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for (id, entry) in permissions.iter() {
            let resp = jsonrpc::success_response(
                *id,
                serde_json::json!({"outcome": {"outcome": "cancelled"}}),
            );
            let _ = self.transport.send(resp.to_string());
            let _ = self.event_tx.send(BridgeEvent::PermissionCancelled {
                session_id: entry.session_id.clone(),
                reason: "bridge_restart".to_string(),
            });
        }
        permissions.clear();
        drop(permissions);
        // 3) 最后广播断连
        let _ = self.event_tx.send(BridgeEvent::Disconnected {
            code: info.code,
            success: info.success,
        });
    }

    /// 路由循环:入站行 → 应答关联 / 事件 / 防御性拒绝。
    /// 循环内 **绝不 await 写队列**(k3 M3 审计 §1:写队列满会冻结全部
    /// 入站分发)——需要写时走 `send_best_effort`。
    async fn route_loop(&self, mut line_rx: mpsc::Receiver<String>) {
        while let Some(line) = line_rx.recv().await {
            let Some(incoming) = jsonrpc::parse_incoming(&line) else {
                tracing::warn!(line = %line, "无法解析的入站行,忽略");
                continue;
            };
            match incoming {
                Incoming::Response { id, result } => {
                    let tx = self
                        .pending
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .remove(&id);
                    match tx {
                        Some(tx) => {
                            let _ = tx.send(result);
                        }
                        None => tracing::debug!(id, "收到无主应答(对端超时后晚到),忽略"),
                    }
                }
                Incoming::Notification { method, params } if method == "session/update" => {
                    let session_id = params
                        .get("sessionId")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    // 未知会话的更新不入前端(防串话,k3 M3 审计 §3)。
                    // 注:session/load 的历史重放发生在登记前,同样被丢弃——
                    // 恢复场景前端以 SessionRestored 为准重拉状态。
                    let update = params
                        .get("update")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    if !self.send_event_if_known(
                        &session_id,
                        BridgeEvent::SessionUpdate {
                            session_id: session_id.clone(),
                            update,
                        },
                    ) {
                        tracing::warn!(%session_id, "未知会话的 update,丢弃");
                    }
                }
                Incoming::Notification { method, params }
                    if method == "x.ai/session_notification" =>
                {
                    // 模型切换回显(update.model_changed):同步缓存并广播,
                    // 前端状态栏随其他客户端/内核内部的切换刷新。
                    let session_id = params
                        .get("sessionId")
                        .and_then(|s| s.as_str())
                        .unwrap_or("");
                    let update = params.get("update");
                    let is_model_changed = update
                        .and_then(|u| u.get("sessionUpdate"))
                        .and_then(|t| t.as_str())
                        == Some("model_changed");
                    if is_model_changed {
                        if let Some(mid) =
                            update.and_then(|u| u.get("model_id")).and_then(|m| m.as_str())
                        {
                            // 回显通知不携带档位变更的权威值(发起方自身被内核门控),
                            // 档位由本端 set_model 就地 patch,故此处 effort 传 None。
                            if self
                                .patch_current_model_if_known(session_id, mid, None)
                                .is_none()
                            {
                                tracing::warn!(%session_id, "已移除会话的模型回显,忽略");
                            }
                        }
                    }
                }
                Incoming::Notification { method, params }
                    if method == "_x.ai/usage_update" =>
                {
                    // 回合中运行态用量:纯转发(瞬态帧,不入库不回放)。
                    // 未知会话直接丢,与 session/update 同语义(防串话)。
                    if let Some((
                        session_id,
                        input_tokens,
                        output_tokens,
                        model_calls,
                        num_turns,
                    )) = parse_usage_update(&params)
                    {
                        let known = self.send_event_if_known(
                            &session_id,
                            BridgeEvent::UsageUpdate {
                                session_id: session_id.clone(),
                                input_tokens,
                                output_tokens,
                                model_calls,
                                num_turns,
                            },
                        );
                        if !known {
                            tracing::debug!(%session_id, "未知会话的 usage_update,丢弃");
                        }
                    } else {
                        tracing::debug!(?params, "畸形的 x.ai/usage_update,丢弃");
                    }
                }
                Incoming::Request { id, method, params } => {
                    self.handle_agent_request(id, method, params);
                }
                Incoming::Notification { method, params } => {
                    tracing::debug!(method, ?params, "未处理的 agent 通知,忽略");
                }
            }
        }
        // line_rx 关闭:transport 被弃(M2 kill_on_drop 语义),无需动作。
    }

    /// agent 请求:权限审批走事件回路;fs/terminal 未在 initialize 声明,
    /// 规范上不应出现,出现则防御性拒绝(best-effort,不阻塞路由)。
    fn handle_agent_request(&self, id: u64, method: String, params: serde_json::Value) {
        if method == "session/request_permission" {
            let session_id = params
                .get("sessionId")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let option_ids = option_ids_of(&params);
            let reject_option = reject_option_of(&params);
            let deadline = Instant::now() + self.permission_timeout;
            // known 检查、在途登记与发射同处一锁:删除后不得再弹过期审批。
            let accepted = {
                let sessions = self
                    .sessions
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if !sessions.iter().any(|r| r.id == session_id) {
                    false
                } else {
                    let _ = self
                        .permission_pending
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .insert(
                            id,
                            PermissionEntry {
                                session_id: session_id.clone(),
                                option_ids,
                                reject_option,
                                deadline,
                            },
                        );
                    let _ = self.event_tx.send(BridgeEvent::PermissionRequest {
                        request_id: id,
                        session_id: session_id.clone(),
                        tool_call: params
                            .get("toolCall")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                        options: params
                            .get("options")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                    });
                    true
                }
            };
            if !accepted {
                tracing::warn!(%session_id, "未知会话的权限请求,拒绝");
                self.send_best_effort(jsonrpc::error_response(id, -32602, "未知会话"));
                return;
            }
            // 落表成功才挂超时任务:未登记的请求已就地拒绝,无需截止时间。
            self.spawn_permission_timeout(id, deadline);
            return;
        }
        tracing::warn!(method = %method, "agent 请求了未声明的能力,拒绝");
        self.send_best_effort(jsonrpc::error_response(
            id,
            ERR_METHOD_NOT_FOUND,
            &format!("未支持的方法: {method}"),
        ));
    }

    /// 后台尽力发送:路由循环与断连收尾专用(写队列满仅记日志)。
    fn send_best_effort(&self, msg: serde_json::Value) {
        let transport = Arc::clone(&self.transport);
        tokio::spawn(async move {
            if let Err(e) = transport.send(msg.to_string()) {
                tracing::warn!("best-effort 发送失败(忽略): {e}");
            }
        });
    }

    /// 前端回填权限选择。optionId 必须是 agent 给出的合法选项,否则 Err
    /// 且条目保留(可重试);成功后条目移除,重复响应报错。
    pub fn resolve_permission(&self, request_id: u64, option_id: &str) -> Result<(), String> {
        let mut pending = self
            .permission_pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(entry) = pending.get(&request_id) else {
            return Err(format!("权限请求 {request_id} 不存在或已响应"));
        };
        if !entry.option_ids.iter().any(|o| o == option_id) {
            return Err(format!(
                "非法选项 {option_id},合法选项:{:?}",
                entry.option_ids
            ));
        }
        let Some(PermissionEntry { session_id, .. }) = pending.remove(&request_id) else {
            unreachable!("上一行 get 刚确认存在,无并发写者");
        };
        drop(pending);
        self.send_permission_outcome(request_id, &session_id, "selected", Some(option_id))
    }

    /// 用户取消权限对话框(前端关弹窗场景,k3 M3 审计 §3-P2)。
    pub fn cancel_permission(&self, request_id: u64) -> Result<(), String> {
        let entry = self
            .permission_pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&request_id)
            .ok_or_else(|| format!("权限请求 {request_id} 不存在或已响应"))?;
        self.send_permission_outcome(request_id, &entry.session_id, "cancelled", None)
    }

    /// Trigger B (turn-terminal linkage), called by the event forwarding loops
    /// in commands.rs when a turn-terminal event for `session_id` is forwarded:
    /// drop every pending permission entry of that session and report each one
    /// to the frontend as [`BridgeEvent::PermissionCancelled`]. Once the turn is
    /// over the kernel cancels its own pending interactions, so those requests
    /// can never be answered - the ids are deliberately NOT re-pushed.
    /// No response is written back to the agent: the turn already ended.
    /// Returns the number of dropped entries (0 = nothing to do).
    pub fn expire_permissions_for_session(&self, session_id: &str, reason: &str) -> usize {
        let dropped = {
            let mut guard = self
                .permission_pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let ids: Vec<u64> = guard
                .iter()
                .filter(|(_, entry)| entry.session_id == session_id)
                .map(|(id, _)| *id)
                .collect();
            for id in &ids {
                guard.remove(id);
            }
            ids.len()
        };
        for _ in 0..dropped {
            let _ = self.event_tx.send(BridgeEvent::PermissionCancelled {
                session_id: session_id.to_string(),
                reason: reason.to_string(),
            });
        }
        dropped
    }

    /// 为一条已落表的权限请求挂超时任务:到期仍 pending → 按**拒绝**回填
    /// (无 reject 选项则走 `cancelled`),并广播 `PermissionCancelled{reason:"timeout"}`。
    ///
    /// **绝不自动 allow**。与其他三方(`resolve_permission` / `cancel_permission` /
    /// `expire_permissions_for_session`)共用 `permission_pending` 锁:先到者移除
    /// 条目,本任务到期加锁时查无此项即静默退出(先到者生效,后到者 no-op)。
    /// 不保存 JoinHandle:任务自行 no-op 结束;`transport` 只持 `Weak`,
    /// 超时任务绝不延长传输层寿命(kill_on_drop:桥弃则进程可即刻回收)。
    fn spawn_permission_timeout(&self, request_id: u64, deadline: Instant) {
        let pending = Arc::clone(&self.permission_pending);
        let transport = Arc::downgrade(&self.transport);
        let event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            // 复核 + 摘除同处一锁:期间若被回填/取消/清表,条目已不在 → 不动手。
            let entry = {
                let mut guard = pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let due = guard
                    .get(&request_id)
                    .map(|entry| entry.deadline <= Instant::now())
                    .unwrap_or(false);
                if due {
                    guard.remove(&request_id)
                } else {
                    None
                }
            };
            let Some(entry) = entry else {
                return;
            };
            tracing::warn!(
                request_id,
                session_id = %entry.session_id,
                "权限请求超时,按拒绝处理"
            );
            // 有 reject 选项 → selected(reject);否则 cancelled。两条都不是 allow。
            let (outcome, option_id) = match entry.reject_option.as_deref() {
                Some(reject) => ("selected", Some(reject)),
                None => ("cancelled", None),
            };
            match transport.upgrade() {
                Some(transport) => {
                    if let Err(e) = send_permission_outcome_on(
                        transport.as_ref(),
                        request_id,
                        &entry.session_id,
                        outcome,
                        option_id,
                    ) {
                        tracing::warn!(request_id, "权限超时应答发送失败(忽略): {e}");
                    }
                }
                // 桥/进程已销毁:无对象可应答,只剩清表与前端通知。
                None => {
                    tracing::warn!(request_id, "权限超时但 transport 已弃,跳过应答");
                }
            }
            let _ = event_tx.send(BridgeEvent::PermissionCancelled {
                session_id: entry.session_id,
                reason: "timeout".to_string(),
            });
        });
    }

    fn send_permission_outcome(
        &self,
        request_id: u64,
        session_id: &str,
        outcome: &str,
        option_id: Option<&str>,
    ) -> Result<(), String> {
        send_permission_outcome_on(
            self.transport.as_ref(),
            request_id,
            session_id,
            outcome,
            option_id,
        )
    }

    /// 发起一个 RPC 并等待应答(带请求级超时)。
    async fn rpc(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, tx);

        let msg = jsonrpc::request(id, method, params);
        // known 检查与发送同处一锁(出锁后才 await 应答):通过检查后不得再被删除。
        let gate_session = if method == "session/set_model" {
            msg.get("params")
                .and_then(|p| p.get("sessionId"))
                .and_then(|s| s.as_str())
                .map(str::to_string)
        } else {
            None
        };
        let send_result = match gate_session {
            Some(session_id) => {
                let sessions = self
                    .sessions
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let known = sessions.iter().any(|r| r.id == session_id);
                if known {
                    let result = self.transport.send(msg.to_string());
                    drop(sessions);
                    result
                } else {
                    drop(sessions);
                    self.pending
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .remove(&id);
                    return Err(format!("会话 {session_id} 未登记或已删除"));
                }
            }
            None => self.transport.send(msg.to_string()),
        };
        if let Err(e) = send_result {
            // 发送失败必须清掉在途项:transport 半死时 on_exit 不会来,
            // 残留即泄漏(k3 M3 审计 §1)。
            self.pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&id);
            return Err(format!("发送 {method} 失败: {e}"));
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(result))) => Ok(result),
            Ok(Ok(Err((code, message)))) => Err(format!("{method} 失败 [{code}]: {message}")),
            Ok(Err(_recv)) => Err(format!("{method} 应答通道关闭(agent 断开)")),
            Err(_) => {
                self.pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&id);
                Err(format!("{method} 超时({timeout:?})"))
            }
        }
    }

    /// 协议握手 + 版本协商(k3 M3 审计 §4:agent 升级协议属静默错误源)。
    async fn initialize(&self) -> Result<serde_json::Value, String> {
        let params = initialize_params();
        let result = self.rpc("initialize", params, RPC_TIMEOUT).await?;
        let version = result.get("protocolVersion").and_then(|v| v.as_u64());
        if version != Some(ACP_PROTOCOL_VERSION as u64) {
            return Err(format!(
                "ACP 协议版本不匹配:期望 {ACP_PROTOCOL_VERSION},agent 报告 {version:?}"
            ));
        }
        Ok(result)
    }

    /// 幂等初始化:已成功过 initialize 则直接返回(重 spawn 的新桥才会
    /// 真正执行协议握手;版本校验失败不置位)。
    pub async fn ensure_initialized(&self) -> Result<(), String> {
        if self.initialized.load(Ordering::Relaxed) {
            return Ok(());
        }
        self.initialize().await?;
        self.initialized.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub async fn new_session(
        &self,
        cwd: PathBuf,
        office_task: Option<String>,
    ) -> Result<String, String> {
        // 办公任务绑定透传:官方扩展通道 _meta.office_task(仅绑定时落键;
        // 顶层不放——内核 camelCase 结构体会静默丢弃顶层未知键)。
        let params = with_office_task_meta(
            serde_json::json!({"cwd": cwd.to_string_lossy(), "mcpServers": []}),
            office_task.as_deref(),
        );
        let result = self.rpc("session/new", params, RPC_TIMEOUT).await?;
        let session_id = result
            .get("sessionId")
            .and_then(|s| s.as_str())
            .ok_or_else(|| format!("session/new 应答缺少 sessionId: {result}"))?
            .to_string();
        self.cache_model_state(&session_id, &result);
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(SessionRecord {
                id: session_id.clone(),
                cwd,
                office_task,
            });
        Ok(session_id)
    }

    /// 恢复既有会话(重 spawn 后)。成功即重新登记。
    /// 注:load 期间的历史重放(session/update)发生在登记前,会被路由
    /// 循环按未知会话丢弃——恢复场景前端以 SessionRestored 为准。
    pub async fn load_session(
        &self,
        session_id: &str,
        cwd: PathBuf,
        office_task: Option<String>,
    ) -> Result<(), String> {
        // 与 session/new 同一契约:绑定的办公任务工作区名走 _meta.office_task
        // (未绑定不落键;已有 _meta 时合入,不覆盖其它键)。
        let params = with_office_task_meta(
            serde_json::json!({
                "sessionId": session_id,
                "cwd": cwd.to_string_lossy(),
                "mcpServers": []
            }),
            office_task.as_deref(),
        );
        self.rpc("session/load", params, RPC_TIMEOUT)
            .await
            .map(|result| self.cache_model_state(session_id, &result))?;
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(SessionRecord {
                id: session_id.to_string(),
                cwd,
                office_task,
            });
        Ok(())
    }

    /// 单会话恢复完成通知(session_resume 用):事件经 ensure_bridge 的
    /// 转发链到前端,前端切 active sessionId。
    /// 会话已删除时不发射(否则前端会把刚删掉的会话切回来)。
    pub fn notify_session_restored(&self, session_id: &str) {
        self.send_event_if_known(
            session_id,
            BridgeEvent::SessionRestored {
                session_id: session_id.to_string(),
            },
        );
    }

    /// 补发会话恢复失败事件(恢复链订阅建立后使用,见 commands.rs agent_recover;
    /// k3 交叉审计 W3:reconnect 内的原发射发生在订阅前,会被 broadcast 丢弃)。
    pub fn notify_session_restore_failed(&self, session_id: &str, error: &str) {
        let _ = self.event_tx.send(BridgeEvent::SessionRestoreFailed {
            session_id: session_id.to_string(),
            error: error.to_string(),
        });
    }

    /// 展示流拥堵告警回灌(转发任务遇 `RecvError::Lagged` 时调用):把
    /// [`BridgeEvent::QueueLagged`] 发进同一 broadcast,下一轮被转发任务
    /// 取出并透传给前端。`broadcast::Sender::send` 非阻塞(容量满只丢最旧
    /// 帧、不阻塞、不借出 receiver),故无死锁风险。
    pub fn notify_queue_lagged(&self, dropped: u64) {
        let _ = self.event_tx.send(BridgeEvent::QueueLagged { dropped });
    }

    /// 会话绑定的办公任务工作区名(未登记/未绑定 → None)。
    fn office_task_of(&self, session_id: &str) -> Option<String> {
        let guard = self.sessions.lock().ok()?;
        guard
            .iter()
            .find(|r| r.id == session_id)
            .and_then(|r| r.office_task.clone())
    }

    /// prompt 拼装:绑定会话前置一行产物登记硬指令(注入点 = prompt 组装处,
    /// 与 ACP session/prompt 的文本块同路下发,不进前端可见转录)。
    /// 未绑定会话原文返回(与旧版逐字一致)。
    fn decorate_prompt(&self, session_id: &str, text: &str) -> String {
        match self.office_task_of(session_id) {
            Some(task) => {
                // Audit hardening: strip quotes/control chars so a workspace
                // name cannot break the directive structure.
                let clean: String = task
                    .chars()
                    .filter(|c| *c != '"' && !c.is_control())
                    .collect();
                if clean.is_empty() {
                    return text.to_string();
                }
                format!(
                "[系统指令] 本次会话产物登记一律调用 card.py 时使用 --task \"{clean}\" 参数。\n{text}"
                            )
            }
            None => text.to_string(),
        }
    }

    /// 发起回合:立即返回,回合完成经 [`BridgeEvent::TurnCompleted`] 通知
    /// (GUI 调用不应阻塞数分钟)。`self: &Arc<Self>` 以便等待任务克隆桥。
    /// 注意:30 分钟超时 ≠ 取消——超时只发错误事件,agent 仍在跑,
    /// 真正中断需 `cancel`(M4 编排)。
    pub fn prompt(
        self: &Arc<Self>,
        session_id: &str,
        text: &str,
        images: &[ImagePayload],
    ) -> Result<(), String> {
        // decorate_prompt 内部取 sessions 锁,须在下面的临界区之前完成。
        let text = self.decorate_prompt(session_id, text);
        let blocks = build_prompt_blocks(&text, images)?;
        let params = serde_json::json!({
            "sessionId": session_id,
            "prompt": blocks
        });
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            // known 检查、在途登记与发送同处一锁:否则通过检查后被 forget,请求仍会下发。
            let sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !sessions.iter().any(|r| r.id == session_id) {
                return Err(format!("会话 {session_id} 未登记或已删除"));
            }
            self.pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(id, tx);

            let msg = jsonrpc::request(id, "session/prompt", params);
            if let Err(e) = self.transport.send(msg.to_string()) {
                self.pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&id);
                return Err(format!("发送 session/prompt 失败: {e}"));
            }
        }

        let this = Arc::clone(self);
        let session = session_id.to_string();
        tokio::spawn(async move {
            match tokio::time::timeout(PROMPT_TIMEOUT, rx).await {
                Ok(Ok(Ok(result))) => {
                    let stop = result
                        .get("stopReason")
                        .and_then(|s| s.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    // 先 usage 后 completed:前端在 completed 时把用量挂到
                    // 刚完成的 assistant 消息并累计(见 useAgent.ts)。
                    // _meta 无 schema 上限,畸形内核可塞出巨帧:超限丢弃
                    // (防御纵深,k3 审计 W2,GUI 用量显示降级为缺失)
                    let meta = result
                        .get("_meta")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    if !meta.is_null() {
                        if serde_json::to_string(&meta).map_or(false, |s| s.len() <= 64 * 1024) {
                            this.send_event_if_known(
                                &session,
                                BridgeEvent::TurnUsage {
                                    session_id: session.clone(),
                                    meta,
                                },
                            );
                        } else {
                            tracing::warn!(%session, "session/prompt _meta 超过 64KB,丢弃用量透传");
                        }
                    }
                    this.send_event_if_known(
                        &session,
                        BridgeEvent::TurnCompleted {
                            session_id: session.clone(),
                            stop_reason: stop,
                        },
                    );
                }
                Ok(Ok(Err((code, message)))) => {
                    tracing::error!(%session, code, %message, "session/prompt 失败");
                    this.send_event_if_known(
                        &session,
                        BridgeEvent::TurnCompleted {
                            session_id: session.clone(),
                            stop_reason: format!("error: {code} {message}"),
                        },
                    );
                }
                Ok(Err(_)) | Err(_) => {
                    tracing::error!(%session, "session/prompt 超时或断开");
                    this.send_event_if_known(
                        &session,
                        BridgeEvent::TurnCompleted {
                            session_id: session.clone(),
                            stop_reason: "error: timeout/disconnected".into(),
                        },
                    );
                }
            }
        });
        Ok(())
    }

    pub fn cancel(&self, session_id: &str) -> Result<(), String> {
        let note = jsonrpc::notification(
            "session/cancel",
            serde_json::json!({"sessionId": session_id}),
        );
        self.transport
            .send(note.to_string())
            .map_err(|e| format!("发送 session/cancel 失败: {e}"))
    }

    /// 从 session/new|load 应答缓存 models 字段并透传给前端(缺字段=旧内核
    /// 或未启用该 ACP 不稳定面,静默跳过,前端降级用到合模型上报)。
    fn cache_model_state(&self, session_id: &str, result: &serde_json::Value) {
        let Some(state) = result.get("models") else {
            return;
        };
        if let Ok(mut m) = self.model_states.lock() {
            m.insert(session_id.to_string(), state.clone());
        }
        let _ = self.event_tx.send(BridgeEvent::ModelState {
            session_id: session_id.to_string(),
            state: state.clone(),
        });
    }

    /// 把 currentModelId 更新为 model_id 并广播;无缓存时以最小状态建一条
    /// (availableModels 为空,前端仅展示 id)。`effort` 为 Some 时一并把档位
    /// 写进该模型 meta(内核 set_model 应答只回 model id、不回档位,强度下拉
    /// 据缓存刷新)。返回更新后的快照;已删除(未登记)→ None,不写缓存不发射。
    fn patch_current_model_if_known(
        &self,
        session_id: &str,
        model_id: &str,
        effort: Option<&str>,
    ) -> Option<serde_json::Value> {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !sessions.iter().any(|r| r.id == session_id) {
            return None;
        }
        let state = {
            let mut guard = self
                .model_states
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let entry = guard
                .entry(session_id.to_string())
                .or_insert_with(|| serde_json::json!({"availableModels": []}));
            if let Some(obj) = entry.as_object_mut() {
                obj.insert(
                    "currentModelId".to_string(),
                    serde_json::Value::String(model_id.to_string()),
                );
                if let Some(effort) = effort {
                    patch_model_effort(obj, model_id, effort);
                }
            }
            entry.clone()
        };
        let _ = self.event_tx.send(BridgeEvent::ModelState {
            session_id: session_id.to_string(),
            state: state.clone(),
        });
        Some(state)
    }

    /// 最近一次缓存的会话模型状态(无则 None;由 session_models 命令查询)。
    pub fn model_state(&self, session_id: &str) -> Option<serde_json::Value> {
        self.model_states
            .lock()
            .ok()
            .and_then(|m| m.get(session_id).cloned())
    }

    /// 热切换会话模型(session/set_model,同会话立即生效,不重启内核)。
    /// `effort`:可选思考强度档位,经官方扩展通道 `_meta.reasoningEffort` 下发
    /// (键名与内核 `model_switch.rs` 读取一致);仅 `Some` 时落键,`None` 时
    /// params 与旧版逐字一致(不落 `_meta`)。
    pub async fn set_model(
        &self,
        session_id: &str,
        model_id: &str,
        effort: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        if !self.session_known(session_id) {
            return Err(format!("会话 {session_id} 未登记或已删除"));
        }
        let mut params = serde_json::json!({"sessionId": session_id, "modelId": model_id});
        if let Some(effort) = effort {
            params["_meta"] = serde_json::json!({ "reasoningEffort": effort });
        }
        self.rpc("session/set_model", params, RPC_TIMEOUT).await?;
        // 应答后复查登记:等待期间被 forget 的会话不得因迟到应答重建缓存。
        self.patch_current_model_if_known(session_id, model_id, effort)
            .ok_or_else(|| format!("会话 {session_id} 已删除,丢弃迟到的模型应答"))
    }
}

/// session/prompt 图片 MIME 白名单(与内核图片处理面一致;其余一律拒绝)。
const ALLOWED_IMAGE_MIMES: [&str; 4] =
    ["image/png", "image/jpeg", "image/webp", "image/gif"];
/// 单张图片解码后字节上限(前端已按 10MB 校验,此处防御纵深)。
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// 组装 `session/prompt` 的 prompt 块数组(ACP ContentBlock 线上格式):
/// 文本块在前(`text` 非空时),图片块随后(每图 `{"type":"image","data","mimeType"}`)。
/// 文本为空且无图片 → Err(空 prompt 无意义);MIME 非白名单 / 单图超限 → Err。
fn build_prompt_blocks(
    text: &str,
    images: &[ImagePayload],
) -> Result<Vec<serde_json::Value>, String> {
    let mut blocks: Vec<serde_json::Value> = Vec::new();
    if !text.is_empty() {
        blocks.push(serde_json::json!({"type": "text", "text": text}));
    }
    for img in images {
        if !ALLOWED_IMAGE_MIMES.contains(&img.mime_type.as_str()) {
            return Err(format!(
                "不支持的图片类型 {}:仅支持 png/jpeg/webp/gif",
                img.mime_type
            ));
        }
        // base64 长度 → 近似解码字节数(4 字符 ≈ 3 字节),超限拒绝。
        if img.data.len().saturating_mul(3) / 4 > MAX_IMAGE_BYTES {
            return Err(format!(
                "图片超过 {}MB 上限",
                MAX_IMAGE_BYTES / (1024 * 1024)
            ));
        }
        blocks.push(serde_json::json!({
            "type": "image",
            "data": img.data,
            "mimeType": img.mime_type
        }));
    }
    if blocks.is_empty() {
        return Err("prompt 为空:需提供文本或至少一张图片".to_string());
    }
    Ok(blocks)
}

/// 把档位写进缓存状态里 `model_id` 对应模型的 `_meta.reasoningEffort`(仅当该
/// 模型已声明 `supportsReasoningEffort` 时;找不到模型 / 不支持则原样不改)。
fn patch_model_effort(
    state: &mut serde_json::Map<String, serde_json::Value>,
    model_id: &str,
    effort: &str,
) {
    let Some(models) = state.get_mut("availableModels").and_then(|v| v.as_array_mut()) else {
        return;
    };
    for model in models.iter_mut() {
        let Some(model) = model.as_object_mut() else {
            continue;
        };
        if model.get("modelId").and_then(|v| v.as_str()) != Some(model_id) {
            continue;
        }
        let supports = model
            .get("_meta")
            .and_then(|v| v.get("supportsReasoningEffort"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !supports {
            return;
        }
        let meta = model
            .entry("_meta")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(meta) = meta.as_object_mut() {
            meta.insert(
                "reasoningEffort".to_string(),
                serde_json::Value::String(effort.to_string()),
            );
        }
        return;
    }
}

/// 断开后的重 spawn 编排(M3 崩溃恢复,方案 v2 §D2):
/// spawn 新 agent → initialize → 逐会话 session/load;
/// 加载失败的单会话经 `SessionRestoreFailed` 上报,不拖垮其余会话。
/// 返回 (新桥, 恢复成功的会话 id, 失败清单 [(id, 错误)])。
pub async fn reconnect(
    transport: Arc<dyn AgentTransport>,
    sessions: Vec<(String, PathBuf, Option<String>)>,
) -> Result<(Arc<AcpBridge>, Vec<String>, Vec<(String, String)>), String> {
    let bridge = AcpBridge::attach(transport).map_err(|e| e.to_string())?;
    bridge.initialize().await?;
    bridge.initialized.store(true, Ordering::Relaxed);
    let mut restored = Vec::new();
    let mut failed = Vec::new();
    for (id, cwd, office_task) in sessions {
        match bridge.load_session(&id, cwd, office_task).await {
            Ok(()) => {
                let _ = bridge.event_tx.send(BridgeEvent::SessionRestored {
                    session_id: id.clone(),
                });
                restored.push(id);
            }
            Err(e) => {
                let _ = bridge.event_tx.send(BridgeEvent::SessionRestoreFailed {
                    session_id: id.clone(),
                    error: e.clone(),
                });
                failed.push((id, e));
            }
        }
    }
    Ok((bridge, restored, failed))
}

/// TransportError 的显示便捷转换(供 commands 层 map)。
impl From<TransportError> for String {
    fn from(e: TransportError) -> Self {
        e.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 线上形状(ACP camelCase `optionId`):两个合法选项都要取到。
    #[test]
    fn option_ids_read_the_camel_case_wire_key() {
        let params = serde_json::json!({
            "sessionId": "s",
            "options": [
                {"optionId": "allow-once", "name": "Allow once", "kind": "allow_once"},
                {"optionId": "reject-once", "name": "Reject once", "kind": "reject_once"}
            ]
        });
        assert_eq!(
            option_ids_of(&params),
            vec!["allow-once".to_string(), "reject-once".to_string()]
        );
    }

    /// 回归锁定:曾按 `id` 取值,列表恒空 → 每次点「允许」都判非法选项,
    /// 界面表现为按钮失灵。缺 options / 空数组同样得空列表。
    #[test]
    fn option_ids_ignore_the_wrong_key() {
        let wrong = serde_json::json!({
            "options": [{"id": "allow", "name": "允许", "kind": "allow_once"}]
        });
        assert!(option_ids_of(&wrong).is_empty());
        assert!(option_ids_of(&serde_json::json!({"sessionId": "s"})).is_empty());
        assert!(option_ids_of(&serde_json::json!({"options": []})).is_empty());
    }

    /// 绑定会话:office_task 落在 `_meta` 通道,顶层不落键(内核会丢弃顶层未知键)。
    #[test]
    fn office_task_goes_into_meta_not_top_level() {
        let params = with_office_task_meta(
            serde_json::json!({"cwd": "C:\\ws", "mcpServers": []}),
            Some("写标书"),
        );
        assert_eq!(
            params,
            serde_json::json!({
                "cwd": "C:\\ws",
                "mcpServers": [],
                "_meta": {"office_task": "写标书"}
            })
        );
        assert!(
            params.get("office_task").is_none(),
            "顶层不得出现 office_task: {params}"
        );
    }

    /// 未绑定:params 与旧版逐字一致(无 _meta 键)。
    #[test]
    fn unbound_params_carry_no_meta_key() {
        let params =
            with_office_task_meta(serde_json::json!({"cwd": "C:\\ws", "mcpServers": []}), None);
        assert_eq!(params, serde_json::json!({"cwd": "C:\\ws", "mcpServers": []}));
        assert!(params.get("_meta").is_none());
    }

    /// 已有 _meta 时合入:保留其它键,只新增 office_task(不覆盖)。
    #[test]
    fn office_task_merges_into_existing_meta() {
        let params = with_office_task_meta(
            serde_json::json!({"cwd": "C:\\ws", "_meta": {"keep": 1}}),
            Some("周报"),
        );
        assert_eq!(params["_meta"]["keep"], serde_json::json!(1));
        assert_eq!(params["_meta"]["office_task"], serde_json::json!("周报"));
    }

    /// P0-1:initialize params 携带 `_meta.bufferingSettings`(激活内核 chunk
    /// 合并;camelCase 三键齐全,平级于 clientCapabilities 而非顶层)。
    #[test]
    fn initialize_params_activate_buffering() {
        let params = initialize_params();
        // wire 键名必须是 `_meta`(顶层未知字段会被内核静默丢弃)
        let bs = &params["_meta"]["bufferingSettings"];
        assert_eq!(bs["maxItems"], serde_json::json!(100), "params={params}");
        assert_eq!(bs["maxBytes"], serde_json::json!(2048), "params={params}");
        assert_eq!(bs["maxDurationMs"], serde_json::json!(10), "params={params}");
        // 与 clientCapabilities 平级,且 protocolVersion 仍在顶层
        assert_eq!(
            params["protocolVersion"],
            serde_json::json!(ACP_PROTOCOL_VERSION)
        );
        assert!(params.get("clientCapabilities").is_some());
        // bufferingSettings 不得落在顶层(内核只认 _meta 通道)
        assert!(
            params.get("bufferingSettings").is_none(),
            "顶层不得出现 bufferingSettings: {params}"
        );
    }

    /// P0-2:QueueLagged 的 serde 形状(前端按 `type` tag + `dropped` 解析)。
    #[test]
    fn queue_lagged_serializes_to_type_tag() {
        let v = serde_json::to_value(BridgeEvent::QueueLagged { dropped: 7 }).unwrap();
        assert_eq!(v, serde_json::json!({"type": "queue_lagged", "dropped": 7}));
    }

    /// PermissionCancelled serde shape: the frontend parses the `type` tag plus
    /// `session_id` / `reason`; the bridge_restart and turn_ended triggers share
    /// this exact shape.
    #[test]
    fn permission_cancelled_serializes_to_type_tag() {
        let v = serde_json::to_value(BridgeEvent::PermissionCancelled {
            session_id: "sess-1".to_string(),
            reason: "bridge_restart".to_string(),
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "type": "permission_cancelled",
                "session_id": "sess-1",
                "reason": "bridge_restart"
            })
        );
    }

    /// The turn_ended trigger uses the same shape with a different reason.
    #[test]
    fn permission_cancelled_turn_ended_reason() {
        let v = serde_json::to_value(BridgeEvent::PermissionCancelled {
            session_id: "sess-2".to_string(),
            reason: "turn_ended".to_string(),
        })
        .unwrap();
        assert_eq!(v["type"], serde_json::json!("permission_cancelled"));
        assert_eq!(v["session_id"], serde_json::json!("sess-2"));
        assert_eq!(v["reason"], serde_json::json!("turn_ended"));
    }

    fn img(data: &str, mime: &str) -> ImagePayload {
        ImagePayload {
            data: data.to_string(),
            mime_type: mime.to_string(),
        }
    }

    /// 纯文本:单文本块(与旧版逐字一致)。
    #[test]
    fn prompt_blocks_text_only() {
        let blocks = build_prompt_blocks("写周报", &[]).unwrap();
        assert_eq!(blocks, vec![serde_json::json!({"type": "text", "text": "写周报"})]);
    }

    /// 纯图(空文本):仅图片块,camelCase `mimeType`。
    #[test]
    fn prompt_blocks_image_only() {
        let images = vec![img("QUJD", "image/png")];
        let blocks = build_prompt_blocks("", &images).unwrap();
        assert_eq!(
            blocks,
            vec![serde_json::json!({"type": "image", "data": "QUJD", "mimeType": "image/png"})]
        );
    }

    /// 图文混合:文本块在前,图片块随后(顺序即契约)。
    #[test]
    fn prompt_blocks_mixed_text_then_images() {
        let images = vec![img("QUJD", "image/png"), img("REVG", "image/jpeg")];
        let blocks = build_prompt_blocks("看图", &images).unwrap();
        assert_eq!(
            blocks,
            vec![
                serde_json::json!({"type": "text", "text": "看图"}),
                serde_json::json!({"type": "image", "data": "QUJD", "mimeType": "image/png"}),
                serde_json::json!({"type": "image", "data": "REVG", "mimeType": "image/jpeg"}),
            ]
        );
    }

    /// 超限拒绝:单图解码后 > 10MB → Err。
    #[test]
    fn prompt_blocks_reject_oversize_image() {
        let big = "A".repeat(MAX_IMAGE_BYTES / 3 * 4 + 8);
        let images = vec![img(&big, "image/png")];
        let err = build_prompt_blocks("x", &images).expect_err("超限图片必须拒绝");
        assert!(err.contains("10MB"), "err={err}");
    }

    /// MIME 白名单外拒绝(bmp 等)。
    #[test]
    fn prompt_blocks_reject_bad_mime() {
        let images = vec![img("QUJD", "image/bmp")];
        let err = build_prompt_blocks("x", &images).expect_err("非白名单 MIME 必须拒绝");
        assert!(err.contains("image/bmp"), "err={err}");
    }

    /// 全空拒绝(空文本 + 无图片)。
    #[test]
    fn prompt_blocks_reject_empty() {
        assert!(build_prompt_blocks("", &[]).is_err());
    }

    /// parse_usage_update:线上 camelCase 键正确映射成桥层 snake_case 元组;
    /// 缺键 / 类型不符 → None(畸形帧丢弃,不进前端)。
    #[test]
    fn usage_update_parses_to_event() {
        let params = serde_json::json!({
            "sessionId": "sess-1",
            "inputTokens": 11,
            "outputTokens": 7,
            "modelCalls": 3,
            "numTurns": 2,
        });
        assert_eq!(
            parse_usage_update(&params),
            Some(("sess-1".to_string(), 11, 7, 3, 2))
        );
        assert!(parse_usage_update(&serde_json::json!({"sessionId": "s"})).is_none());
        assert!(parse_usage_update(&serde_json::json!({
            "sessionId": "s",
            "inputTokens": "11",
            "outputTokens": 7,
            "modelCalls": 1,
            "numTurns": 1
        }))
        .is_none());
        // 序列化形状:snake_case type tag + snake_case 字段(前端按此解析)。
        let v = serde_json::to_value(BridgeEvent::UsageUpdate {
            session_id: "s".into(),
            input_tokens: 1,
            output_tokens: 2,
            model_calls: 3,
            num_turns: 4,
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "type": "usage_update",
                "session_id": "s",
                "input_tokens": 1,
                "output_tokens": 2,
                "model_calls": 3,
                "num_turns": 4
            })
        );
    }

    /// 行传输 mock:测试向 `line_tx` 推 agent→桥 帧,桥经
    /// take_line_receiver 消费(唯一消费者契约)。
    struct MockTransport {
        lines: std::sync::Mutex<Option<mpsc::Receiver<String>>>,
    }

    impl AgentTransport for MockTransport {
        fn send(&self, _line: String) -> Result<(), TransportError> {
            Ok(())
        }
        fn take_line_receiver(&self) -> Result<mpsc::Receiver<String>, TransportError> {
            self.lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .ok_or(TransportError::Closed)
        }
        fn subscribe_exit(&self) -> broadcast::Receiver<ExitInfo> {
            let (_tx, rx) = broadcast::channel::<ExitInfo>(1);
            rx
        }
        fn is_running(&self) -> bool {
            true
        }
        fn terminate(&self) {}
        fn shutdown(
            &self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
            Box::pin(async {})
        }
    }

    /// 未知会话的 usage_update 不得上屏:send_event_if_known 丢弃,事件流
    /// 收不到任何帧(防串话,与 session/update 同语义)。
    #[tokio::test]
    async fn usage_update_unknown_session_dropped() {
        use tokio::time::{timeout, Duration};
        let (line_tx, line_rx) = mpsc::channel::<String>(16);
        let transport = Arc::new(MockTransport {
            lines: std::sync::Mutex::new(Some(line_rx)),
        });
        let bridge = AcpBridge::attach(transport).expect("attach");
        assert!(bridge.sessions().is_empty(), "前置:无登记会话");
        let mut events = bridge.subscribe();
        line_tx
            .send(
                r#"{"jsonrpc":"2.0","method":"_x.ai/usage_update","params":{"sessionId":"ghost","inputTokens":11,"outputTokens":7,"modelCalls":1,"numTurns":1}}"#
                    .to_string(),
            )
            .await
            .expect("send line");
        // route_loop 异步消费该帧;若错误转发,事件必在窗口内到达。
        let got = timeout(Duration::from_millis(100), events.recv()).await;
        assert!(
            got.is_err(),
            "未知会话的 usage_update 必须被丢弃,不得上屏: {got:?}"
        );
    }

    /// 已知会话的 usage_update 必须转发上屏:直接登记会话后订阅事件流,
    /// 发送线上真实格式帧(方法名带前导下划线 `_x.ai/usage_update`),
    /// 断言窗口内收到完整 UsageUpdate(此测试用真实方法名,
    /// 能抓住 arm 方法串漏写下划线的回归)。
    #[tokio::test]
    async fn usage_update_known_session_forwarded() {
        use tokio::time::{timeout, Duration};
        let (line_tx, line_rx) = mpsc::channel::<String>(16);
        let transport = Arc::new(MockTransport {
            lines: std::sync::Mutex::new(Some(line_rx)),
        });
        let bridge = AcpBridge::attach(transport).expect("attach");
        bridge
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(SessionRecord {
                id: "s1".to_string(),
                cwd: PathBuf::from("."),
                office_task: None,
            });
        let mut events = bridge.subscribe();
        line_tx
            .send(
                r#"{"jsonrpc":"2.0","method":"_x.ai/usage_update","params":{"sessionId":"s1","inputTokens":11,"outputTokens":7,"modelCalls":1,"numTurns":1}}"#
                    .to_string(),
            )
            .await
            .expect("send line");
        let got = timeout(Duration::from_millis(100), events.recv()).await;
        match got {
            Ok(Ok(BridgeEvent::UsageUpdate {
                session_id,
                input_tokens,
                output_tokens,
                model_calls,
                num_turns,
            })) => {
                assert_eq!(session_id, "s1");
                assert_eq!(input_tokens, 11);
                assert_eq!(output_tokens, 7);
                assert_eq!(model_calls, 1);
                assert_eq!(num_turns, 1);
            }
            other => panic!("已知会话的 usage_update 应转发上屏: {other:?}"),
        }
    }

    /// §P:权限请求 per-request 超时 → 拒绝。
    ///
    /// 独立子模块:自带「记录下发帧」的 mock(既有 `MockTransport` 只吞帧,
    /// 断言不了回给 agent 的 outcome),并统一放行测试专用的
    /// unwrap/expect/panic 告警(与 `jsonrpc.rs` 测试模块同一风格,
    /// 非测试代码仍受 `[lints.clippy]` 约束)。
    mod permission_timeout {
        #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

        use super::super::*;
        use crate::transport::{AgentTransport, ExitInfo, TransportError};
        use std::path::PathBuf;
        use std::sync::Arc;
        use tokio::sync::{broadcast, mpsc};
        use tokio::time::{sleep, Duration};

        /// 记录型 mock:喂入站行的同时收集桥下发到 agent 的帧。
        struct RecordingTransport {
            lines: std::sync::Mutex<Option<mpsc::Receiver<String>>>,
            sent: std::sync::Mutex<Vec<String>>,
        }

        impl RecordingTransport {
            fn sent_frames(&self) -> Vec<serde_json::Value> {
                self.sent
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .iter()
                    .map(|line| serde_json::from_str(line).expect("下发帧必须是合法 JSON"))
                    .collect()
            }
        }

        impl AgentTransport for RecordingTransport {
            fn send(&self, line: String) -> Result<(), TransportError> {
                self.sent
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(line);
                Ok(())
            }
            fn take_line_receiver(&self) -> Result<mpsc::Receiver<String>, TransportError> {
                self.lines
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take()
                    .ok_or(TransportError::Closed)
            }
            fn subscribe_exit(&self) -> broadcast::Receiver<ExitInfo> {
                let (_tx, rx) = broadcast::channel::<ExitInfo>(1);
                rx
            }
            fn is_running(&self) -> bool {
                true
            }
            fn terminate(&self) {}
            fn shutdown(
                &self,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
                Box::pin(async {})
            }
        }

        /// 起一座注入短超时的桥,并登记会话 `s1`(权限请求的前提)。
        fn boot(
            timeout: Duration,
        ) -> (
            Arc<AcpBridge>,
            Arc<RecordingTransport>,
            mpsc::Sender<String>,
        ) {
            let (line_tx, line_rx) = mpsc::channel::<String>(16);
            let transport = Arc::new(RecordingTransport {
                lines: std::sync::Mutex::new(Some(line_rx)),
                sent: std::sync::Mutex::new(Vec::new()),
            });
            let bridge = AcpBridge::new_with_timeout(transport.clone(), timeout).expect("attach");
            bridge
                .sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(SessionRecord {
                    id: "s1".to_string(),
                    cwd: PathBuf::from("."),
                    office_task: None,
                });
            (bridge, transport, line_tx)
        }

        /// 线上形状的 `session/request_permission`(camelCase optionId/kind)。
        fn permission_frame(id: u64, options: serde_json::Value) -> String {
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "session/request_permission",
                "params": {
                    "sessionId": "s1",
                    "toolCall": {"title": "cargo test"},
                    "options": options
                }
            })
            .to_string()
        }

        fn allow_and_reject() -> serde_json::Value {
            serde_json::json!([
                {"optionId": "allow-once", "name": "Allow once", "kind": "allow_once"},
                {"optionId": "reject-once", "name": "Reject once", "kind": "reject_once"}
            ])
        }

        /// 等 route_loop 把入站帧落表(1ms 轮询,不赌固定 sleep)。
        async fn wait_pending(bridge: &AcpBridge, request_id: u64) {
            for _ in 0..500 {
                if bridge
                    .permission_pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .contains_key(&request_id)
                {
                    return;
                }
                sleep(Duration::from_millis(1)).await;
            }
            panic!("权限请求 {request_id} 未在 500ms 内落表");
        }

        /// 等 `reason="timeout"` 的 `PermissionCancelled`,返回 (session_id, reason)。
        async fn wait_timeout_event(
            events: &mut broadcast::Receiver<BridgeEvent>,
        ) -> (String, String) {
            let wait = async {
                loop {
                    match events.recv().await {
                        Ok(BridgeEvent::PermissionCancelled { session_id, reason }) => {
                            if reason == "timeout" {
                                return (session_id, reason);
                            }
                        }
                        Ok(_) => {}
                        Err(e) => panic!("事件流中断: {e}"),
                    }
                }
            };
            tokio::time::timeout(Duration::from_millis(1500), wait)
                .await
                .expect("1.5s 内必须收到 reason=timeout 的 PermissionCancelled")
        }

        fn pending_len(bridge: &AcpBridge) -> usize {
            bridge
                .permission_pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len()
        }

        /// P.4-1:到期 → `selected` + reject optionId,pending 清空,事件带 timeout。
        #[tokio::test]
        async fn permission_timeout_rejects() {
            let (bridge, transport, line_tx) = boot(Duration::from_millis(20));
            let mut events = bridge.subscribe();
            line_tx
                .send(permission_frame(7, allow_and_reject()))
                .await
                .expect("send frame");

            let (session_id, reason) = wait_timeout_event(&mut events).await;
            assert_eq!(session_id, "s1");
            assert_eq!(reason, "timeout");

            let frames = transport.sent_frames();
            assert_eq!(frames.len(), 1, "超时只回一条应答: {frames:?}");
            assert_eq!(frames[0]["id"], serde_json::json!(7));
            assert_eq!(frames[0]["result"]["outcome"]["outcome"], "selected");
            assert_eq!(
                frames[0]["result"]["outcome"]["optionId"], "reject-once",
                "超时只能选 reject 选项: {frames:?}"
            );
            assert_eq!(pending_len(&bridge), 0, "超时后条目必须移除");
        }

        /// P.4-2:先 resolve → 超时任务到点查无此项,不得有第二条应答。
        #[tokio::test]
        async fn permission_resolved_before_timeout_no_second_response() {
            let (bridge, transport, line_tx) = boot(Duration::from_millis(20));
            line_tx
                .send(permission_frame(9, allow_and_reject()))
                .await
                .expect("send frame");
            wait_pending(&bridge, 9).await;

            bridge
                .resolve_permission(9, "allow-once")
                .expect("合法选项应回填成功");
            // 6 倍超时窗口:sleep_until 早已到期,任务应已 no-op 退出。
            sleep(Duration::from_millis(120)).await;

            let frames = transport.sent_frames();
            assert_eq!(frames.len(), 1, "resolve 后不得再有超时应答: {frames:?}");
            assert_eq!(frames[0]["result"]["outcome"]["outcome"], "selected");
            assert_eq!(frames[0]["result"]["outcome"]["optionId"], "allow-once");
            assert_eq!(pending_len(&bridge), 0);
            assert!(
                bridge.cancel_permission(9).is_err(),
                "已回填的请求不可再取消(后到者 no-op)"
            );
        }

        /// P.4-3:无 reject 选项(只有 allow)→ 到期走 cancelled,不带 optionId。
        #[tokio::test]
        async fn permission_timeout_without_reject_option_cancels() {
            let (bridge, transport, line_tx) = boot(Duration::from_millis(20));
            let mut events = bridge.subscribe();
            line_tx
                .send(permission_frame(
                    11,
                    serde_json::json!([
                        {"optionId": "allow-once", "name": "Allow once", "kind": "allow_once"}
                    ]),
                ))
                .await
                .expect("send frame");

            let (_session_id, reason) = wait_timeout_event(&mut events).await;
            assert_eq!(reason, "timeout");

            let frames = transport.sent_frames();
            assert_eq!(frames.len(), 1, "超时只回一条应答: {frames:?}");
            let outcome = &frames[0]["result"]["outcome"];
            assert_eq!(outcome["outcome"], "cancelled");
            assert!(
                outcome.get("optionId").is_none(),
                "cancelled 不得携带 optionId: {outcome}"
            );
            assert_eq!(pending_len(&bridge), 0);
        }

        /// P.4-4:所有超时路径下发的 optionId 均不属于 allow 集合。
        #[tokio::test]
        async fn timeout_never_allows() {
            let cases: [(&str, serde_json::Value); 4] = [
                ("allow 在前", allow_and_reject()),
                (
                    "只有 allow",
                    serde_json::json!([
                        {"optionId": "allow-always", "name": "Always", "kind": "allow_always"}
                    ]),
                ),
                ("空 options", serde_json::json!([])),
                (
                    "reject_always",
                    serde_json::json!([
                        {"optionId": "allow-once", "name": "Allow once", "kind": "allow_once"},
                        {"optionId": "reject-always", "name": "Never", "kind": "reject_always"}
                    ]),
                ),
            ];

            for (idx, (label, options)) in cases.into_iter().enumerate() {
                let allow_ids: Vec<String> = options
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter(|o| {
                                o["kind"]
                                    .as_str()
                                    .unwrap_or("")
                                    .to_ascii_lowercase()
                                    .contains("allow")
                            })
                            .filter_map(|o| o["optionId"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let request_id = 100 + idx as u64;
                let (bridge, transport, line_tx) = boot(Duration::from_millis(20));
                let mut events = bridge.subscribe();
                line_tx
                    .send(permission_frame(request_id, options))
                    .await
                    .expect("send frame");

                let (_session_id, reason) = wait_timeout_event(&mut events).await;
                assert_eq!(reason, "timeout", "[{label}]");

                let frames = transport.sent_frames();
                assert_eq!(frames.len(), 1, "[{label}] 超时只回一条: {frames:?}");
                let outcome = &frames[0]["result"]["outcome"];
                let kind = outcome["outcome"].as_str().unwrap_or("");
                assert!(
                    kind == "selected" || kind == "cancelled",
                    "[{label}] 非法 outcome: {outcome}"
                );
                if let Some(option_id) = outcome["optionId"].as_str() {
                    assert!(
                        !allow_ids.iter().any(|allow| allow == option_id),
                        "[{label}] 超时不得选择 allow 选项({option_id}): {outcome}"
                    );
                }
                assert_eq!(pending_len(&bridge), 0, "[{label}] 条目必须移除");

                // 任务已退出:静置一个超时窗口,仍不得出现第二条应答。
                sleep(Duration::from_millis(60)).await;
                assert_eq!(transport.sent_frames().len(), 1, "[{label}] 不得二次应答");
            }
        }
    }
}
