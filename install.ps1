# Install znap per user: copy exe + auto-start shortcut, then launch.
# Usage: powershell -ExecutionPolicy Bypass -File install.ps1
$ErrorActionPreference = 'Stop'
$src = Join-Path $PSScriptRoot 'znap.exe'
$dir = Join-Path $env:LOCALAPPDATA 'Programs\znap'
$exe = Join-Path $dir 'znap.exe'

Get-Process znap -ErrorAction SilentlyContinue | Stop-Process -Force
New-Item -ItemType Directory -Force $dir | Out-Null
Copy-Item $src $exe -Force
Unblock-File $exe  # drop "downloaded from internet" mark

$lnk = Join-Path ([Environment]::GetFolderPath('Startup')) 'znap.lnk'
$sc = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
$sc.TargetPath = $exe
$sc.WorkingDirectory = $dir
$sc.Save()

Start-Process $exe
Write-Host "znap installed to $exe (auto-start on). Ctrl+Alt+S to capture."
