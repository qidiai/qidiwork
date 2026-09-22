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

use crate::acp::jsonrpc::{self, ACP_PROTOCOL_VERSION, ERR_METHOD_NOT_FOUND, Incoming};
use crate::transport::{AgentTransport, ExitInfo, TransportError};

/// 单个 RPC 的默认超时。session/prompt 的回合可能跑很久,单独放宽。
const RPC_TIMEOUT: Duration = Duration::from_secs(60);
const PROMPT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
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
        let line_rx = transport.take_line_receiver()?;
        let (event_tx, _) = broadcast::channel(EVENT_CAPACITY);
        let bridge = Arc::new(Self {
            transport,
            next_id: AtomicU64::new(1),
            initialized: AtomicBool::new(false),
            pending: Arc::new(Mutex::new(HashMap::new())),
            permission_pending: Arc::new(Mutex::new(HashMap::new())),
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
        let mut permissions = self
            .permission_pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for id in permissions.keys() {
            let resp = jsonrpc::success_response(
                *id,
                serde_json::json!({"outcome": {"outcome": "cancelled"}}),
            );
            let _ = self.transport.send(resp.to_string());
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
                    if !self.session_known(&session_id) {
                        tracing::warn!(%session_id, "未知会话的 update,丢弃");
                        continue;
                    }
                    let update = params
                        .get("update")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let _ = self
                        .event_tx
                        .send(BridgeEvent::SessionUpdate { session_id, update });
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
                    if is_model_changed && self.session_known(session_id) {
                        if let Some(mid) =
                            update.and_then(|u| u.get("model_id")).and_then(|m| m.as_str())
                        {
                            // 回显通知不携带档位变更的权威值(发起方自身被内核门控),
                            // 档位由本端 set_model 就地 patch,故此处 effort 传 None。
                            self.patch_current_model(session_id, mid, None);
                        }
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
            if !self.session_known(&session_id) {
                tracing::warn!(%session_id, "未知会话的权限请求,拒绝");
                self.send_best_effort(jsonrpc::error_response(id, -32602, "未知会话"));
                return;
            }
            let option_ids: Vec<String> = params
                .get("options")
                .and_then(|o| o.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|o| o.get("id").and_then(|i| i.as_str()))
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            self.permission_pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(
                    id,
                    PermissionEntry {
                        session_id: session_id.clone(),
                        option_ids,
                    },
                );
            let _ = self.event_tx.send(BridgeEvent::PermissionRequest {
                request_id: id,
                session_id,
                tool_call: params
                    .get("toolCall")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
                options: params
                    .get("options")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            });
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

    fn send_permission_outcome(
        &self,
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
        self.transport
            .send(resp.to_string())
            .map_err(|e| format!("发送权限应答失败: {e}"))?;
        tracing::info!(request_id, %session_id, %outcome, ?option_id, "权限已回填");
        Ok(())
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
        if let Err(e) = self.transport.send(msg.to_string()) {
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
    pub fn notify_session_restored(&self, session_id: &str) {
        let _ = self.event_tx.send(BridgeEvent::SessionRestored {
            session_id: session_id.to_string(),
        });
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
        let text = self.decorate_prompt(session_id, text);
        let blocks = build_prompt_blocks(&text, images)?;
        let params = serde_json::json!({
            "sessionId": session_id,
            "prompt": blocks
        });
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
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
                            let _ = this.event_tx.send(BridgeEvent::TurnUsage {
                                session_id: session.clone(),
                                meta,
                            });
                        } else {
                            tracing::warn!(%session, "session/prompt _meta 超过 64KB,丢弃用量透传");
                        }
                    }
                    let _ = this.event_tx.send(BridgeEvent::TurnCompleted {
                        session_id: session,
                        stop_reason: stop,
                    });
                }
                Ok(Ok(Err((code, message)))) => {
                    tracing::error!(%session, code, %message, "session/prompt 失败");
                    let _ = this.event_tx.send(BridgeEvent::TurnCompleted {
                        session_id: session,
                        stop_reason: format!("error: {code} {message}"),
                    });
                }
                Ok(Err(_)) | Err(_) => {
                    tracing::error!(%session, "session/prompt 超时或断开");
                    let _ = this.event_tx.send(BridgeEvent::TurnCompleted {
                        session_id: session,
                        stop_reason: "error: timeout/disconnected".into(),
                    });
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
    /// 据缓存刷新)。返回更新后的快照。
    fn patch_current_model(
        &self,
        session_id: &str,
        model_id: &str,
        effort: Option<&str>,
    ) -> serde_json::Value {
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
        state
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
        let mut params = serde_json::json!({"sessionId": session_id, "modelId": model_id});
        if let Some(effort) = effort {
            params["_meta"] = serde_json::json!({ "reasoningEffort": effort });
        }
        self.rpc("session/set_model", params, RPC_TIMEOUT).await?;
        Ok(self.patch_current_model(session_id, model_id, effort))
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
}
