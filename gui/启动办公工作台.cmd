@echo off
title QidiWork 启动器
rem 正式模式:连接真实 QIDI 内核(有真实 AI 回复)。
rem 前提:主工程已构建内核(G:\qidiwork\target\debug\qidi.exe)。
rem 构建方法(首次较慢,内存不足加 -j 2):
rem   cd /d G:\qidiwork
rem   set PROTOC=G:\qidiwork\bin\bin\protoc.exe
rem   cargo build -p cf-pager-bin

set KERNEL=G:\qidiwork\target\debug\qidi.exe

if not exist "%KERNEL%" (
    echo [错误] 未找到内核 %KERNEL%
    echo 请先构建主工程内核(见文件头注释),或使用「启动测试模式.cmd」。
    pause
    exit /b 1
)

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

set QIDIWORK_AGENT_PATH=%KERNEL%
start "" "%GUI_EXE%"
echo 已启动(正式模式)。关闭工作台窗口即可退出。
ping -n 4 127.0.0.1 >nul
