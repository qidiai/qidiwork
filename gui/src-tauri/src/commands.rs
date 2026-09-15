//! Tauri IPC 命令层:agent 子进程生命周期(M2)+ ACP 会话(M3)。
//!
//! 事件契约(前端,M4 消费):
//!
//! - `agent-exit`:agent 进程退出(ExitInfo)
//! - `acp-event`:BridgeEvent(serde tag=type),含 session_update /
//!   permission_request / disconnected / turn_completed / session_restored
//!
//! 命令契约:`session_start` / `session_prompt` / `session_cancel` /
//! `permission_respond` / `permission_cancel` / `agent_recover`(崩溃恢复)。
//!
//! 锁序约束(k3 M3 审计 §6):AgentState 与 BridgeState 两把 tokio 锁
//! **禁止嵌套持有**——所有路径都是"短锁克隆/取出 Arc 后立即释放"或
//! "跨 spawn 全程持锁但锁内不取另一把"。改动任何启动/恢复路径前先核对本约束。M2 的行级命令
//! (`agent_send`/`agent-line`)已由 ACP 桥取代,移除。

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::acp::{AcpBridge, reconnect};
use crate::office::{self, ArtifactCard, WorkspaceInfo, watch::ManifestWatch};
use crate::persist::{self, PersistedSession};
use crate::process::{AgentProcess, SpawnConfig};
use crate::skills;
use crate::settings;
use crate::transport::AgentTransport;

/// 全局 agent 句柄。None = 未启动/已退出。
/// 用 tokio Mutex(而非 std):启动需要跨 spawn 全程持锁,
/// 防止并发调用双开 agent 进程(自查发现)。
#[derive(Default)]
pub struct AgentState {
    inner: tokio::sync::Mutex<Option<Arc<AgentProcess>>>,
}

/// 全局 ACP 桥。None = 未 attach(与 agent 生命周期 1:1,断开即失效)。
/// generation:每次替换桥递增;事件转发任务捕获创建时的代际,发射前
/// 校验——旧桥断连/收尾事件晚到时被静默丢弃,防前端代际混淆
/// (k3 M3 审计 §5)。
pub struct BridgeState {
    inner: tokio::sync::Mutex<Option<Arc<AcpBridge>>>,
    generation: std::sync::atomic::AtomicU64,
}

