@echo off
rem CutForge 编辑器一键启动器(产品自带,E1-4):双击或命令行均可。
rem 优先用 PATH 里的 cutforge-cli,否则回退到本仓库 target 构建产物;找不到则给出补救指引。
setlocal
set "HERE=%~dp0"
set "CLI="
where cutforge-cli >nul 2>nul && set "CLI=cutforge-cli"
if not defined CLI if exist "%HERE%target\release\cutforge-cli.exe" set "CLI=%HERE%target\release\cutforge-cli.exe"
if not defined CLI if exist "%HERE%target\debug\cutforge-cli.exe" set "CLI=%HERE%target\debug\cutforge-cli.exe"
if not defined CLI (
  echo 未找到 cutforge-cli:请下载官方预编译包并加入 PATH,或在仓库根执行 cargo build --release
  exit /b 3
)
"%CLI%" serve --open %*
