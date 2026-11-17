//! M1：真实页数测量（WPS COM 实测，不估算）。
//!
//! 用本机 WPS（COM `KWPS.Application`）**只读**打开 docx → `Repaginate()` →
//! `ComputeStatistics(2)`（页）/ `ComputeStatistics(0)`（字），**不保存**、不改动原文件。
//! 实现照 `gui/tests/bid-export/render-wps.ps1` 的既有写法：docx 路径经环境变量
//! `BID_DOCX` 传给内联 PowerShell 脚本（避免命令行引号转义），脚本输出单行
//! `PAGES=<n> WORDS=<m>`，Rust 侧解析。
//!
//! 平台：仅 Windows（需本机装 WPS 且 COM 已注册）；其他平台 `measure` 直接返回 Err。
//!
//! 超时兜底：WPS 偶发卡在 `Repaginate()`，子进程最长等 90s（每 200ms `try_wait`），
//! 超时 → `kill()` + 回收 → `Err("页数测量超时（90s）")`，绝不无限阻塞调用方。

use std::path::Path;
#[cfg(windows)]
use std::time::{Duration, Instant};

use super::{BidError, BidResult};

/// 子进程最长等待（WPS 卡死时必须有上限，否则 GUI 命令永久挂起）
const MEASURE_TIMEOUT: Duration = Duration::from_secs(90);
/// `try_wait` 轮询间隔
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// 实测页数 / 字数（WPS 渲染后统计，非字符数估算）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageStat {
    /// 页数（`ComputeStatistics(2)`，wdStatisticPages）
    pub pages: u32,
    /// 字数（`ComputeStatistics(0)`，wdStatisticWords）
    pub words: u32,
}

/// 超页判定（纯函数，便于单测）
///
/// - `max_pages = None`：未设红线 → 永不超页
/// - `max_pages = Some(n)`：`pages > n` 才算超；`pages == n` 视为**刚好达标**（红线允许等于）
pub fn exceeds(pages: u32, max_pages: Option<u32>) -> bool {
    matches!(max_pages, Some(max) if pages > max)
}

/// 内联 PowerShell 脚本（Windows 专用）。
///
/// 要点：
/// - `OutputEncoding` / `ErrorEncoding` 均为 UTF8：重定向输出按 UTF-8 解码，
///   stdout 与 stderr（中文异常信息）都不乱码；
/// - 先 `Get-Process wps` 判断是否已有实例，**只在自己拉起实例时** `Quit()`，
///   避免关掉用户正在编辑的窗口；
/// - `Documents.Open($env:BID_DOCX, $false, $true)` 第三参 `$true` = 只读；
/// - `Repaginate()` 后取页/字，`Close([ref]0)` = wdDoNotSaveChanges；
/// - 正常输出单行 `PAGES=<n> WORDS=<m>`；异常写 stderr 并让退出码非 0。
#[cfg(windows)]
const SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
try {
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    [Console]::ErrorEncoding = [System.Text.Encoding]::UTF8
} catch { }
$docxPath = $env:BID_DOCX
if (-not $docxPath) { [Console]::Error.WriteLine('BID_DOCX 为空'); exit 3 }
if (-not (Test-Path -LiteralPath $docxPath)) { [Console]::Error.WriteLine(('文件不存在: ' + $docxPath)); exit 3 }
$preExistingWps = @(Get-Process -Name wps -ErrorAction SilentlyContinue).Count -gt 0
$app = New-Object -ComObject KWPS.Application
try {
    # 只在自建实例时隐藏窗口：附着到用户已开着的实例时改 Visible 会把用户窗口一起藏掉
    if (-not $preExistingWps) { $app.Visible = $false }
    try { $app.DisplayAlerts = 0 } catch { }
    $doc = $app.Documents.Open($docxPath, $false, $true)
    try {
        $doc.Repaginate()
        $pages = $null
        try { $pages = $doc.ComputeStatistics(2) } catch { }
        if (-not $pages) {
            try { $pages = $doc.BuiltInDocumentProperties('Number of Pages').Value } catch { }
        }
        $words = $null
        try { $words = $doc.ComputeStatistics(0) } catch { }
        if ($null -eq $words) { $words = 0 }
        Write-Output ('PAGES=' + $pages + ' WORDS=' + $words)
    } finally {
        $doc.Close([ref]0)
    }
} finally {
    if (-not $preExistingWps) { $app.Quit() }
    [void][System.Runtime.InteropServices.Marshal]::ReleaseComObject($app)
    [GC]::Collect()
}
"#;

