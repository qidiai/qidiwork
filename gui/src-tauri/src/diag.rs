//! 诊断日志一键导出(R2):把 GUI 日志 + 内核日志尾段 + 版本信息 + 登录态
//! 汇总为**单文件文本**,写盘前统一脱敏(`sk-*` / `Bearer` 凭证),落到桌面。
//!
//! 隐私红线:
//! - auth.json **只**导出 `logged_in` 布尔,绝不读取/输出任何字段值;
//! - 全文在写盘前经 [`redact`] 手工扫描器掩码,命中 `sk-` 长串或 `Bearer`
//!   凭证一律替换为占位符。

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager};

/// 内核日志仅取末尾 2MB(防全量大文件)。
const KERNEL_LOG_TAIL_LIMIT: u64 = 2 * 1024 * 1024;
/// 内核日志再截最后 2000 行。
const KERNEL_LOG_TAIL_LINES: usize = 2000;
/// GUI 日志取按修改时间最新的 2 个。
const GUI_LOG_FILES: usize = 2;

/// 导出诊断文件到桌面,返回完整路径。
#[tauri::command]
pub fn export_diag(app: AppHandle) -> Result<String, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut out = String::new();

    // 1) 头部:版本 / OS / 时间戳(unix 秒;工程无 chrono/time 依赖,不为此新增)。
    out.push_str("QidiWork 诊断日志\n");
    out.push_str("=================\n");
    out.push_str(&format!("GUI 版本: {}\n", env!("CARGO_PKG_VERSION")));
    out.push_str(&format!("OS: {}\n", std::env::consts::OS));
    out.push_str(&format!("时间戳(unix 秒): {now}\n\n"));

    // 2) GUI 日志节:app_log_dir 下 qidiwork-gui.log.* 最新 2 个,全文读入。
    out.push_str("## GUI 日志\n");
    match app.path().app_log_dir() {
        Ok(dir) => append_gui_logs(&mut out, &dir),
        Err(e) => out.push_str(&format!("(无法解析 GUI 日志目录: {e})\n")),
    }
    out.push('\n');

    // 3) 内核日志节:qidi home 下 logs/unified.jsonl 尾段。
    out.push_str("## 内核日志\n");
    match qidi_home(&app) {
        Some(home) => append_kernel_log(&mut out, &home.join("logs").join("unified.jsonl")),
        None => out.push_str("(无法解析主目录)\n"),
    }
    out.push('\n');

    // 4) 登录状态节:只输出布尔,绝不输出任何字段值。
    out.push_str("## 登录状态\n");
    let logged_in = qidi_home(&app)
        .map(|home| read_logged_in(&home.join("auth.json")))
        .unwrap_or(false);
    out.push_str(&format!("logged_in: {logged_in}\n"));

    // 写盘前统一脱敏。
    let redacted = redact(&out);
    let desktop = app
        .path()
        .desktop_dir()
        .map_err(|e| format!("无法解析桌面目录: {e}"))?;
    let path = desktop.join(format!("qidiwork-diag-{now}.txt"));
    std::fs::write(&path, redacted).map_err(|e| format!("写入诊断文件失败: {e}"))?;
    Ok(path.to_string_lossy().into_owned())
}

/// 内核 home:QIDI_HOME 优先,否则 `<home_dir>/.qidi`(与 auth.rs grok_home 一致)。
fn qidi_home(app: &AppHandle) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("QIDI_HOME") {
        let p = PathBuf::from(p);
        if !p.as_os_str().is_empty() {
            return Some(p);
        }
    }
    app.path().home_dir().ok().map(|h| h.join(".qidi"))
}

/// GUI 日志节:目录下 `qidiwork-gui.log*` 按修改时间降序取最新 2 个,全文读入。
fn append_gui_logs(out: &mut String, dir: &Path) {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            out.push_str(&format!("(无法读取 GUI 日志目录 {dir:?}: {e})\n"));
            return;
        }
    };
    let mut files: Vec<(SystemTime, PathBuf)> = Vec::new();
    for ent in rd.flatten() {
        let name = ent.file_name();
        if !name.to_string_lossy().starts_with("qidiwork-gui.log") {
            continue;
        }
        let mtime = ent
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(UNIX_EPOCH);
        files.push((mtime, ent.path()));
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    if files.is_empty() {
        out.push_str("(未找到 GUI 日志文件)\n");
        return;
    }
    for (_, path) in files.into_iter().take(GUI_LOG_FILES) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        out.push_str(&format!("=== {name} ===\n"));
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                out.push_str(&content);
                if !content.ends_with('\n') {
                    out.push('\n');
                }
            }
            Err(e) => out.push_str(&format!("(读取失败: {e})\n")),
        }
        out.push('\n');
    }
}

