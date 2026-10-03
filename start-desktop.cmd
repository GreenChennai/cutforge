@echo off
rem CutForge 桌面壳一键启动器(M5-4 / docs/upstream/03 C-FE5):
rem 用法:start-desktop.cmd --root <工程目录> [--port N] [--attach URL]
rem 壳会自动拉起内核子进程(随壳退出);--attach 可接已运行的 serve。
setlocal
cd /d "%~dp0"

if not exist "target\debug\cutforge-desktop.exe" (
    echo [start-desktop] 首次运行,构建桌面壳 ^(debug=0 控盘^)...
    set CARGO_PROFILE_DEV_DEBUG=0
    cargo build -p cutforge-desktop || exit /b 1
)

if "%~1"=="" (
    echo 用法:start-desktop.cmd --root ^<工程目录^> [--port N] [--attach URL]
    echo   工程目录需含 05_时间线工程\project.json ^(或旧 05_ir\^)。
    echo   没有工程?cutforge-cli new ^<目录^> 新建。
    exit /b 2
)

target\debug\cutforge-desktop.exe %*
