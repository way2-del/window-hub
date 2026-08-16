@echo off
cd /d "%~dp0"

set "EXE=%~dp0src-tauri\target\release-fast\window-hub.exe"

for /f "delims=" %%i in ('rustc --print sysroot') do set "RUST_SYSROOT=%%i"
set "LLD_BIN=%RUST_SYSROOT%\lib\rustlib\x86_64-pc-windows-msvc\bin"
if exist "%LLD_BIN%\rust-lld.exe" set "PATH=%LLD_BIN%;%PATH%"

echo === Stopping running Window Hub (if any) ===
taskkill /F /IM window-hub.exe >nul 2>&1
timeout /t 1 /nobreak >nul

echo === Window Hub: release-fast (no bundle, rust-lld) ===
echo Note: first build of a profile recompiles everything; later edits mainly re-link.
call npm run tauri:build:fast
if errorlevel 1 (
  echo.
  echo Build failed.
  echo If you still see access denied on window-hub.exe, close it in Task Manager and retry.
  exit /b 1
)

if not exist "%EXE%" (
  echo.
  echo Release exe not found:
  echo   %EXE%
  exit /b 1
)

echo.
echo === Starting release-fast ===
echo   %EXE%
start "" "%EXE%"
exit /b 0
