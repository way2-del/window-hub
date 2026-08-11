@echo off
setlocal EnableExtensions EnableDelayedExpansion
cd /d "%~dp0"

REM ============================================================
REM  Window Hub portable package (folder + zip)
REM  Note: app has DLL/plugins, not a true single-file exe.
REM        For a single distributable file, use build-installer.bat
REM ============================================================

call :check_tools
if errorlevel 1 exit /b 1

call :ensure_deps
if errorlevel 1 exit /b 1

if /i "%SKIP_BUILD%"=="1" goto :after_build

echo [INFO] Building portable release --no-bundle ...
echo.
call npm run tauri build -- --no-bundle
set "EXIT_CODE=%ERRORLEVEL%"
if not "%EXIT_CODE%"=="0" (
  echo.
  echo [ERROR] Build failed, exit code: %EXIT_CODE%
  call :maybe_pause
  exit /b %EXIT_CODE%
)

:after_build
if /i "%SKIP_BUILD%"=="1" echo [INFO] SKIP_BUILD=1, reuse existing release binaries.

set "REL=%~dp0src-tauri\target\release"
set "EXE=%REL%\window-hub.exe"
set "DLL=%REL%\window_hub_trayhook.dll"
if not exist "%EXE%" (
  echo [ERROR] Missing: %EXE%
  call :maybe_pause
  exit /b 1
)
if not exist "%DLL%" (
  echo [ERROR] Missing: %DLL%
  echo        trayhook DLL should be produced by release build.
  call :maybe_pause
  exit /b 1
)

call :read_app_version
if not defined APP_VER set "APP_VER=0.0.0"

set "OUT_ROOT=%~dp0dist-portable"
set "OUT_DIR=%OUT_ROOT%\WindowHub-v%APP_VER%-portable"
set "ZIP_PATH=%OUT_ROOT%\WindowHub-v%APP_VER%-portable.zip"

echo.
echo [INFO] Assembling portable dir: %OUT_DIR%
if exist "%OUT_DIR%" rmdir /s /q "%OUT_DIR%" 2>nul
mkdir "%OUT_DIR%" 2>nul
mkdir "%OUT_DIR%\resources" 2>nul

copy /y "%EXE%" "%OUT_DIR%\window-hub.exe" >nul
copy /y "%DLL%" "%OUT_DIR%\window_hub_trayhook.dll" >nul
copy /y "%DLL%" "%OUT_DIR%\resources\window_hub_trayhook.dll" >nul

robocopy "%~dp0src-tauri\resources\plugins" "%OUT_DIR%\resources\plugins" /E /NFL /NDL /NJH /NJS /nc /ns /np >nul
set "RC=%ERRORLEVEL%"
if %RC% GEQ 8 (
  echo [ERROR] Failed to copy plugin resources, robocopy exit: %RC%
  call :maybe_pause
  exit /b 1
)

echo [INFO] Creating zip...
if exist "%ZIP_PATH%" del /f /q "%ZIP_PATH%" >nul 2>&1
powershell -NoProfile -Command ^
  "Compress-Archive -Path '%OUT_DIR%' -DestinationPath '%ZIP_PATH%' -Force"
if errorlevel 1 (
  echo [ERROR] Zip creation failed.
  call :maybe_pause
  exit /b 1
)

echo.
echo [OK] Portable package ready.
echo.
echo Directory:
echo   %OUT_DIR%
echo.
echo Zip:
echo   %ZIP_PATH%
echo.
echo Usage: unzip then run window-hub.exe
echo.

if /i not "%BUILD_NO_OPEN%"=="1" (
  if exist "%OUT_ROOT%" explorer "%OUT_ROOT%"
)

call :maybe_pause
exit /b 0

:read_app_version
set "APP_VER="
for /f "usebackq delims=" %%V in (`node -p "require('./src-tauri/tauri.conf.json').version"`) do set "APP_VER=%%V"
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
