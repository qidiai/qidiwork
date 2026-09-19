@echo off
title QidiWork 启动器
rem 正式模式:连接真实 QIDI 内核,使用生产版 GUI(内嵌前端,无需 vite)。
rem
rem 前提 1:主工程已构建内核。二进制 2026-09-12 起名为 qidiwork.exe,旧 qidi.exe 已弃。
rem   cd /d G:\qidiwork
rem   set PROTOC=G:\qidiwork\bin\bin\protoc.exe
rem   cargo build -p cf-pager-bin
rem 前提 2:生产版 GUI 构建时必须带 custom-protocol,否则运行时仍找 devUrl。
rem   cd /d G:\qidiwork\gui && npm run build
rem   cd src-tauri && cargo build --release --features custom-protocol

set KERNEL=G:\qidiwork\target\release\qidiwork.exe

if not exist "%KERNEL%" (
    echo [错误] 未找到内核,请先构建主工程内核,见文件头注释。
    echo 也可使用「启动测试模式.cmd」走 mock 内核。
    pause
    exit /b 1
)

set GUI_EXE=G:\qidiwork\gui\src-tauri\target\release\qidiwork-gui.exe

if not exist "%GUI_EXE%" (
    echo [错误] 未找到生产版 GUI,请先构建:npm run build 后 cargo build --release --features custom-protocol。
    pause
    exit /b 1
)

set QIDIWORK_AGENT_PATH=%KERNEL%
start "" "%GUI_EXE%"
echo 已启动-正式模式,真实内核。关闭工作台窗口即可退出。
ping -n 3 127.0.0.1 >nul
