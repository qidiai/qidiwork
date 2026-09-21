//! 会话转录回放(R5):GUI **只读**直读内核会话文件,零内核改动、零协议改动。
//!
//! 内核把每会话的对话落在 `<qidi_home>/sessions/<urlencoded-cwd>/<session-id>/`:
//! - `chat_history.jsonl`:每行一条消息 `{"type":"system|user|assistant|...","content":...}`
//!   (Anthropic 消息格式,与实时流同构——前端复用现有渲染管线)
//! - `summary.json`:`session_summary`(会话标题)/`num_messages`/`updated_at` 等元信息
//!
//! 本模块只提供三个只读命令:尾部回放 / 更早分页 / 会话摘要。大会话(>2MB)
//! 尾部截取,避免全量解析拖慢首屏。路径段编码手写 percent-encode,与内核
//! `urlencoding::encode` 规则对齐(非 `[A-Za-z0-9-_.~]` 一律 `%XX` 大写十六进制),
//! **不引入任何新依赖**。

use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};

/// 尾部读取窗口:文件超过该阈值时只 seek 末尾 2MB(丢弃不完整首行)。
const TAIL_WINDOW_BYTES: u64 = 2 * 1024 * 1024;
/// 尾部默认条数(前端可覆盖)。
const DEFAULT_TAIL_LIMIT: usize = 50;
/// 更早分页默认条数。
const DEFAULT_EARLIER_LIMIT: usize = 200;

/// 一次回放/分页的返回结构(前端负责把 `messages` 渲染进转录区)。
#[derive(Debug, Serialize)]
pub struct TranscriptPage {
    /// 原始消息(内核 `chat_history.jsonl` 原样,serde_json::Value 数组)。
    pub messages: Vec<serde_json::Value>,
    /// 会话消息总数(取自 summary.json 的 `num_messages`;缺失时为 0)。
    pub total_messages: u64,
    /// 尾部读取时文件超过窗口被截断(前端据此提示)。
    pub truncated: bool,
    /// 会话文件缺失(前端据此静默降级)。
    pub missing: bool,
    /// 本次返回消息中最早一条的**全局行序号**(0 基,行号空间),供前端翻页。
    pub loaded_upto: u64,
    /// 会话文件字节数(前端「加载全部」前的大文件确认)。
    pub size_bytes: u64,
}

impl TranscriptPage {
    /// 文件缺失时的空页。
    fn missing() -> Self {
        Self {
            messages: Vec::new(),
            total_messages: 0,
            truncated: false,
            missing: true,
            loaded_upto: 0,
            size_bytes: 0,
        }
    }
}

/// 会话摘要(历史列表标题升级用);字段缺失一律 `null`。
#[derive(Debug, Default, Serialize)]
pub struct SessionSummaryInfo {
    pub title: Option<String>,
    pub num_messages: Option<u64>,
    pub updated_at: Option<String>,
}

// ------------------------------------------------------------- 路径解析 --

/// 内核 home:`QIDI_HOME` 优先,否则 `<home_dir>/.qidi`(与 diag.rs `qidi_home` 一致)。
fn qidi_home(app: &AppHandle) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("QIDI_HOME") {
        let p = PathBuf::from(p);
        if !p.as_os_str().is_empty() {
            return Some(p);
        }
    }
    app.path().home_dir().ok().map(|h| h.join(".qidi"))
}

/// percent-encode 路径段:非 `[A-Za-z0-9-_.~]` 一律 `%XX`(大写十六进制),
/// 与内核 `urlencoding::encode` 一致(如 `G:\qidicode` → `G%3A%5Cqidicode`)。
fn percent_encode(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(input.len());
    for &b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
        }
    }
    out
}

/// 定位某会话的目录。`session_id` 做最小校验(拒绝空/分隔符/点路径),
/// 防止越界读取;`cwd` 经 percent-encode 后是单个路径段,天然不可穿越。
fn session_dir(app: &AppHandle, session_id: &str, cwd: &str) -> Option<PathBuf> {
    if session_id.is_empty() || session_id.contains(['/', '\\']) || session_id.contains("..") {
        return None;
    }
    let home = qidi_home(app)?;
    // Audit hardening: `.` and `..` survive percent-encoding verbatim.
    let encoded = percent_encode(cwd);
    if encoded == "." || encoded == ".." {
        return None;
    }
    Some(home.join("sessions").join(encoded).join(session_id))
}

