@echo off
setlocal EnableExtensions EnableDelayedExpansion
cd /d "%~dp0"

REM ============================================================
REM  Window Hub NSIS installer (single *-setup.exe)
REM  Same identifier + higher version => upgrade existing install.
REM  Env: SKIP_VERSION_PROMPT=1 | BUILD_VERSION=x.y.z | BUILD_NO_PAUSE=1 | BUILD_NO_OPEN=1
REM ============================================================

call :check_tools
if errorlevel 1 exit /b 1

call :ensure_deps
if errorlevel 1 exit /b 1

call "%~dp0scripts\prompt-version.bat"
if errorlevel 1 (
  call :maybe_pause
  exit /b 1
)
call :read_app_version
if not defined APP_VER set "APP_VER=?"

echo [INFO] Building NSIS installer (version %APP_VER%)...
echo [INFO] identifier=com.xushi.window-hub (do not change; used for upgrade detect)
echo.

call npm run tauri build -- --bundles nsis
set "EXIT_CODE=%ERRORLEVEL%"
if not "%EXIT_CODE%"=="0" (
  echo.
  echo [ERROR] Build failed, exit code: %EXIT_CODE%
  call :maybe_pause
  exit /b %EXIT_CODE%
)

set "NSIS_DIR=%~dp0src-tauri\target\release\bundle\nsis"
set "OUT_ROOT=%~dp0dist-installer"
if not exist "%OUT_ROOT%" mkdir "%OUT_ROOT%"

echo.
echo [INFO] Copying installer to dist-installer ...
set "COPIED=0"
for %%F in ("%NSIS_DIR%\*-setup.exe") do (
  if exist "%%~fF" (
    copy /y "%%~fF" "%OUT_ROOT%\" >nul
    echo   - %%~nxF
    set "COPIED=1"
  )
)
if "%COPIED%"=="0" (
  echo [WARN] No *-setup.exe under:
  echo        %NSIS_DIR%
)

echo.
echo [OK] Installer package ready. version=%APP_VER%
echo.
echo Bundle dir:
echo   %NSIS_DIR%
echo.
echo Copied to:
echo   %OUT_ROOT%
echo.
echo Upgrade: keep identifier, bump version, run new *-setup.exe
echo.

if /i not "%BUILD_NO_OPEN%"=="1" (
  if exist "%OUT_ROOT%" (
    explorer "%OUT_ROOT%"
  ) else if exist "%NSIS_DIR%" (
    explorer "%NSIS_DIR%"
  )
)

call :maybe_pause
exit /b 0

:read_app_version
set "APP_VER="
for /f "usebackq delims=" %%V in (`node "scripts\set-version.mjs" --get`) do set "APP_VER=%%V"
goto :eof

:check_tools
where node >nul 2>&1
if errorlevel 1 (
  echo [ERROR] Node.js not found in PATH.
  call :maybe_pause
  exit /b 1
)
where npm >nul 2>&1
if errorlevel 1 (
  echo [ERROR] npm not found. Install Node.js.
  call :maybe_pause
  exit /b 1
)
where rustc >nul 2>&1
if errorlevel 1 (
  echo [ERROR] Rust not found in PATH.
  call :maybe_pause
  exit /b 1
)
exit /b 0

:ensure_deps
if not exist "node_modules\" (
  echo [INFO] Installing npm dependencies...
  call npm install
  if errorlevel 1 (
    echo [ERROR] npm install failed.
    call :maybe_pause
    exit /b 1
  )
)
exit /b 0

:maybe_pause
if /i "%BUILD_NO_PAUSE%"=="1" goto :eof
pause
goto :eof
