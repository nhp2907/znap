@echo off
rem Remove znap + auto-start.
setlocal
set "SYS=%SystemRoot%\System32"
"%SYS%\taskkill.exe" /f /im znap.exe >nul 2>&1
"%SYS%\reg.exe" delete "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v znap /f >nul 2>&1
rmdir /s /q "%LOCALAPPDATA%\Programs\znap" 2>nul
echo znap removed.
pause