// --------------------------------------------------------------- 读取 --

/// 从 summary.json 读会话消息总数(缺失/无字段 → 0)。
fn read_total_messages(dir: &Path) -> u64 {
    std::fs::read_to_string(dir.join("summary.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("num_messages").and_then(|x| x.as_u64()))
        .unwrap_or(0)
}

/// 统计 `[0, end)` 字节区间内的换行数(用于把尾部行号还原为全局行号)。
fn count_newlines_prefix(path: &Path, end: u64) -> u64 {
    let Ok(mut f) = std::fs::File::open(path) else {
        return 0;
    };
    let mut remaining = end;
    let mut count = 0u64;
    let mut buf = vec![0u8; 1 << 16];
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        match f.read(&mut buf[..want]) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                count += buf[..n].iter().filter(|&&b| b == b'\n').count() as u64;
                remaining -= n as u64;
            }
        }
    }
    count
}

/// 解析 JSONL:逐行 `serde_json`,坏行跳过并计数。
/// 返回 `(消息, 全局行号)` 列表与坏行数;`base_line` 是首行的全局行号。
fn parse_jsonl(text: &str, base_line: u64) -> (Vec<(u64, serde_json::Value)>, u64) {
    let mut out = Vec::new();
    let mut bad = 0u64;
    for (i, raw) in text.split('\n').enumerate() {
        let line = raw.trim_end_matches('\r').trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(v) => out.push((base_line + i as u64, v)),
            Err(_) => bad += 1,
        }
    }
    (out, bad)
}

/// 尾部读取:文件 >2MB 时 seek 末尾 2MB(丢弃不完整首行),取最后 `limit` 条。
fn tail_page(path: &Path, limit: usize, total_messages: u64) -> Result<TranscriptPage, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let truncated = size > TAIL_WINDOW_BYTES;

    let (text, base_line) = if truncated {
        // 窗口起点前的换行数 = 被切半那行的行号;丢弃该行后,下一条完整行
        // 的全局行号 = 前缀换行数 + 1(与整文件 split('\n') 的行号空间一致)。
        let prefix_lines = count_newlines_prefix(path, size - TAIL_WINDOW_BYTES);
        let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
        // Audit hardening: probe the byte right before the window. If it is a
        // newline the window starts on a complete line and nothing is dropped.
        f.seek(SeekFrom::Start(size - TAIL_WINDOW_BYTES - 1))
            .map_err(|e| e.to_string())?;
        let mut prev = [0u8; 1];
        f.read_exact(&mut prev).map_err(|e| e.to_string())?;
        let at_line_start = prev[0] == b'\n';
        f.seek(SeekFrom::End(-(TAIL_WINDOW_BYTES as i64)))
            .map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        let s = String::from_utf8_lossy(&buf).into_owned();
        if at_line_start {
            (s, prefix_lines)
        } else {
            let s = match s.find('\n') {
                Some(i) => s[i + 1..].to_string(),
                None => String::new(),
            };
            (s, prefix_lines + 1)
        }
    } else {
        let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        (String::from_utf8_lossy(&buf).into_owned(), 0)
    };

    let (mut msgs, _bad) = parse_jsonl(&text, base_line);
    let start = msgs.len().saturating_sub(limit);
    let page_msgs = msgs.split_off(start);
    let loaded_upto = page_msgs.first().map(|(idx, _)| *idx).unwrap_or(0);
    Ok(TranscriptPage {
        messages: page_msgs.into_iter().map(|(_, v)| v).collect(),
        total_messages,
        truncated,
        missing: false,
        loaded_upto,
        size_bytes: size,
    })
}

/// 更早分页:全量读后按行切片,取全局行号 `< before` 的最后 `limit` 条。
fn earlier_page(
    path: &Path,
    before: u64,
    limit: usize,
    total_messages: u64,
) -> Result<TranscriptPage, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let size = bytes.len() as u64;
    // 环形缓冲:只保留窗口内最后 limit 条,避免整文件消息同时驻留。
    let mut ring: VecDeque<(u64, serde_json::Value)> = VecDeque::new();
    for (i, raw) in bytes.split(|&b| b == b'\n').enumerate() {
        let idx = i as u64;
        if idx >= before {
            break;
        }
        let line = String::from_utf8_lossy(raw);
        let line = line.trim_end_matches('\r').trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            ring.push_back((idx, v));
            if ring.len() > limit {
                ring.pop_front();
            }
        }
    }
    let loaded_upto = ring.front().map(|(i, _)| *i).unwrap_or(0);
    Ok(TranscriptPage {
        messages: ring.into_iter().map(|(_, v)| v).collect(),
        total_messages,
        truncated: false,
        missing: false,
        loaded_upto,
        size_bytes: size,
    })
}

