@echo off
setlocal
title Silas Blue v3.3.2

:: Change directory to the folder containing this batch script
cd /d "%~dp0"

echo ========================================
echo        Starting Silas Blue v3.3.2
echo ========================================
echo.

:: 1. Check for project virtual environment
if exist "%~dp0.venv\Scripts\python.exe" (
    echo [INFO] Using virtual environment: .venv
    if exist "%~dp0.venv\Scripts\activate.bat" (
        call "%~dp0.venv\Scripts\activate.bat"
    )
    "%~dp0.venv\Scripts\python.exe" SilasBlue.py %*
    goto :handle_exit
)

:: 2. Check for system python
where python >nul 2>&1
if %ERRORLEVEL% equ 0 (
    echo [INFO] Using system Python...
    python SilasBlue.py %*
    goto :handle_exit
)

:: 3. Check for py launcher
where py >nul 2>&1
if %ERRORLEVEL% equ 0 (
    echo [INFO] Using Python launcher (py -3)...
    py -3 SilasBlue.py %*
    goto :handle_exit
)

:: 4. No Python found
echo [ERROR] Python was not found on your system!
echo Please install Python 3 and ensure it is added to your PATH:
echo https://www.python.org/downloads
echo.
pause
exit /b 1

:handle_exit
set "EXITCODE=%ERRORLEVEL%"
if %EXITCODE% neq 0 (
    echo.
    echo [ERROR] Silas Blue exited with error code %EXITCODE%.
    pause
)
exit /b %EXITCODE%
