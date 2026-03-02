@echo off
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cd /d "%~dp0"
cargo build --release -p ai-manager-tauri --features custom-protocol
