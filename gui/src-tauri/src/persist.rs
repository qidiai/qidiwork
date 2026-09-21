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
    /// 会话标题(首条任务截断,前端在首条 prompt 后回写)。
    /// 旧版 sessions.json 无此字段,serde default 保持可读。
    #[serde(default)]
    pub title: Option<String>,
    /// 绑定的办公任务工作区名(session/new|load 的 `office_task`,即产物
    /// 登记 card.py 的 `--task` 归属)。旧版 sessions.json 无此字段,
    /// serde default 保持可读。
    #[serde(default)]
    pub task: Option<String>,
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

/// 清空登记簿。archive=true 时先改名留档(sessions-archive-<时间戳>.json)
/// 再清空,误删可找回。仅清 GUI 登记簿;agent 侧 ~/.qidi/sessions 的
/// 对话内容不受影响。返回给用户的提示信息。
pub fn clear_sessions(dir: &Path, archive: bool) -> Result<String, std::io::Error> {
    let path = dir.join("sessions.json");
    if !path.exists() {
        return Ok("没有历史会话".to_string());
    }
    if archive {
        // 毫秒级:同一秒内"存新会话→清空"会在 Windows rename 上撞名(k3 审计)
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let archived = dir.join(format!("sessions-archive-{stamp}.json"));
        std::fs::rename(&path, &archived)?;
        return Ok(format!(
            "已清空历史会话,原登记簿归档为 {}",
            archived.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    std::fs::remove_file(&path)?;
    Ok("已清空历史会话".to_string())
}

/// 设置会话标题(不存在则 NotFound)。
pub fn set_title(dir: &Path, session_id: &str, title: &str) -> std::io::Result<()> {
    let mut list = load_sessions(dir);
    match list.iter_mut().find(|s| s.session_id == session_id) {
        Some(s) => {
            s.title = Some(title.to_string());
            save_sessions(dir, &list)
        }
        None => Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "会话未登记",
        )),
    }
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
                title: None,
                task: None,
            },
        )
        .unwrap();
        upsert_session(
            &dir,
            PersistedSession {
                session_id: "a".into(),
                cwd: "C:\\ws2".into(),
                title: None,
                task: None,
            },
        )
        .unwrap();
        upsert_session(
            &dir,
            PersistedSession {
                session_id: "b".into(),
                cwd: "C:\\ws".into(),
                title: None,
                task: None,
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
    #[test]
    fn set_title_roundtrip() {
        let dir = fresh_dir("title");
        upsert_session(
            &dir,
            PersistedSession {
                session_id: "s1".into(),
                cwd: "C:\\ws".into(),
                title: None,
                task: None,
            },
        )
        .unwrap();
        set_title(&dir, "s1", "写标书任务").unwrap();
        let list = load_sessions(&dir);
        assert_eq!(list[0].title.as_deref(), Some("写标书任务"));
        // 未登记会话报错
        assert!(set_title(&dir, "ghost", "x").is_err());
    }

    #[test]
    fn clear_sessions_archives_then_empties() {
        let dir = fresh_dir("clear");
        upsert_session(
            &dir,
            PersistedSession {
                session_id: "s1".into(),
                cwd: "C:\\ws".into(),
                title: None,
                task: None,
            },
        )
        .unwrap();
        let msg = clear_sessions(&dir, true).unwrap();
        assert!(msg.contains("归档"));
        assert!(!dir.join("sessions.json").exists(), "登记簿已清空");
        let archived: Vec<_> = dir
            .read_dir()
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("sessions-archive-"))
            .collect();
        assert_eq!(archived.len(), 1, "归档文件存在");
        // 再清一次:没有可清的,提示但不报错
        let msg2 = clear_sessions(&dir, false).unwrap();
        assert!(msg2.contains("没有历史会话"));
    }

    #[test]
    fn legacy_file_without_title_field_loads() {
        let dir = fresh_dir("legacy");
        std::fs::create_dir_all(&dir).unwrap();
        // 旧版字段集(无 title):serde default 必须可读
        std::fs::write(
            dir.join("sessions.json"),
            r#"[{"session_id":"old","cwd":"C:\\ws"}]"#,
        )
        .unwrap();
        let list = load_sessions(&dir);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, None);
    }

    /// 旧版 sessions.json(无 task 字段)必须能反序列化:serde default
    /// 保持向后兼容,读到的绑定为 None。
    #[test]
    fn legacy_file_without_task_field_loads() {
        let dir = fresh_dir("legacy-task");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("sessions.json"),
            r#"[{"session_id":"old","cwd":"C:\\ws","title":"周报"}]"#,
        )
        .unwrap();
        let list = load_sessions(&dir);
        assert_eq!(list.len(), 1, "无 task 字段的旧 JSON 必须可读");
        assert_eq!(list[0].task, None, "缺省即未绑定");
        assert_eq!(list[0].title.as_deref(), Some("周报"));
    }

    /// 带绑定的会话落盘后原样读回(写侧支持 task 字段)。
    #[test]
    fn upsert_roundtrip_preserves_task_binding() {
        let dir = fresh_dir("task-roundtrip");
        upsert_session(
            &dir,
            PersistedSession {
                session_id: "s1".into(),
                cwd: "C:\\ws".into(),
                title: None,
                task: Some("写标书".into()),
            },
        )
        .unwrap();
        let list = load_sessions(&dir);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].task.as_deref(), Some("写标书"));
    }

}
