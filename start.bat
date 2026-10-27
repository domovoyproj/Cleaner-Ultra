@echo off
cd /d "%~dp0"
if exist "dist\CleanerUltra-0.7.0\CleanerUltra.exe" (
  start "CleanerUltra" "dist\CleanerUltra-0.7.0\CleanerUltra.exe"
) else (
  cargo run --release
)
if errorlevel 1 pause
