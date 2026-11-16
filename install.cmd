@echo off
rem Double-click to install znap (per user, auto-start on login).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1"
pause
