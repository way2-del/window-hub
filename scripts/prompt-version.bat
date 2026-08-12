@echo off
REM Shared version prompt for build-*.bat
REM Expects: cd already at repo root, delayed expansion optional
REM Sets: APP_VER
REM Env:
REM   SKIP_VERSION_PROMPT=1  keep current (no ask)
REM   BUILD_VERSION=x.y.z    set this version without ask

call :prompt_and_set_version
exit /b %ERRORLEVEL%

:prompt_and_set_version
set "APP_VER="
for /f "usebackq delims=" %%V in (`node "scripts\set-version.mjs" --get`) do set "APP_VER=%%V"
if not defined APP_VER (
  echo [ERROR] Cannot read version from tauri.conf.json
  exit /b 1
)

echo.
echo ========================================
echo   Window Hub  current version: %APP_VER%
echo ========================================
echo.

if /i "%SKIP_VERSION_PROMPT%"=="1" (
  echo [INFO] SKIP_VERSION_PROMPT=1, keep %APP_VER%
  exit /b 0
)

if defined BUILD_VERSION (
  set "NEW_VER=%BUILD_VERSION%"
  goto :apply_version
)

echo Enter new version ^(x.y.z^), or press Enter to keep %APP_VER%:
set /p "NEW_VER=Version> "
if not defined NEW_VER (
  echo [INFO] Keep version %APP_VER%
  exit /b 0
)
set "NEW_VER=%NEW_VER: =%"
if "%NEW_VER%"=="" (
  echo [INFO] Keep version %APP_VER%
  exit /b 0
)
if /i "%NEW_VER%"=="%APP_VER%" (
  echo [INFO] Same as current, no write.
  exit /b 0
)

:apply_version
echo [INFO] Writing version %NEW_VER% to package.json / tauri.conf.json / Cargo.toml ...
node "scripts\set-version.mjs" "%NEW_VER%"
if errorlevel 1 (
  echo [ERROR] Failed to write version.
  exit /b 1
)
set "APP_VER="
for /f "usebackq delims=" %%V in (`node "scripts\set-version.mjs" --get`) do set "APP_VER=%%V"
echo [OK] Version is now %APP_VER%
echo.
exit /b 0