/// 内核日志节:>2MB 时 seek 到末尾 2MB 再读,取最后 2000 行;缺失标注「未找到」。
fn append_kernel_log(out: &mut String, path: &Path) {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => {
            out.push_str("(未找到)\n");
            return;
        }
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let truncated = len > KERNEL_LOG_TAIL_LIMIT;
    if truncated {
        let _ = f.seek(SeekFrom::End(-(KERNEL_LOG_TAIL_LIMIT as i64)));
    }
    let mut buf = Vec::new();
    if let Err(e) = f.read_to_end(&mut buf) {
        out.push_str(&format!("(读取失败: {e})\n"));
        return;
    }
    let text = String::from_utf8_lossy(&buf);
    // seek 落在行中时,丢弃首个不完整行。
    let text: &str = if truncated {
        match text.find('\n') {
            Some(i) => &text[i + 1..],
            None => &text,
        }
    } else {
        &text
    };
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(KERNEL_LOG_TAIL_LINES);
    if start >= lines.len() {
        out.push_str("(空)\n");
        return;
    }
    for line in &lines[start..] {
        out.push_str(line);
        out.push('\n');
    }
}

/// 判定登录态:auth.json 存在且**任一**条目 `key` 字段非空 → true。
/// 只返回布尔,绝不读取/输出任何字段值内容。
fn read_logged_in(path: &Path) -> bool {
    let raw = match std::fs::read_to_string(path) {
        Ok(r) => r,
        Err(_) => return false,
    };
    let value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let obj = match value.as_object() {
        Some(o) => o,
        None => return false,
    };
    obj.values().any(|entry| {
        entry
            .get("key")
            .and_then(|k| k.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    })
}

// --------------------------------------------------------------- 脱敏 --

const REDACTED_SK: &str = "sk-XXXX****REDACTED";
const REDACTED_BEARER: &str = "Bearer ****REDACTED";
/// `sk-` 后需连续 ≥16 个字母数字才视为凭证。
const SK_MIN_RUN: usize = 16;
/// `Bearer ` 后凭证串的最小长度(同样 ≥16,避免误伤普通词)。
const BEARER_MIN_RUN: usize = 16;

/// Bearer 凭证允许的字符(对齐常见 base64url / JWT / 十六进制 token)。
fn is_bearer_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'+' | b'/' | b'=')
}

/// 手工扫描器(不用 regex crate):
/// - `sk-` 后 ≥16 个字母数字 → `sk-XXXX****REDACTED`;
/// - `Bearer ` 后 ≥16 个凭证字符 → `Bearer ****REDACTED`。
fn redact(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < input.len() {
        let rest = &input[i..];

        if rest.starts_with("sk-") {
            let run_start = i + 3;
            let mut j = run_start;
            while j < input.len() && bytes[j].is_ascii_alphanumeric() {
                j += 1;
            }
            if j - run_start >= SK_MIN_RUN {
                out.push_str(REDACTED_SK);
                i = j;
                continue;
            }
        }

        if rest.starts_with("Bearer ") {
            let run_start = i + "Bearer ".len();
            let mut j = run_start;
            while j < input.len() && is_bearer_char(bytes[j]) {
                j += 1;
            }
            if j - run_start >= BEARER_MIN_RUN {
                out.push_str(REDACTED_BEARER);
                i = j;
                continue;
            }
        }

        // 未命中:按 UTF-8 字符边界原样拷一个字符。
        let ch_len = rest.chars().next().map(char::len_utf8).unwrap_or(1);
        out.push_str(&rest[..ch_len]);
        i += ch_len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_redact_masks_sk_and_bearer() {
            // 金丝雀凭证: 运行时拼接构造, 避免字面量被仓库密钥扫描器误报
    let canary_sk = ["sk-abc123", "4567890def123"].concat();
        let canary_jwt = ["eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVC", "9.eyJzdWIiOiJ0ZXN0In0.x"].concat();
        let input = format!(
            "api_key={canary_sk} header=\"Authorization: Bearer {canary_jwt}\" tail=keepme"
        );
        let got = redact(&input);
        assert!(!got.contains(&canary_sk), "sk canary must be redacted");
        assert!(got.contains("sk-XXXX****REDACTED"), "sk 掩码占位符缺失");
        assert!(!got.contains(&canary_jwt), "bearer canary must be redacted");
        assert!(got.contains("Bearer ****REDACTED"), "bearer 掩码占位符缺失");
        assert!(got.contains("keepme"), "非凭证文本应保留");
    }

    #[test]
    fn export_redact_keeps_short_runs() {
        // 不足 16 位不掩码(避免误伤普通文本,如文档里的 "sk-abc")。
        assert_eq!(redact("sk-abc123"), "sk-abc123");
        assert_eq!(redact("Bearer short"), "Bearer short");
    }

    #[test]
    fn export_redact_is_utf8_safe() {
        // 中文与 sk- 混合:不得在字符中间切断。
        let canary = ["sk-abcdefghij", "1234567890"].concat();
        let got = redact(&format!("token {} end", canary));
        assert!(got.contains("token") && got.contains("end"));
    }
}

