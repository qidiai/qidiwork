# QIDI Code 一键发布链脚本(内核 release + NSIS 安装包)
# 用法:在 PowerShell 里执行  .\build_release.ps1
#   .\build_release.ps1 -SkipInstaller   仅构建内核(跳过 GUI 安装包)
# 结果写入 build_result.txt,日志写入 build_log.txt
#
# 产物:
#   target\release\qidiwork.exe                         —— 内核
#   gui\src-tauri\target\release\bundle\nsis\*.exe      —— NSIS 安装包(内核已随包分发)
#
# 注意:代码签名走 SignPath(可选)。配置了 SIGNPATH_API_TOKEN 等环境变量时,
# 出包后自动提交 Authenticode 签名再落盘(见 tools/release/signpath-sign.py、
# docs/ops/code-signing.md);未配置则维持无签名分发,SmartScreen 提示
# 「更多信息 → 仍要运行」绕过,对外文档务必说明(见 docs 安装指引)。

param(
    [switch]$SkipInstaller
)

Set-Location $PSScriptRoot

if (-not $env:PROTOC) {
    $localProtoc = Join-Path $PSScriptRoot "bin\bin\protoc.exe"
    if (Test-Path $localProtoc) { $env:PROTOC = $localProtoc }
}

$log = "build_log.txt"
Remove-Item $log -ErrorAction SilentlyContinue

function Step($name, $block) {
    Write-Host ""
    Write-Host "==== $name ===="
    & $block *>> $log
    if ($LASTEXITCODE -ne 0) {
        Set-Content -Path build_result.txt -Value "FAIL - $name 失败,详见 build_log.txt"
        Write-Host "!!!! $name 失败(退出码 $LASTEXITCODE),日志: $log"
        [console]::beep(400, 500)
        exit 1
    }
    Write-Host "---- $name OK"
}

Write-Host "======================================"
Write-Host " 发布链:release 内核 → GUI → NSIS 安装包"
Write-Host " 内核全量编译预计 40~90 分钟,请勿关闭窗口"
Write-Host "======================================"

Step "1/4 内核 cargo build --release -p cf-pager-bin" {
    cargo build --release -p cf-pager-bin -j 2
}

$kernel = Join-Path $PSScriptRoot "target\release\qidiwork.exe"
if (-not (Test-Path $kernel)) {
    Set-Content -Path build_result.txt -Value "FAIL - 未找到 $kernel"
    Write-Host "!!!! 内核产物缺失:$kernel"
    exit 1
}

if ($SkipInstaller) {
    Set-Content -Path build_result.txt -Value "OK - 仅内核: $kernel"
    Write-Host "已按 -SkipInstaller 跳过安装包构建"
    exit 0
}

Step "2/4 GUI 前端 npm run build(vue-tsc + vite)" {
    Push-Location (Join-Path $PSScriptRoot "gui")
    npm run build
    Pop-Location
}

Step "3/4 Tauri 打包 npx tauri build(NSIS)" {
    Push-Location (Join-Path $PSScriptRoot "gui")
    npx tauri build
    Pop-Location
}

# tauri build 会跑 beforeBuildCommand 再次执行 npm run build,步骤 2 冗余
# 但可提前暴露前端错误,失败更早、日志更清晰。

$installer = Get-ChildItem (Join-Path $PSScriptRoot "gui\src-tauri\target\release\bundle\nsis") -Filter "*.exe" -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $installer) {
    Set-Content -Path build_result.txt -Value "FAIL - 未找到 NSIS 安装包(bundle\nsis 目录为空)"
    Write-Host "!!!! 安装包产物缺失,检查 build_log.txt 中 tauri build 段"
    exit 1
}

Set-Content -Path build_result.txt -Value "OK - 内核: $kernel; 安装包: $($installer.FullName)"
Write-Host ""
Write-Host "======================================"
Write-Host " 发布链完成!"
Write-Host " 内核:     $kernel"
Write-Host " 安装包:   $($installer.FullName)"

# 4/4(可选)SignPath Authenticode 代码签名:未配置 token 就跳过,维持旧行为。
# 顺序铁律:先 Authenticode 签名,再跑 tools/release/sign-update.py 算
# sha256/Ed25519 sig——签名改变文件字节,清单必须签最终分发文件。
if ($env:SIGNPATH_API_TOKEN) {
    Step "4/4 SignPath 代码签名(Authenticode)" {
        python "$PSScriptRoot\tools\release\signpath-sign.py" --file "$($installer.FullName)"
    }
    Write-Host " 已签名:   发布者显示 SignPath Foundation"
    Write-Host " 下一步:   python tools/release/sign-update.py 算 sha256/sig(必须在签名后)"
} else {
    Write-Host " 未签名:   未配置 SIGNPATH_API_TOKEN,跳过(见 docs/ops/code-signing.md)"
    Write-Host " 分发时请附 SmartScreen 绕过说明。"
}
Write-Host " 安装包已内置内核(Tauri resources),用户机器"
Write-Host " 无需安装 Rust/配置环境变量,双击安装即用。"
Write-Host "======================================"
[console]::beep(1000, 300)
