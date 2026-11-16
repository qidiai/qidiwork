//! M1：真实页数测量（WPS COM 实测，不估算）。
//!
//! 用本机 WPS（COM `KWPS.Application`）**只读**打开 docx → `Repaginate()` →
//! `ComputeStatistics(2)`（页）/ `ComputeStatistics(0)`（字），**不保存**、不改动原文件。
//! 实现照 `gui/tests/bid-export/render-wps.ps1` 的既有写法：docx 路径经环境变量
//! `BID_DOCX` 传给内联 PowerShell 脚本（避免命令行引号转义），脚本输出单行
//! `PAGES=<n> WORDS=<m>`，Rust 侧解析。
//!
//! 平台：仅 Windows（需本机装 WPS 且 COM 已注册）；其他平台 `measure` 直接返回 Err。

use std::path::Path;

use super::{BidError, BidResult};

/// 实测页数 / 字数（WPS 渲染后统计，非字符数估算）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageStat {
    /// 页数（`ComputeStatistics(2)`，wdStatisticPages）
    pub pages: u32,
    /// 字数（`ComputeStatistics(0)`，wdStatisticWords）
    pub words: u32,
}

/// 内联 PowerShell 脚本（Windows 专用）。
///
/// 要点：
/// - `OutputEncoding=UTF8`：重定向输出按 UTF-8 解码，异常信息不乱码；
/// - 先 `Get-Process wps` 判断是否已有实例，**只在自己拉起实例时** `Quit()`，
///   避免关掉用户正在编辑的窗口；
/// - `Documents.Open($env:BID_DOCX, $false, $true)` 第三参 `$true` = 只读；
/// - `Repaginate()` 后取页/字，`Close([ref]0)` = wdDoNotSaveChanges；
/// - 正常输出单行 `PAGES=<n> WORDS=<m>`；异常写 stderr 并让退出码非 0。
#[cfg(windows)]
const SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch { }
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

    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", SCRIPT])
        .env("BID_DOCX", &abs)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| BidError::new(format!("启动 PowerShell 失败: {e}")))?;

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
