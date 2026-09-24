@echo off
REM Compatibility entrypoint: use the same implementation as PowerShell.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\build\build-and-run.ps1" %*
exit /b %errorlevel%
