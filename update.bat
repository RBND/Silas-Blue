@echo off
setlocal enabledelayedexpansion
title Silas Blue System Updater

:: Change directory to the folder containing this batch script
cd /d "%~dp0"

set "PYTHONHOME="
set "CHANNEL=stable"

:parse_args
if "%~1"=="" goto after_args
if /i "%~1"=="--channel" (
    set "CHANNEL=%~2"
    shift
    shift
    goto parse_args
)
if /i "%~1"=="latest" (
    set "CHANNEL=latest"
    shift
    goto parse_args
)
if /i "%~1"=="stable" (
    set "CHANNEL=stable"
    shift
    goto parse_args
)
shift
goto parse_args

:after_args
echo ========================================
echo        Silas Blue System Updater        
echo ========================================
echo Target Channel: %CHANNEL%
echo.

:: 1. Snapshot Backups
for /f "tokens=2 delims==" %%I in ('wmic os get localdatetime /value') do set datetime=%%I
set "TIMESTAMP=%datetime:~0,8%_%datetime:~8,6%"
set "BACKUP_PATH=%~dp0backups\backup_%TIMESTAMP%"

echo [1/4] Creating safety snapshot of configuration & state...
if not exist "%~dp0backups" mkdir "%~dp0backups"
if exist "%~dp0config" xcopy /E /I /Q /Y "%~dp0config" "%BACKUP_PATH%\config\" >nul 2>&1
if exist "%~dp0state" xcopy /E /I /Q /Y "%~dp0state" "%BACKUP_PATH%\state\" >nul 2>&1
echo [OK] Backup saved to %BACKUP_PATH%
echo.

:: 2. Fetch updates from Git remote
echo [2/4] Fetching latest updates from GitHub...
git fetch origin --tags

for /f %%i in ('git rev-parse --short HEAD 2^>nul') do set "PREV_COMMIT=%%i"

if /i "%CHANNEL%"=="latest" (
    echo [INFO] Pulling latest commits from main branch...
    git checkout main
    git reset --hard origin/main
) else (
    echo [INFO] Updating on stable release channel...
    for /f "tokens=*" %%T in ('git tag -l "v*" "V*" ^| sort') do set "LATEST_TAG=%%T"
    if defined LATEST_TAG (
        echo [INFO] Checking out latest release tag: !LATEST_TAG!
        git checkout !LATEST_TAG!
    ) else (
        echo [WARN] No release tags found. Pulling main branch...
        git checkout main
        git reset --hard origin/main
    )
)

for /f %%i in ('git rev-parse --short HEAD 2^>nul') do set "NEW_COMMIT=%%i"
echo [OK] Updated repository: %PREV_COMMIT% -^> %NEW_COMMIT%
echo.

:: 3. Check Python dependencies
echo [3/4] Checking Python dependencies...
set "PYTHON_EXE=python"
if exist "%~dp0.venv\Scripts\python.exe" (
    set "PYTHON_EXE=%~dp0.venv\Scripts\python.exe"
)

git diff --name-only %PREV_COMMIT%..%NEW_COMMIT% 2>nul | findstr /i "requirements.txt" >nul
if %ERRORLEVEL% equ 0 (
    echo [INFO] requirements.txt changed. Installing updated packages...
    "%PYTHON_EXE%" -m pip install --no-cache-dir -r requirements.txt
) else (
    echo [INFO] Dependencies are up to date.
)
echo.

echo ========================================
echo    Update completed successfully!      
echo ========================================
echo You can now start Silas Blue using start.bat
echo.
pause
