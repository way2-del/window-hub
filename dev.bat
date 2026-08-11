@echo off
setlocal EnableExtensions
cd /d "%~dp0"

where node >nul 2>&1
if errorlevel 1 (
  echo [ERROR] 未找到 Node.js，请先安装并加入 PATH。
  pause
  exit /b 1
)

where npm >nul 2>&1
if errorlevel 1 (
  echo [ERROR] 未找到 npm，请先安装 Node.js。
  pause
  exit /b 1
)

if not exist "node_modules\" (
  echo [INFO] 首次运行，正在安装依赖...
  call npm install
  if errorlevel 1 (
    echo [ERROR] npm install 失败。
    pause
    exit /b 1
  )
)

REM 偶尔清理旧编译产物（默认间隔 30 分钟；设 DEV_CLEAN_FORCE=1 强制清理）
call :maybe_clean_build

echo [INFO] 启动 Window Hub (tauri dev)...
call npm run tauri dev
set "EXIT_CODE=%ERRORLEVEL%"

if not "%EXIT_CODE%"=="0" (
  echo.
  echo [ERROR] 启动失败，退出码: %EXIT_CODE%
  pause
)

exit /b %EXIT_CODE%

:maybe_clean_build
set "STAMP=.dev-clean.stamp"
set "CLEAN_NEEDED=0"

if /i "%DEV_CLEAN_FORCE%"=="1" set "CLEAN_NEEDED=1"
if not exist "%STAMP%" set "CLEAN_NEEDED=1"

if "%CLEAN_NEEDED%"=="0" (
  powershell -NoProfile -Command ^
    "if ((Get-Date) - (Get-Item -LiteralPath '.dev-clean.stamp').LastWriteTime -ge [TimeSpan]::FromMinutes(30)) { exit 0 } else { exit 1 }" >nul 2>&1
  if not errorlevel 1 set "CLEAN_NEEDED=1"
)

if "%CLEAN_NEEDED%"=="0" (
  echo [INFO] 跳过清理编译产物（距上次清理不足 30 分钟；DEV_CLEAN_FORCE=1 可强制）
  goto :eof
)

echo [INFO] 清理旧前端产物（保留 Rust 增量缓存，避免全量重编）...
if exist "dist\" (
  rmdir /s /q "dist" 2>nul
  echo   - removed dist\
)
if exist "node_modules\.vite\" (
  rmdir /s /q "node_modules\.vite" 2>nul
  echo   - removed node_modules\.vite\
)
REM 仅在明确要求时清 Rust debug（会触发超久全量编译）
if /i "%DEV_CLEAN_RUST%"=="1" (
  if exist "src-tauri\target\debug\" (
    rmdir /s /q "src-tauri\target\debug" 2>nul
    echo   - removed src-tauri\target\debug\ ^(DEV_CLEAN_RUST=1^)
  )
)

> "%STAMP%" echo cleaned %DATE% %TIME%
echo [INFO] 清理完成。
goto :eof
