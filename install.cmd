@echo off
rem Install znap per user + auto-start on login. No PowerShell needed.
setlocal
set "SYS=%SystemRoot%\System32"
set "DIR=%LOCALAPPDATA%\Programs\znap"
set "EXE=%DIR%\znap.exe"

"%SYS%\taskkill.exe" /f /im znap.exe >nul 2>&1
if not exist "%DIR%" mkdir "%DIR%"
copy /y "%~dp0znap.exe" "%EXE%" >nul || (echo Copy failed & pause & exit /b 1)
rem Drop "downloaded from internet" mark.
type nul > "%EXE%:Zone.Identifier" 2>nul

"%SYS%\reg.exe" add "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v znap /t REG_SZ /d "\"%EXE%\"" /f >nul || (echo Auto-start failed & pause & exit /b 1)

start "" "%EXE%"
echo znap installed to %EXE% (auto-start on). Ctrl+Alt+S to capture.
pause
