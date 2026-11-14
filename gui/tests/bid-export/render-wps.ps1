# 用本机 WPS（COM `KWPS.Application`）打开 docx → 更新版式 → 报页数/字数 → 导出 PDF。
#
# 用途：R-025「页数必须渲染器实测」的可复现证据；P0a/P0b 页数基线与吞图抽检。
# 前提：本机已装 WPS（本项目实测 F:\wps\WPS Office\12.1.0.29690），COM 已注册为 KWPS.Application。
# 用法：
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\render-wps.ps1 `
#       -Docx ..\tests\bid-export\out\p0a-sample.docx -Pdf ..\tests\bid-export\out\p0a-sample.pdf
param(
    [Parameter(Mandatory = $true)][string]$Docx,
    [Parameter(Mandatory = $true)][string]$Pdf
)
$ErrorActionPreference = "Stop"
$docxPath = (Resolve-Path $Docx).Path
if ([System.IO.Path]::IsPathRooted($Pdf)) {
    $pdfPath = [System.IO.Path]::GetFullPath($Pdf)
} else {
    $pdfPath = [System.IO.Path]::GetFullPath((Join-Path (Get-Location).Path $Pdf))
}
$pdfDir = Split-Path -Parent $pdfPath
if (-not (Test-Path $pdfDir)) { New-Item -ItemType Directory -Path $pdfDir -Force | Out-Null }
if (Test-Path $pdfPath) { Remove-Item $pdfPath -Force }

$app = New-Object -ComObject KWPS.Application
# 用户可能正开着 WPS 文档：COM 会附着到已有实例。仅在"我们自己拉起"时才 Quit，
# 避免把用户正在编辑的窗口一起关掉。
$preExistingWps = @(Get-Process -Name wps -ErrorAction SilentlyContinue).Count -gt 0
try {
    $app.Visible = $false
    try { $app.DisplayAlerts = 0 } catch { }
    # Open(FileName, ConfirmConversions, ReadOnly)
    $doc = $app.Documents.Open($docxPath, $false, $true)
    try {
        $doc.Repaginate()
        $pages = $null
        try { $pages = $doc.ComputeStatistics(2) } catch { Write-Output ("COMPUTE_STATS_ERR=" + $_.Exception.Message) }
        if (-not $pages) {
            try { $pages = $doc.BuiltInDocumentProperties("Number of Pages").Value } catch { }
        }
        $words = $null
        try { $words = $doc.ComputeStatistics(0) } catch { }
        Write-Output ("PAGES=" + $pages)
        Write-Output ("WORDS=" + $words)
        $saved = $false
        try {
            $doc.SaveAs([ref]$pdfPath, [ref]17)
            $saved = $true
        } catch {
            Write-Output ("SAVEAS_ERR=" + $_.Exception.Message)
            try {
                $doc.ExportAsFixedFormat($pdfPath, 17)
                $saved = $true
            } catch {
                Write-Output ("EXPORT_ERR=" + $_.Exception.Message)
            }
        }
        Write-Output ("PDF_SAVED=" + $saved)
    } finally {
        $doc.Close([ref]0)
    }
} finally {
    if (-not $preExistingWps) { $app.Quit() }
    [void][System.Runtime.InteropServices.Marshal]::ReleaseComObject($app)
    [GC]::Collect()
}
if (Test-Path $pdfPath) { Write-Output ("PDF_BYTES=" + (Get-Item $pdfPath).Length) } else { Write-Output "PDF_BYTES=0" }
