@echo off
setlocal enabledelayedexpansion
title CutForge Editor
chcp 936 >nul
cd /d "%~dp0"
echo DEBUG-0-start
if not exist "target\release\cutforge-cli.exe" (
    echo DEBUG-1-need-build
    cargo build --release -p cutforge-cli
)
echo DEBUG-2-post-build
set TOKEN=cutforge-local
set PORT=7720
set ROOT=E:\平日资料\视频剪辑测试\工程文件
set N=0
for /d %%D in ("%ROOT%\*") do (
    if exist "%%D\05_时间线工程\project.json" (
        set /a N+=1
        set "PROJ_!N!=%%D"
        echo    !N!. %%~nxD
    )
)
echo DEBUG-3-N=%N%
set "PICK=1"
set /p PICK=INPUT-NUMBER:
echo DEBUG-4-PICK=%PICK%
set "TARGET=!PROJ_%PICK%!"
echo DEBUG-5-TARGET=%TARGET%
start "" "http://127.0.0.1:%PORT%/?token=%TOKEN%"
echo DEBUG-6-before-serve
"target\release\cutforge-cli.exe" serve "%TARGET%" --port %PORT% --token %TOKEN% --web "%~dp0apps\web" --open
echo DEBUG-7-after-serve
pause