// -------------------------------------------------------------- 命令 --

/// 会话尾部转录(续接/崩溃恢复时回放最近 limit 条,默认 50)。
#[tauri::command]
pub async fn session_transcript_tail(
    app: AppHandle,
    session_id: String,
    cwd: String,
    limit: Option<usize>,
) -> Result<TranscriptPage, String> {
    let limit = limit.unwrap_or(DEFAULT_TAIL_LIMIT);
    let dir = session_dir(&app, &session_id, &cwd).ok_or_else(|| "会话路径不可用".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = dir.join("chat_history.jsonl");
        if !path.is_file() {
            return Ok(TranscriptPage::missing());
        }
        let total = read_total_messages(&dir);
        tail_page(&path, limit, total)
    })
    .await
    .map_err(|e| format!("转录读取任务失败: {e}"))?
}

/// 更早转录分页(折叠横幅「加载全部」/「加载更早 N 条」)。
/// `before` = 当前最早已加载行的全局行号;返回 `[before-limit, before)` 区间。
#[tauri::command]
pub async fn session_transcript_earlier(
    app: AppHandle,
    session_id: String,
    cwd: String,
    before: u64,
    limit: Option<usize>,
) -> Result<TranscriptPage, String> {
    let limit = limit.unwrap_or(DEFAULT_EARLIER_LIMIT);
    let dir = session_dir(&app, &session_id, &cwd).ok_or_else(|| "会话路径不可用".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = dir.join("chat_history.jsonl");
        if !path.is_file() {
            return Ok(TranscriptPage::missing());
        }
        let total = read_total_messages(&dir);
        earlier_page(&path, before, limit, total)
    })
    .await
    .map_err(|e| format!("转录读取任务失败: {e}"))?
}