impl Default for BridgeState {
    fn default() -> Self {
        Self {
            inner: tokio::sync::Mutex::new(None),
            generation: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl BridgeState {
    fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn next_generation(&self) -> u64 {
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentStatusInfo {
    pub running: bool,
    pub pid: Option<u32>,
    pub uptime_secs: Option<u64>,
}

fn status_of(proc: Option<&Arc<AgentProcess>>) -> AgentStatusInfo {
    match proc {
        Some(p) => AgentStatusInfo {
            running: p.is_running(),
            pid: p.pid(),
            uptime_secs: Some(p.uptime().as_secs()),
        },
        None => AgentStatusInfo {
            running: false,
            pid: None,
            uptime_secs: None,
        },
    }
}

/// 确保 agent 子进程在运行(幂等),并挂好退出监控。
async fn ensure_agent(app: &AppHandle, state: &AgentState) -> Result<Arc<AgentProcess>, String> {
    let mut guard = state.inner.lock().await;
    if let Some(p) = guard.as_ref() {
        if p.is_running() {
            return Ok(Arc::clone(p));
        }
    }

    let home = app.path().home_dir().ok();
    let process = AgentProcess::spawn(SpawnConfig::default_agent(home))
        .await
        .map_err(String::from)?;

    // 退出转发 + 状态清理:agent 崩溃后 GUI 可感知(bridge 侧另发
    // Disconnected 事件;恢复编排见 agent_recover)。
    let exit_task_proc = Arc::clone(&process);
    let exit_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut rx = exit_task_proc.subscribe_exit();
        let info = loop {
            match rx.recv().await {
                Ok(info) => break info,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        };
        let _ = exit_app.emit("agent-exit", info);
        if let Ok(mut guard) = exit_app.state::<AgentState>().inner.try_lock()
            && guard
                .as_ref()
                .is_some_and(|p| Arc::ptr_eq(p, &exit_task_proc))
        {
            *guard = None;
        }
    });

    *guard = Some(Arc::clone(&process));
    tracing::info!(pid = process.pid(), "agent 子进程已就绪");
    Ok(process)
}

/// 确保桥存在且 attach 在当前 transport 上,并挂好事件转发。
async fn ensure_bridge(
    app: &AppHandle,
    agent: &Arc<AgentProcess>,
    state: &BridgeState,
) -> Result<Arc<AcpBridge>, String> {
    let mut guard = state.inner.lock().await;
    if let Some(b) = guard.as_ref() {
        if b.is_running() {
            return Ok(Arc::clone(b));
        }
        // 旧桥挂在已死的 transport 上:弃用(其会话清单由 recover 续接)。
        *guard = None;
    }

    let bridge = AcpBridge::attach(Arc::clone(agent) as Arc<dyn AgentTransport>)
        .map_err(|e| e.to_string())?;

    // 桥事件 → Tauri 事件(前端唯一入口);发射前校验代际,
    // 旧桥晚到事件(断连收尾等)在替换后不再透传。
    let generation = state.next_generation();
    let event_bridge = Arc::clone(&bridge);
    let event_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut rx = event_bridge.subscribe();
        loop {
            if event_app.state::<BridgeState>().generation() != generation {
                break; // 桥已被替换:本转发任务退役
            }
            match rx.recv().await {
                Ok(event) => {
                    if let Ok(payload) = serde_json::to_value(&event) {
                        let _ = event_app.emit("acp-event", payload);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(dropped = n, "acp-event 订阅滞后");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    *guard = Some(Arc::clone(&bridge));
    Ok(bridge)
}

/// 启动 agent 子进程(幂等)。事件:退出 → `agent-exit`。
/// (行级 `agent-line` 事件已由 M3 的 `acp-event` 取代。)
#[tauri::command]
pub async fn agent_start(
    app: AppHandle,
    state: State<'_, AgentState>,
) -> Result<AgentStatusInfo, String> {
    let process = ensure_agent(&app, &state).await?;
    Ok(status_of(Some(&process)))
}

/// 终止 agent(整树)并等待退出。桥随进程失效(BridgeState 清理由
/// 下次 ensure_bridge 的 is_running 判定完成)。
#[tauri::command]
pub async fn agent_stop(
    agent: State<'_, AgentState>,
    bridge: State<'_, BridgeState>,
) -> Result<AgentStatusInfo, String> {
    bridge.inner.lock().await.take();
    let proc = agent.inner.lock().await.take();
    match proc {
        Some(p) => {
            p.shutdown().await;
            Ok(status_of(None))
        }
        None => Ok(status_of(None)),
    }
}

#[tauri::command]
pub async fn agent_status(state: State<'_, AgentState>) -> Result<AgentStatusInfo, String> {
    let guard = state.inner.lock().await;
    Ok(status_of(guard.as_ref()))
}

/// 开启(或恢复)一个 ACP 会话:确保 agent + 桥 + initialize,
/// 然后 session/new。返回 sessionId。
#[tauri::command]
pub async fn session_start(
    app: AppHandle,
    agent: State<'_, AgentState>,
    bridge: State<'_, BridgeState>,
    cwd: Option<String>,
) -> Result<String, String> {
    let process = ensure_agent(&app, &agent).await?;
    let bridge = ensure_bridge(&app, &process, &bridge).await?;
    bridge.ensure_initialized().await?;
    let cwd = cwd
        .map(PathBuf::from)
        .or_else(|| app.path().home_dir().ok());
    let cwd = cwd.unwrap_or_else(|| PathBuf::from("."));
    let session_id = bridge.new_session(cwd.clone()).await?;
    // 会话登记落盘(k3 M3 审计登记项:GUI 重启后可续接)。
    if let Ok(dir) = app.path().app_data_dir() {
        if let Err(e) = persist::upsert_session(
            &dir,
            PersistedSession {
                session_id: session_id.clone(),
                cwd: cwd.to_string_lossy().into_owned(),
                title: None,
            },
        ) {
            tracing::warn!(%session_id, error = %e, "会话登记落盘失败");
        }
    }
    Ok(session_id)
}

/// 发起回合:立即返回;流式更新与回合结束经 `acp-event` 推送。
#[tauri::command]
pub async fn session_prompt(
    bridge: State<'_, BridgeState>,
    session_id: String,
    text: String,
) -> Result<(), String> {
    let b = bridge.inner.lock().await.clone();
    match b {
        Some(b) if b.is_running() => b.prompt(&session_id, &text),
        _ => Err("桥未就绪(agent 未运行或已断开)".into()),
    }
}

#[tauri::command]
pub async fn session_cancel(
    bridge: State<'_, BridgeState>,
    session_id: String,
) -> Result<(), String> {
    let b = bridge.inner.lock().await.clone();
    match b {
        Some(b) if b.is_running() => b.cancel(&session_id),
        _ => Err("桥未就绪".into()),
    }
}

/// 回填权限审批选择(前端弹窗的结果)。
#[tauri::command]
pub async fn permission_respond(
    bridge: State<'_, BridgeState>,
    request_id: u64,
    option_id: String,
) -> Result<(), String> {
    let b = bridge.inner.lock().await.clone();
    match b {
        Some(b) => b.resolve_permission(request_id, &option_id),
        None => Err("桥未就绪".into()),
    }
}

/// 用户取消权限对话框(非选择式关闭)。
#[tauri::command]
pub async fn permission_cancel(
    bridge: State<'_, BridgeState>,
    request_id: u64,
) -> Result<(), String> {
    let b = bridge.inner.lock().await.clone();
    match b {
        Some(b) => b.cancel_permission(request_id),
        None => Err("桥未就绪".into()),
    }
}

/// 崩溃恢复:重 spawn agent + initialize + 逐会话 session/load。
/// 返回成功恢复的会话数;单会话失败不拖垮其余(经 SessionRestoreFailed
/// 事件上报)。恢复语义见方案 v2 §D2。
#[tauri::command]
pub async fn agent_recover(
    app: AppHandle,
    agent: State<'_, AgentState>,
    bridge: State<'_, BridgeState>,
) -> Result<usize, String> {
    // 会话清单以磁盘为准(进程内旧桥可能已丢);两处合并去重。
    let data_dir = app.path().app_data_dir().ok();
    let mut sessions: Vec<(String, PathBuf)> = {
        let guard = bridge.inner.lock().await;
        guard.as_ref().map(|b| b.sessions()).unwrap_or_default()
    };
    if let Some(dir) = &data_dir {
        for s in persist::load_sessions(dir) {
            let cwd = PathBuf::from(&s.cwd);
            // cwd 校验(k3 M4 审计 §3):不存在/非目录的登记项跳过
            if !cwd.is_dir() {
                tracing::warn!(session_id = %s.session_id, cwd = %s.cwd, "持久化会话 cwd 无效,跳过");
                continue;
            }
            let entry = (s.session_id, cwd);
            if !sessions.iter().any(|(id, _)| *id == entry.0) {
                sessions.push(entry);
            }
        }
    }

    let process = ensure_agent(&app, &agent).await?;
    let (new_bridge, restored, failed) =
        reconnect(Arc::clone(&process) as Arc<dyn AgentTransport>, sessions).await?;
    // 恢复失败的会话从磁盘移除(下次 recover 不再撞同一堵墙)。
    if let Some(dir) = &data_dir {
        for (id, _) in &failed {
            if let Err(e) = persist::remove_session(dir, id) {
                tracing::warn!(session_id = %id, error = %e, "移除恢复失败会话失败");
            }
        }
    }

    // 挂事件转发(带新代际:替换后旧桥晚到事件被丢弃,k3 M3 审计 §5)。
    let generation = bridge.next_generation();
    let event_bridge = Arc::clone(&new_bridge);
    let event_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut rx = event_bridge.subscribe();
        loop {
            if event_app.state::<BridgeState>().generation() != generation {
                break; // 桥已被替换:本转发任务退役
            }
            match rx.recv().await {
                Ok(event) => {
                    if let Ok(payload) = serde_json::to_value(&event) {
                        let _ = event_app.emit("acp-event", payload);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    // 展示流有损(k3 审计 W3):丢帧最坏导致前端少一条
                    // TurnUsage/TurnCompleted,后者会让 busy 卡住——记日志
                    // 便于与"任务卡住"类用户报告对账
                    tracing::warn!(dropped = n, "recover 桥 acp-event 订阅滞后");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    *bridge.inner.lock().await = Some(Arc::clone(&new_bridge));
    Ok(restored.len())
}

/// 退出路径的同步清理(main 的 ExitRequested 钩子调用):
/// Job Object terminate 是同步整树回收,不等待进程完全退出
/// (应用退出场景下由内核兜底,方案 v2 §D4)。
pub fn terminate_current(state: &AgentState) {
    // 退出钩子是同步上下文:try_lock 失败(正被启动流程持有)时
    // 依赖 Job Object kill-on-close 兜底,不阻塞退出。
    if let Ok(guard) = state.inner.try_lock()
        && let Some(p) = guard.as_ref()
    {
        p.terminate();
    }
}

/// 办公工作台状态:watcher 句柄(单实例)。root 由首次调用方决定。
#[derive(Default)]
pub struct OfficeState {
    watch: tokio::sync::Mutex<Option<Arc<ManifestWatch>>>,
}

/// 办公数据面挂载点:扫描 + 启动 manifest 监听(幂等)。前端在应用
/// 初始化时调用一次;每次 manifest 变更经 `office-event` 推送
/// {task, artifacts}(变更后即时重读,卡片实时刷新,方案 v2 P1)。
#[tauri::command]
pub async fn office_watch_start(
    app: AppHandle,
    state: State<'_, OfficeState>,
) -> Result<(), String> {
    let mut guard = state.watch.lock().await;
    if guard.is_some() {
        return Ok(());
    }
    let home = app.path().home_dir().ok().ok_or("无法解析主目录")?;
    let root = office::workspaces_root(&home);
    let _ = std::fs::create_dir_all(&root);

    let watch_root = root.clone();
    let office_app = app.clone();
    let watch = ManifestWatch::start(watch_root, move |task| {
        // 失效通知而非快照(k3 P1a 审计 §4:事件与拉取乱序时,
        // 快照可能用过期数据覆盖新数据;前端收到后自行重拉)
        let _ = office_app.emit("office-event", serde_json::json!({ "task": task }));
    })
    .map_err(|e| e.to_string())?;
    *guard = Some(Arc::new(watch));
    tracing::info!(root = %root.display(), "office manifest 监听已启动");
    Ok(())
}

/// 工作区列表(左区)。
#[tauri::command]
pub async fn office_scan(app: AppHandle) -> Result<Vec<WorkspaceInfo>, String> {
    let home = app.path().home_dir().ok().ok_or("无法解析主目录")?;
    Ok(office::list_workspaces(&office::workspaces_root(&home)))
}

/// 技能列表(左区「常用技能」)。只读扫描 ~/.qidi/skills/*/SKILL.md
/// frontmatter;真正的技能执行与权限审批都在 agent 内核侧。
#[tauri::command]
pub async fn skills_list(app: AppHandle) -> Result<Vec<skills::SkillInfo>, String> {
    let home = app.path().home_dir().ok().ok_or("无法解析主目录")?;
    Ok(skills::list_skills(&skills::skills_root(&home)))
}

/// 设置读取(设置面):当前默认模型 + [model.*] 可选项。
/// 永不回传 api_key 明文,只回传 has_api_key。
#[tauri::command]
pub async fn settings_read(app: AppHandle) -> Result<settings::SettingsInfo, String> {
    let home = app.path().home_dir().ok().ok_or("无法解析主目录")?;
    settings::read_settings(&settings::config_path(&home))
}

/// 设置保存(设置面):设 [models].default = model_id;api_key 非空时
/// 一并写入该模型的 [model.<id>].api_key。内核在下次启动时生效。
#[tauri::command]
pub async fn settings_save(
    app: AppHandle,
    model_id: String,
    api_key: Option<String>,
) -> Result<(), String> {
    let home = app.path().home_dir().ok().ok_or("无法解析主目录")?;
    settings::save_settings(&settings::config_path(&home), &model_id, api_key.as_deref())
}

/// 历史会话清单(侧栏「历史会话」)。
#[tauri::command]
pub async fn sessions_history(app: AppHandle) -> Result<Vec<PersistedSession>, String> {
    let dir = app
        .path()
        .app_data_dir()
        .ok()
        .ok_or("无法解析数据目录")?;
    Ok(persist::load_sessions(&dir))
}

/// 记录会话标题(前端在会话首条 prompt 后调用;截断在前端做)。
#[tauri::command]
pub async fn session_set_title(
    app: AppHandle,
    session_id: String,
    title: String,
) -> Result<(), String> {
    let dir = app
        .path()
        .app_data_dir()
        .ok()
        .ok_or("无法解析数据目录")?;
    persist::set_title(&dir, &session_id, &title).map_err(|e| format!("标题记录失败: {e}"))
}

/// 恢复单个历史会话:ensure agent/桥 → session/load;SessionRestored
/// 事件经既有转发到前端切换 active sessionId(界面消息不回放,历史
/// 上下文在 agent 侧续接)。
#[tauri::command]
pub async fn session_resume(
    app: AppHandle,
    agent: State<'_, AgentState>,
    bridge: State<'_, BridgeState>,
    session_id: String,
) -> Result<(), String> {
    let dir = app
        .path()
        .app_data_dir()
        .ok()
        .ok_or("无法解析数据目录")?;
    let s = persist::load_sessions(&dir)
        .into_iter()
        .find(|s| s.session_id == session_id)
        .ok_or("会话未登记")?;
    let cwd = PathBuf::from(&s.cwd);
    if !cwd.is_dir() {
        return Err(format!("会话工作目录已失效: {}", s.cwd));
    }
    let process = ensure_agent(&app, &agent).await?;
    let b = ensure_bridge(&app, &process, &bridge).await?;
    b.ensure_initialized().await?;
    if b.sessions().iter().any(|(id, _)| id == &session_id) {
        // 已在本桥也要发事件:前端只在收到 SessionRestored 时切 active
        // sessionId,早退不发会让点击静默无效(k3 审计)
        b.notify_session_restored(session_id);
        return Ok(());
    }
    b.load_session(&session_id, cwd).await?;
    b.notify_session_restored(session_id);
    Ok(())
}

/// 历史会话数量(启动自动连接的判断依据)。
#[tauri::command]
pub async fn sessions_count(app: AppHandle) -> Result<usize, String> {
    let dir = app.path().app_data_dir().ok().ok_or("无法解析数据目录")?;
    Ok(persist::load_sessions(&dir).len())
}

/// 清空历史会话登记;archive=true 时先改名留档。仅清 GUI 登记簿,
/// agent 侧对话内容不动。
#[tauri::command]
pub async fn sessions_clear(app: AppHandle, archive: bool) -> Result<String, String> {
    let dir = app.path().app_data_dir().ok().ok_or("无法解析数据目录")?;
    persist::clear_sessions(&dir, archive).map_err(|e| format!("清空失败: {e}"))
}

/// 删除任务工作区(含目录内全部产物文件,不可恢复)。校验链:
/// task 名合法(拒绝分隔符/点路径/盘符)→ canonicalize 后必须严格落在
/// workspaces 根内(含拒绝根本身,防 task="." 删光整个根,k3 审计 P1)。
#[tauri::command]
pub async fn office_delete_workspace(app: AppHandle, task: String) -> Result<(), String> {
    if task.contains(['/', '\\', ':'])
        || task == ".."
        || task == "."
        || task.trim().is_empty()
    {
        return Err(format!("非法 task 名: {task}"));
    }
    let home = app.path().home_dir().ok().ok_or("无法解析主目录")?;
    let root = office::workspaces_root(&home);
    let dir = root.join(&task);
    if !dir.is_dir() {
        return Err(format!("工作区不存在: {task}"));
    }
    let canonical = dunce::canonicalize(&dir).map_err(|e| format!("路径解析失败: {e}"))?;
    let canonical_root = dunce::canonicalize(&root).unwrap_or_else(|_| root.clone());
    // 等值排除:canonical == 根本身时 starts_with 也为 true,必须单独拒绝
    if canonical == canonical_root {
        return Err("非法:目标就是工作区根目录本身".to_string());
    }
    if !canonical.starts_with(&canonical_root) {
        return Err(format!(
            "路径越界:{} 不在 {} 内",
            canonical.display(),
            canonical_root.display()
        ));
    }
    std::fs::remove_dir_all(&canonical).map_err(|e| format!("删除失败: {e}"))
}

/// 指定任务的产物卡片(右区;切换工作区/初始拉取)。
#[tauri::command]
pub async fn office_artifacts(app: AppHandle, task: String) -> Result<Vec<ArtifactCard>, String> {
    let home = app.path().home_dir().ok().ok_or("无法解析主目录")?;
    office::read_manifest(&office::workspaces_root(&home), &task)
}

/// 用系统默认程序打开产物。签名是 (task, name) 而非裸路径(k3 P1a
/// 审计 P1):服务端重读 manifest,仅放行**已登记**的产物路径,
/// 再过扩展名白名单 + 本地盘符约束(根约束已放宽,见 open_allowed)。
#[tauri::command]
pub async fn office_open(app: AppHandle, task: String, name: String) -> Result<(), String> {
    let home = app.path().home_dir().ok().ok_or("无法解析主目录")?;
    let root = office::workspaces_root(&home);
    let manifest = office::read_manifest(&root, &task)?;
    let card = manifest
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| format!("产物 {name} 未登记于任务 {task}"))?;
    let canonical = office::open_allowed(&card.path)?;
    office::open_in_system(&canonical);
    Ok(())
}

/// 预览读取上限:标书正文 1-2MB 级,留足余量;超限引导走系统打开。
const PREVIEW_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// 预览读取产物字节(base64 返回)。与 office_open 同一道校验链:
/// manifest 已登记 → 扩展名白名单 → 本地盘符约束(read 链另剔除 html,
/// 见 open_allowed_for_read),另加大小上限。
/// `max_bytes` 允许调用方收紧上限(如 md 预览 2MB),在后端读文件前
/// 短路(k3 审计 Note5:避免超大文件白走 base64 IPC 传输+解码);
/// 上限恒被钳制在 PREVIEW_MAX_BYTES 内,不会放大。
#[tauri::command]
pub async fn office_read_file(
    app: AppHandle,
    task: String,
    name: String,
    max_bytes: Option<u64>,
) -> Result<String, String> {
    let home = app.path().home_dir().ok().ok_or("无法解析主目录")?;
    let root = office::workspaces_root(&home);
    let manifest = office::read_manifest(&root, &task)?;
    let card = manifest
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| format!("产物 {name} 未登记于任务 {task}"))?;
    let canonical = office::open_allowed_for_read(&card.path)?;
    let limit = max_bytes.unwrap_or(PREVIEW_MAX_BYTES).min(PREVIEW_MAX_BYTES);
    // 32MB 级读入 + base64 编码是重阻塞活,移到阻塞线程池执行,
    // 避免拖慢同进程的其他 IPC 命令(W6,step 交叉审计遗留项)。
    tauri::async_runtime::spawn_blocking(move || {
        // 预检给出带实际大小的明确错误;强制上限由下方 take() 保证——
        // metadata 与读取之间文件可能被替换膨胀,读后再验一次(deepseek 审计 W1)。
        let size = std::fs::metadata(&canonical)
            .map_err(|e| format!("读取元数据失败: {e}"))?
            .len();
        if size > limit {
            return Err(format!(
                "文件 {}MB 超过预览上限 {}MB",
                size / (1024 * 1024),
                limit / (1024 * 1024)
            ));
        }
        use std::io::Read as _;
        let file = std::fs::File::open(&canonical).map_err(|e| format!("打开文件失败: {e}"))?;
        let mut bytes = Vec::with_capacity(size as usize);
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("读取失败: {e}"))?;
        if bytes.len() as u64 > limit {
            return Err(format!(
                "文件在读取期间增长,超过预览上限 {}MB",
                limit / (1024 * 1024)
            ));
        }
        use base64::Engine as _;
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    })
    .await
    .map_err(|e| {
        // panic 与普通 join 失败分开记录,便于定位(step 审计 W1)
        match &e {
            tauri::Error::JoinError(je) if je.is_panic() => {
                tracing::error!("office_read_file 阻塞任务 panic");
                "预览读取内部错误".to_string()
            }
            _ => format!("预览读取任务失败: {e}"),
        }
    })?
}
