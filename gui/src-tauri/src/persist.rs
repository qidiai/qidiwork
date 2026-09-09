//! 会话登记持久化(k3 M3 审计登记:M3 只做进程内恢复,GUI 自身重启
//! 即丢;本模块把会话清单落盘,启动/recover 时可续接)。
//!
//! 存储:`<app_data_dir>/sessions.json`,原子写(临时文件 + rename)。
//! 只存 {session_id, cwd}——对话内容仍由 agent 侧 `~/.qidi/sessions`
//! 持有,GUI 重启后经 `agent_recover` → session/load 续接。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 单条持久化会话记录。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedSession {
    pub session_id: String,
    pub cwd: String,
}

/// 读全部持久化会话。文件不存在/损坏 → 空列表(损坏文件改名留证)。
pub fn load_sessions(dir: &Path) -> Vec<PersistedSession> {
    let path = dir.join("sessions.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<PersistedSession>>(&raw) {
        Ok(list) => list,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "sessions.json 损坏,按空处理");
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = std::fs::rename(&path, dir.join(format!("sessions.json.corrupt-{stamp}")));
            Vec::new()
        }
    }
}

/// 全量覆盖写(原子:临时文件 + 持久化 rename)。
pub fn save_sessions(dir: &Path, sessions: &[PersistedSession]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("sessions.json");
    let tmp = dir.join("sessions.json.tmp");
    let json = serde_json::to_string_pretty(sessions).unwrap_or_else(|_| "[]".to_string());
    std::fs::write(&tmp, json)?;
    // 落盘前刷盘(k3 M4 审计 §5:防断电零字节),再原子替换
    if let Ok(f) = std::fs::File::open(&tmp) {
        let _ = f.sync_all();
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// 追加一条(去重按 session_id:重复加载不产生重复行)。
pub fn upsert_session(dir: &Path, session: PersistedSession) -> std::io::Result<()> {
    let mut list = load_sessions(dir);
    list.retain(|s| s.session_id != session.session_id);
    list.push(session);
    save_sessions(dir, &list)
}

/// 移除一条(恢复失败/用户关闭会话时)。
pub fn remove_session(dir: &Path, session_id: &str) -> std::io::Result<()> {
    let mut list = load_sessions(dir);
    list.retain(|s| s.session_id != session_id);
    save_sessions(dir, &list)
}

/// 便于测试:使用调用方给定的目录。
pub fn temp_dir_for_test() -> Option<PathBuf> {
    Some(std::env::temp_dir().join(format!("qidiwork-test-{}", std::process::id())))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn fresh_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("qidiwork-persist-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn load_missing_is_empty() {
        let dir = fresh_dir("missing");
        assert!(load_sessions(&dir).is_empty());
    }

    #[test]
    fn upsert_and_roundtrip() {
        let dir = fresh_dir("upsert");
        upsert_session(
            &dir,
            PersistedSession {
                session_id: "a".into(),
                cwd: "C:\\ws".into(),
            },
        )
        .unwrap();
        upsert_session(
            &dir,
            PersistedSession {
                session_id: "a".into(),
                cwd: "C:\\ws2".into(),
            },
        )
        .unwrap();
        upsert_session(
            &dir,
            PersistedSession {
                session_id: "b".into(),
                cwd: "C:\\ws".into(),
            },
        )
        .unwrap();
        let list = load_sessions(&dir);
        assert_eq!(list.len(), 2, "同 id 去重");
        assert!(
            list.iter()
                .any(|s| s.session_id == "a" && s.cwd == "C:\\ws2")
        );
        assert!(
            !dir.join("sessions.json.tmp").exists(),
            "临时文件应已 rename"
        );
    }

    #[test]
    fn corrupt_file_quarantined() {
        let dir = fresh_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sessions.json"), "not json").unwrap();
        assert!(load_sessions(&dir).is_empty());
        assert!(
            dir.read_dir().unwrap().filter_map(|e| e.ok()).any(|e| e
                .file_name()
                .to_string_lossy()
                .starts_with("sessions.json.corrupt")),
            "损坏文件应改名留证(带时间戳)"
        );
    }
}
