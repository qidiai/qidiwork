@echo off
title QidiWork 启动器(测试模式)
rem 测试模式:用内置 mock 内核,无真实 AI 回复,不消耗模型额度。

set GUI_DIR=G:\qidiwork\gui
set GUI_EXE=G:\qidiwork\gui\src-tauri\target\debug\qidiwork-gui.exe

if not exist "%GUI_EXE%" (
    echo [错误] 未找到 %GUI_EXE%
    echo 请先构建: cd /d G:\qidiwork\gui\src-tauri && cargo build
    pause
    exit /b 1
)

echo [1/2] 启动前端服务(5173 端口,若已在跑则自动复用)……
cd /d %GUI_DIR%
start "qidiwork-vite" /min cmd /c "npm run dev"

echo [2/2] 等待前端就绪并启动工作台……
powershell -Command "$t=20; while($t -gt 0){ try{ $r=Invoke-WebRequest -Uri http://127.0.0.1:5173 -UseBasicParsing -TimeoutSec 2; if($r.StatusCode -eq 200){break} }catch{}; $t--; Start-Sleep -Milliseconds 500 }"

set QIDIWORK_AGENT_PATH=%GUI_EXE%
set QIDIWORK_AGENT_ARGS=--mock-agent
start "" "%GUI_EXE%"
echo 已启动(测试模式)。关闭工作台窗口即可退出。
ping -n 4 127.0.0.1 >nul
