//! 内置 mock agent(`qidiwork-gui.exe --mock-agent`)。
//!
//! 用途:
//! 1. 传输层单元测试(`SpawnConfig::for_mock`)——完全离线,不依赖 qidi.exe;
//! 2. 手动联调 GUI↔桥管道:`cargo run -- --mock-agent` 后在真实窗口里
//!    走一遍 spawn/发送/接收/退出,不必启动真内核。
//!
//! 行为(回显模式):对 stdin 收到的每一行,先发一条通知再回一条应答:
//! ```json
//! {"jsonrpc":"2.0","method":"mock/event","params":{"seq":1}}
//! {"jsonrpc":"2.0","id":1,"result":{"echo":"<原行>"}}
//! ```
//! stdin EOF 后以 0 退出。`--sleep` 模式只睡 300s(手动测 Job/退出路径)。

use std::io::{BufRead, Write};

pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--sleep") {
        std::thread::sleep(std::time::Duration::from_secs(300));
        return 0;
    }

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut seq: u64 = 0;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.is_empty() {
            continue;
        }
        seq += 1;

        let notification =
            format!(r#"{{"jsonrpc":"2.0","method":"mock/event","params":{{"seq":{seq}}}}}"#);
        let id = serde_json::from_str::<serde_json::Value>(&line)
            .ok()
            .and_then(|v| v.get("id").cloned())
            .unwrap_or(serde_json::Value::Null);
        let response = format!(
            r#"{{"jsonrpc":"2.0","id":{},"result":{{"echo":{}}}}}"#,
            id,
            serde_json::Value::String(line)
        );

        if writeln!(stdout, "{notification}").is_err()
            || writeln!(stdout, "{response}").is_err()
            || stdout.flush().is_err()
        {
            // stdout 断开(父进程死亡)→ 正常退出
            return 0;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #[test]
    fn response_format_roundtrip() {
        // 快速校验 mock 的应答 JSON 是合法的、id 保真。
        let line = r#"{"jsonrpc":"2.0","id":42,"method":"x"}"#;
        let id = serde_json::from_str::<serde_json::Value>(line)
            .unwrap()
            .get("id")
            .cloned()
            .unwrap();
        let response = format!(
            r#"{{"jsonrpc":"2.0","id":{},"result":{{"echo":{}}}}}"#,
            id,
            serde_json::Value::String(line.to_string())
        );
        let v: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(v["id"], 42);
        assert_eq!(v["result"]["echo"], line);
    }
}
