@echo off
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0packaging\launch.ps1"
exit /b %errorlevel%
