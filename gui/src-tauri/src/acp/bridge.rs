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
const EVENT_CAPACITY: usize = 256;

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
}

#[derive(Debug, Clone)]
struct SessionRecord {
    id: String,
    cwd: PathBuf,
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
    pub fn sessions(&self) -> Vec<(String, PathBuf)> {
        self.sessions
            .lock()
            .map(|s| s.iter().map(|r| (r.id.clone(), r.cwd.clone())).collect())
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
                            self.patch_current_model(session_id, mid);
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
        let params = serde_json::json!({
            "protocolVersion": ACP_PROTOCOL_VERSION,
            "clientCapabilities": {
                // 未声明 fs / terminal 能力:agent 的文件操作走其自有工具
                // (GUI 不经 ACP 提供客户端文件访问,缩小攻击面,方案 v2 §D4)
            }
        });
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

    pub async fn new_session(&self, cwd: PathBuf) -> Result<String, String> {
        let params = serde_json::json!({"cwd": cwd.to_string_lossy(), "mcpServers": []});
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
            });
        Ok(session_id)
    }

    /// 恢复既有会话(重 spawn 后)。成功即重新登记。
    /// 注:load 期间的历史重放(session/update)发生在登记前,会被路由
    /// 循环按未知会话丢弃——恢复场景前端以 SessionRestored 为准。
    pub async fn load_session(&self, session_id: &str, cwd: PathBuf) -> Result<(), String> {
        let params = serde_json::json!({
            "sessionId": session_id,
            "cwd": cwd.to_string_lossy(),
            "mcpServers": []
        });
        self.rpc("session/load", params, RPC_TIMEOUT)
            .await
            .map(|result| self.cache_model_state(session_id, &result))?;
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(SessionRecord {
                id: session_id.to_string(),
                cwd,
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

    /// 发起回合:立即返回,回合完成经 [`BridgeEvent::TurnCompleted`] 通知
    /// (GUI 调用不应阻塞数分钟)。`self: &Arc<Self>` 以便等待任务克隆桥。
    /// 注意:30 分钟超时 ≠ 取消——超时只发错误事件,agent 仍在跑,
    /// 真正中断需 `cancel`(M4 编排)。
    pub fn prompt(self: &Arc<Self>, session_id: &str, text: &str) -> Result<(), String> {
        let params = serde_json::json!({
            "sessionId": session_id,
            "prompt": [{"type": "text", "text": text}]
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
    /// (availableModels 为空,前端仅展示 id)。返回更新后的快照。
    fn patch_current_model(&self, session_id: &str, model_id: &str) -> serde_json::Value {
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
    pub async fn set_model(&self, session_id: &str, model_id: &str) -> Result<serde_json::Value, String> {
        let params = serde_json::json!({"sessionId": session_id, "modelId": model_id});
        self.rpc("session/set_model", params, RPC_TIMEOUT).await?;
        Ok(self.patch_current_model(session_id, model_id))
    }
}

/// 断开后的重 spawn 编排(M3 崩溃恢复,方案 v2 §D2):
/// spawn 新 agent → initialize → 逐会话 session/load;
/// 加载失败的单会话经 `SessionRestoreFailed` 上报,不拖垮其余会话。
/// 返回 (新桥, 恢复成功的会话 id, 失败清单 [(id, 错误)])。
pub async fn reconnect(
    transport: Arc<dyn AgentTransport>,
    sessions: Vec<(String, PathBuf)>,
) -> Result<(Arc<AcpBridge>, Vec<String>, Vec<(String, String)>), String> {
    let bridge = AcpBridge::attach(transport).map_err(|e| e.to_string())?;
    bridge.initialize().await?;
    bridge.initialized.store(true, Ordering::Relaxed);
    let mut restored = Vec::new();
    let mut failed = Vec::new();
    for (id, cwd) in sessions {
        match bridge.load_session(&id, cwd).await {
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