/// 会话摘要(session_summary 标题 / num_messages / updated_at;缺失全 null)。
#[tauri::command]
pub async fn session_summary(
    app: AppHandle,
    session_id: String,
    cwd: String,
) -> Result<SessionSummaryInfo, String> {
    let dir = session_dir(&app, &session_id, &cwd).ok_or_else(|| "会话路径不可用".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = dir.join("summary.json");
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return Ok(SessionSummaryInfo::default());
        };
        let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        Ok(SessionSummaryInfo {
            title: v
                .get("session_summary")
                .and_then(|x| x.as_str())
                .map(str::to_string),
            num_messages: v.get("num_messages").and_then(|x| x.as_u64()),
            updated_at: v
                .get("updated_at")
                .and_then(|x| x.as_str())
                .map(str::to_string),
        })
    })
    .await
    .map_err(|e| format!("会话摘要读取任务失败: {e}"))?
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn tmp_file(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "qidiwork-transcript-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("chat_history.jsonl")
    }

    #[test]
    #[test]
    fn tail_seek_at_line_boundary_keeps_first_line() {
        let path = tmp_file("tail-boundary");
        // Exact 1024-byte lines: size - 2MB then always lands on a line start.
        let line_len = 1024usize;
        let n = 3000usize;
        let mut s = String::new();
        for i in 0..n {
            let mut line = format!("{{\"type\":\"user\",\"content\":\"m{i:05}");
            while line.len() + 3 < line_len {
                line.push('x');
            }
            line.push_str("\"}\n");
            assert_eq!(line.len(), line_len);
            s.push_str(&line);
        }
        std::fs::write(&path, &s).unwrap();
        let page = tail_page(&path, 3000, n as u64).unwrap();
        assert!(page.truncated, "3MB file must be truncated");
        // Window covers lines 952..=2999 (0-based); none may be dropped.
        assert_eq!(page.messages.len(), 2048, "all window lines returned");
        assert!(
            page.messages[0]["content"]
                .as_str()
                .unwrap()
                .starts_with("m00952"),
            "boundary line m00952 must survive"
        );
        assert_eq!(page.loaded_upto, 952, "first window line index kept");
    }
    fn percent_encode_matches_kernel_urlencoding() {
        // 内核样例:urlencoding::encode("G:\\qidicode") = "G%3A%5Cqidicode"
        assert_eq!(percent_encode("G:\\qidicode"), "G%3A%5Cqidicode");
        // 空格 → %20
        assert_eq!(percent_encode("a b"), "a%20b");
        // 中文 → 逐字节多组大写 %XX
        assert_eq!(percent_encode("中文"), "%E4%B8%AD%E6%96%87");
        // 非保留字符原样保留
        assert_eq!(percent_encode("Az-9_.~"), "Az-9_.~");
    }

    #[test]
    fn parse_jsonl_skips_bad_lines() {
        let text = "{\"type\":\"user\",\"content\":\"hi\"}\nnot json\n\n{\"type\":\"assistant\",\"content\":\"yo\"}\n";
        let (msgs, bad) = parse_jsonl(text, 0);
        assert_eq!(msgs.len(), 2, "坏行与空行被跳过");
        assert_eq!(bad, 1, "坏行计数为 1");
        assert_eq!(msgs[0].1["type"], "user");
        assert_eq!(msgs[1].1["type"], "assistant");
        // 行号按物理行推进:坏行/空行不产生消息但占用行号,故第二条落在第 3 行
        assert_eq!(msgs[1].0, 3, "行号按物理行推进(坏行/空行不占消息但占行号)");
    }

    #[test]
    fn tail_takes_last_limit_no_truncation() {
        let path = tmp_file("tail-small");
        let mut s = String::new();
        for i in 0..5 {
            s.push_str(&format!("{{\"type\":\"user\",\"content\":\"m{i}\"}}\n"));
        }
        std::fs::write(&path, &s).unwrap();
        let page = tail_page(&path, 2, 5).unwrap();
        assert!(!page.truncated);
        assert_eq!(page.messages.len(), 2);
        assert_eq!(page.messages[0]["content"], "m3");
        assert_eq!(page.messages[1]["content"], "m4");
        assert_eq!(page.loaded_upto, 3, "窗口首条的全局行号");
        assert_eq!(page.total_messages, 5);
    }

    #[test]
    fn tail_seek_drops_partial_first_line_when_truncated() {
        let path = tmp_file("tail-big");
        // 每行 ~1KB,写 3000 行 ≈ 3MB > 2MB 窗口
        let pad = "x".repeat(1000);
        let mut s = String::new();
        for i in 0..3000 {
            s.push_str(&format!("{{\"type\":\"user\",\"content\":\"m{i}{pad}\"}}\n"));
        }
        std::fs::write(&path, &s).unwrap();
        let page = tail_page(&path, 2, 3000).unwrap();
        assert!(page.truncated, "超过 2MB 应标记截断");
        assert_eq!(page.messages.len(), 2);
        assert!(
            page.messages[0]["content"]
                .as_str()
                .unwrap()
                .starts_with("m2998"),
            "尾部窗口内倒数第二条应为 m2998"
        );
        assert!(
            page.messages[1]["content"]
                .as_str()
                .unwrap()
                .starts_with("m2999"),
            "最后一条应为 m2999"
        );
        // 行号被还原到全局空间(接近文件末尾),而非窗口内相对行号
        assert_eq!(page.loaded_upto, 2998, "全局行号还原正确");
    }

    #[test]
    fn earlier_slices_before_offset() {
        let path = tmp_file("earlier");
        let mut s = String::new();
        for i in 0..10 {
            s.push_str(&format!("{{\"type\":\"user\",\"content\":\"m{i}\"}}\n"));
        }
        std::fs::write(&path, &s).unwrap();
        // 取 [5-3, 5) = m2,m3,m4
        let page = earlier_page(&path, 5, 3, 10).unwrap();
        assert_eq!(page.messages.len(), 3);
        assert_eq!(page.messages[0]["content"], "m2");
        assert_eq!(page.messages[2]["content"], "m4");
        assert_eq!(page.loaded_upto, 2);
        // before=0 → 空(前端「加载全部」传 before=earliest)
        let empty = earlier_page(&path, 0, 200, 10).unwrap();
        assert!(empty.messages.is_empty());
    }
}
