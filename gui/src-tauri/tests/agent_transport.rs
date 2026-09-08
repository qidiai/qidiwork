#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! AgentProcess 传输层集成测试。
//!
//! 关键点:被 spawn 的是 **真实应用二进制**(cargo 官方 `CARGO_BIN_EXE_*`
//! 编译期变量指向 target/debug/qidiwork-gui.exe),走其 `--mock-agent`
//! 分支。不能在单元测试里用 current_exe——那是测试 harness 二进制,
//! 它的 main 是 libtest,不是我们的 mock 分支(本项目踩过的坑)。

use qidiwork_gui::process::{AgentProcess, SpawnConfig};
use qidiwork_gui::transport::AgentTransport as _;

fn mock_cfg() -> SpawnConfig {
    SpawnConfig {
        program: env!("CARGO_BIN_EXE_qidiwork-gui").to_string(),
        args: vec!["--mock-agent".into()],
        cwd: None,
    }
}

async fn recv_line(rx: &mut tokio::sync::mpsc::Receiver<String>) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
        .await
        .expect("等待入站行超时")
        .unwrap()
}

#[tokio::test]
async fn spawn_mock_echo_roundtrip() {
    let proc = AgentProcess::spawn(mock_cfg()).await.unwrap();
    assert!(proc.is_running());
    let mut rx = proc.take_line_receiver().unwrap();

    let lines = vec![
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":2,"method":"session/new"}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":3,"method":"session/prompt"}"#.to_string(),
    ];
    for line in &lines {
        proc.send(line.clone()).unwrap();
    }

    // mock 对每行先发一条通知再回显应答 → 共 6 行,应答 id 可回查。
    let mut ids = Vec::new();
    for _ in 0..(lines.len() * 2) {
        let line = recv_line(&mut rx).await;
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        if v.get("result").is_some() {
            ids.push(v["id"].as_i64().unwrap());
        }
    }
    assert_eq!(ids, vec![1, 2, 3]);

    proc.shutdown().await;
    assert!(!proc.is_running());
}

#[tokio::test]
async fn shutdown_broadcasts_exit() {
    let proc = AgentProcess::spawn(mock_cfg()).await.unwrap();
    let mut exit_rx = proc.subscribe_exit();
    proc.shutdown().await;
    let info = tokio::time::timeout(std::time::Duration::from_secs(5), exit_rx.recv())
        .await
        .expect("等待退出通知超时")
        .unwrap();
    // Job Terminate / start_kill 的退出码非 0,但必须收到通知,
    // 且 success=false(M3 据此走崩溃恢复路径)。
    assert!(!info.success, "强杀退出不应标记为 success");
}

#[tokio::test]
async fn send_after_exit_reports_closed() {
    let proc = AgentProcess::spawn(mock_cfg()).await.unwrap();
    proc.shutdown().await;
    // 进程退出后发送应得到 Closed。writer 任务发现管道破裂需要被调度
    // (current_thread 测试运行时),所以每次尝试间让出一下。
    for _ in 0..20 {
        if proc.send("ping".into()).is_err() {
            return; // 观察到 Closed ✓
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("shutdown 后发送应返回 Closed");
}

#[tokio::test]
async fn line_receiver_is_single_consumer() {
    let proc = AgentProcess::spawn(mock_cfg()).await.unwrap();
    let _rx = proc.take_line_receiver().unwrap();
    assert!(
        proc.take_line_receiver().is_err(),
        "第二次取用必须失败(协议帧不允许被分流)"
    );
    proc.shutdown().await;
}