/// 用本机 WPS（COM `KWPS.Application`）只读打开 docx，`Repaginate` 后取页数/字数；不保存。
#[cfg(windows)]
pub fn measure(docx: &Path) -> BidResult<PageStat> {
    use std::os::windows::process::CommandExt;

    /// 不弹控制台窗口（GUI 宿主里调用时不闪黑框）
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    if !docx.is_file() {
        return Err(BidError::new(format!("文件不存在: {}", docx.display())));
    }
    // WPS COM 的 Open 要求绝对路径（相对路径会落到进程 CWD，行为不可预期）
    let abs = dunce::canonicalize(docx)
        .map_err(|e| BidError::new(format!("路径解析失败 {}: {e}", docx.display())))?;

    // 不用 output()（无超时无上限）：spawn + 轮询 90s，超时 kill 并回收
    let mut child = std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT])
        .env("BID_DOCX", &abs)
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| BidError::new(format!("启动 PowerShell 失败: {e}")))?;

    let deadline = Instant::now() + MEASURE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break, // 已退出：下面读输出
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait(); // 回收，避免僵尸进程
                    return Err(BidError::new("页数测量超时（90s）"));
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BidError::new(format!("等待 PowerShell 进程失败: {e}")));
            }
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|e| BidError::new(format!("读取 PowerShell 输出失败: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let code = match output.status.code() {
            Some(c) => c.to_string(),
            None => "无".to_string(),
        };
        let detail = if stderr.is_empty() {
            format!("PowerShell 退出码 {code}")
        } else {
            format!("PowerShell 退出码 {code}: {stderr}")
        };
        return Err(BidError::new(format!("WPS 页数测量失败: {detail}")));
    }

    parse_output(&String::from_utf8_lossy(&output.stdout))
}

/// 解析脚本输出里的 `PAGES=` / `WORDS=`（容忍其它噪点行）。
#[cfg(windows)]
fn parse_output(stdout: &str) -> BidResult<PageStat> {
    let mut pages: Option<u32> = None;
    let mut words: Option<u32> = None;
    for token in stdout.split_whitespace() {
        if let Some(v) = token.strip_prefix("PAGES=") {
            pages = parse_count(v);
        } else if let Some(v) = token.strip_prefix("WORDS=") {
            words = parse_count(v);
        }
    }
    match (pages, words) {
        (Some(pages), Some(words)) if pages > 0 => Ok(PageStat { pages, words }),
        (Some(_), Some(_)) => Err(BidError::new(format!(
            "页数测量失败: WPS 返回页数 0（输出: {}）",
            stdout.trim()
        ))),
        _ => Err(BidError::new(format!(
            "无法解析页数输出: {}",
            stdout.trim()
        ))),
    }
}

/// 纯数字解析（容忍千分位逗号与首尾空白）；非法 → `None`
#[cfg(windows)]
fn parse_count(raw: &str) -> Option<u32> {
    let cleaned: String = raw.chars().filter(|c| *c != ',').collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || !cleaned.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    cleaned.parse::<u32>().ok()
}

/// 非 Windows：WPS COM 不可用，直接报错（调用方按「测量失败」处理）。
#[cfg(not(windows))]
pub fn measure(_docx: &Path) -> BidResult<PageStat> {
    Err(BidError::new("页数测量仅支持 Windows（需本机 WPS）"))
}

// ---------------------------------------------------------------------------
// 单测：真实测量要 COM + WPS（CI 不可用），此处覆盖 `parse_output` 的异常分支
// ---------------------------------------------------------------------------

/// 超页判定是纯函数（无 COM），跨平台可测
#[cfg(test)]
mod limit_tests {
    use super::exceeds;

    /// 未设红线永不算超；页数等于上限也只是「刚好达标」，不算超
    #[test]
    fn exceeds_none_and_equal_to_limit_is_false() {
        assert!(!exceeds(5, None));
        assert!(!exceeds(0, None));
        assert!(!exceeds(50, Some(50)));
        assert!(!exceeds(1, Some(50)));
    }

    /// 超过上限才算超（多 1 页即超）
    #[test]
    fn exceeds_over_limit_is_true() {
        assert!(exceeds(51, Some(50)));
        assert!(exceeds(5, Some(1)));
    }
}

#[cfg(all(test, windows))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    /// 正常输出：容忍噪点行与千分位逗号
    #[test]
    fn parse_output_accepts_noise_and_thousands() {
        let stat = parse_output("some noise\nPAGES=12 WORDS=1,234\n").unwrap();
        assert_eq!(stat, PageStat { pages: 12, words: 1234 });
    }

    /// 页数为 0（WPS 未重排版/统计失败）→ Err 且含"页数 0"
    #[test]
    fn parse_output_rejects_zero_pages() {
        let err = parse_output("PAGES=0 WORDS=10").unwrap_err();
        assert!(err.to_string().contains("页数 0"), "错误信息不符: {err}");
    }

    /// 缺 `PAGES=` / `WORDS=` → Err 且含"无法解析页数输出"
    #[test]
    fn parse_output_rejects_garbage() {
        let err = parse_output("ComObject 未注册").unwrap_err();
        assert!(
            err.to_string().contains("无法解析页数输出"),
            "错误信息不符: {err}"
        );
    }
}
