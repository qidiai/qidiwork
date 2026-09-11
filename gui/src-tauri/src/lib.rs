//! QidiWork GUI 库主体(Tauri 官方 lib+bin 布局:逻辑在 lib,main 是壳)。
//!
//! 拆 lib 的决定性原因:单元测试二进制的 `current_exe` 是测试 harness,
//! `--mock-agent` 分支不可达;拆分后集成测试经 `CARGO_BIN_EXE_qidiwork-gui`
//! 拿到真实应用二进制,传输层测试spawn 真实 exe 的 mock 分支。

pub mod acp;
pub mod commands;
pub mod logging;
#[cfg(any(test, debug_assertions))]
pub mod mock_agent; // 生产 release 不暴露 --mock-agent 分支(缩小暴露面)
pub mod office;
pub mod persist;
pub mod process;
pub mod settings;
pub mod skills;
pub mod transport;

use tauri::{Manager, RunEvent};

/// Tauri 应用装配与主循环,返回进程退出码。
pub fn run() -> i32 {
    let builder = tauri::Builder::default()
        // single-instance 必须第一个注册(审计决策:办公场景禁止多开,
        // 多开 = 多 agent 进程 + office-workspaces 重复 fsnotify)。
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .invoke_handler(tauri::generate_handler![
            app_version,
            commands::agent_start,
            commands::agent_stop,
            commands::agent_status,
            commands::session_start,
            commands::session_prompt,
            commands::session_cancel,
            commands::permission_respond,
            commands::permission_cancel,
            commands::agent_recover,
            commands::office_scan,
            commands::office_artifacts,
            commands::office_open,
            commands::office_read_file,
            commands::office_watch_start,
            commands::skills_list,
            commands::settings_read,
            commands::settings_save
        ])
        .setup(|app| {
            logging::init(app.path().app_log_dir().ok());
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "QidiWork GUI 启动");
            Ok(())
        })
        .manage(commands::AgentState::default())
        .manage(commands::BridgeState::default())
        .manage(commands::OfficeState::default());

    let app = match builder.build(tauri::generate_context!()) {
        Ok(app) => app,
        Err(e) => {
            // 显式决策:启动失败无可恢复路径,只能退出(官方模板语义)。
            eprintln!("Tauri 应用构建失败: {e}");
            return 1;
        }
    };

    app.run(|app_handle, event| {
        if let RunEvent::ExitRequested { .. } = event {
            // 退出时主动终止 agent 子进程树(方案 v2 §D4);
            // Job Object KILL_ON_JOB_CLOSE 是崩溃场景的兜底。
            commands::terminate_current(&app_handle.state::<commands::AgentState>());
        }
    });
    0
}

/// IPC 冒烟命令:前端取回 GUI 版本号(M1)。
#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
