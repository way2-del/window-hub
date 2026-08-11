@echo off
setlocal EnableExtensions
cd /d "%~dp0"

REM 默认打 NSIS 安装包；便携版请用 build-portable.bat
call "%~dp0build-installer.bat"
exit /b %ERRORLEVEL%
