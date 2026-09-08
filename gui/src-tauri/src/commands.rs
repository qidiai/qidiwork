//! Tauri IPC 命令层:agent 子进程生命周期管理(M2 基座)。
//!
//! M3 的 ACP 桥将在 `AgentTransport` 之上工作,不复用这些裸命令;
//! 这里的行级命令(`agent_send` / `agent-line` 事件)主要服务于
//! M2 验收与手动联调,不是长期对前端契约。

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::process::{AgentProcess, SpawnConfig};
use crate::transport::AgentTransport as _;

/// 全局 agent 句柄。None = 未启动/已退出。
/// 用 tokio Mutex(而非 std):agent_start 需要跨 spawn 全程持锁,
/// 防止并发调用双开 agent 进程(自查发现)。
#[derive(Default)]
pub struct AgentState {
    inner: tokio::sync::Mutex<Option<Arc<AgentProcess>>>,
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

/// 启动 agent 子进程(幂等:已在运行则直接返回现状)。
/// 事件:每行输出 → `agent-line`;退出 → `agent-exit`。
#[tauri::command]
pub async fn agent_start(
    app: AppHandle,
    state: State<'_, AgentState>,
) -> Result<AgentStatusInfo, String> {
    let mut guard = state.inner.lock().await;
    if let Some(p) = guard.as_ref() {
        if p.is_running() {
            return Ok(status_of(Some(p)));
        }
    }

    let home = app.path().home_dir().ok();
    let process = AgentProcess::spawn(SpawnConfig::default_agent(home))
        .await
        .map_err(|e| e.to_string())?;

    // 入站行转发:可靠队列(k3 M2 审计 P0-2)的唯一消费者在本阶段是
    // Tauri 事件转发;M3 起 ACP 桥接管队列,展示事件由桥派生。
    let line_task_proc = Arc::clone(&process);
    let line_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut rx = match line_task_proc.take_line_receiver() {
            Ok(rx) => rx,
            Err(e) => {
                tracing::error!("take_line_receiver 失败: {e}");
                return;
            }
        };
        while let Some(line) = rx.recv().await {
            let _ = line_app.emit("agent-line", line);
        }
    });

    // 退出转发 + 状态清理:agent 崩溃后 GUI 可感知(M3 在此触发
    // 重 spawn + session/load 恢复,见方案 v2 §D2)。
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
        if let Ok(mut guard) = exit_app.state::<AgentState>().inner.try_lock() {
            // 仅当仍是这个进程时清理(避免误删重启后的新实例)。
            if guard
                .as_ref()
                .is_some_and(|p| Arc::ptr_eq(p, &exit_task_proc))
            {
                *guard = None;
            }
        }
    });

    *guard = Some(Arc::clone(&process));
    drop(guard);
    tracing::info!(pid = process.pid(), "agent_start 完成");
    Ok(status_of(Some(&process)))
}

/// 终止 agent(整树)并等待退出。
#[tauri::command]
pub async fn agent_stop(state: State<'_, AgentState>) -> Result<AgentStatusInfo, String> {
    let proc = state.inner.lock().await.take();
    match proc {
        Some(p) => {
            p.shutdown().await;
            Ok(status_of(None))
        }
        None => Ok(status_of(None)),
    }
}

/// 发送一行 JSON-RPC(M2 验收/联调用)。
#[tauri::command]
pub async fn agent_send(state: State<'_, AgentState>, line: String) -> Result<(), String> {
    let proc = state.inner.lock().await.clone();
    match proc {
        Some(p) if p.is_running() => p.send(line).map_err(|e| e.to_string()),
        _ => Err("agent 未运行".into()),
    }
}

#[tauri::command]
pub async fn agent_status(state: State<'_, AgentState>) -> Result<AgentStatusInfo, String> {
    let guard = state.inner.lock().await;
    Ok(status_of(guard.as_ref()))
}

/// 退出路径的同步清理(main.rs 的 ExitRequested 钩子调用):
/// Job Object terminate 是同步整树回收,不等待进程完全退出
/// (应用退出场景下由内核兜底,方案 v2 §D4)。
pub fn terminate_current(state: &AgentState) {
    // 退出钩子是同步上下文:try_lock 失败(正被 agent_start 持有)时
    // 依赖 Job Object kill-on-close 兜底,不阻塞退出。
    if let Ok(guard) = state.inner.try_lock()
        && let Some(p) = guard.as_ref()
    {
        p.terminate();
    }
}
