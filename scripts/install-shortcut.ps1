# CutForge 桌面快捷方式安装器(Windows)
# 用法: powershell -ExecutionPolicy Bypass -File scripts\install-shortcut.ps1 [-LaunchCmd path]
param(
    [string]$LaunchCmd = (Join-Path (Split-Path -Parent $PSScriptRoot) 'Launch-CutForge.cmd')
)
$ws = New-Object -ComObject WScript.Shell
$desktop = [Environment]::GetFolderPath('Desktop')
$lnk = Join-Path $desktop 'CutForge Editor.lnk'
$sc = $ws.CreateShortcut($lnk)
$sc.TargetPath = $LaunchCmd
$sc.WorkingDirectory = Split-Path -Parent $LaunchCmd
$sc.Description = 'CutForge video editor - browse and tweak CutFlow projects'
$sc.WindowStyle = 1
$sc.Save()
Write-Output "OK: $lnk"
