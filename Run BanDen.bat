@echo off
setlocal
cd /d "%~dp0"

rem ============================================================
rem  BanDen one-click launcher (double-clickable, self-contained)
rem  - If BanDen is already running: bring its window to front.
rem  - Otherwise: launch the standalone release build.
rem  - On first run: builds frontend + release exe (one time).
rem ============================================================

set "EXE=%~dp0target\release\banden-app.exe"
set "APPDIR=%~dp0apps\desktop"

tasklist /FI "IMAGENAME eq banden-app.exe" 2>nul | findstr /I "banden-app.exe" >nul
if not errorlevel 1 (
    echo [banden] Already running - bringing it to the front.
    powershell -NoProfile -Command "$p = Get-Process banden-app -ErrorAction SilentlyContinue; if ($p -and $p.MainWindowHandle -ne 0) { Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.Interaction]::AppActivate($p.Id) | Out-Null }" >nul 2>&1
    exit /b 0
)

if exist "%EXE%" goto launch

echo [banden] First run: building BanDen - this takes several minutes, one time only.
echo.

where cargo >nul 2>nul
if errorlevel 1 (
    echo [banden] ERROR: Rust was not found. Install it from https://rustup.rs and try again.
    pause
    exit /b 1
)

if not exist "%APPDIR%\dist\index.html" (
    echo [banden] Building the frontend...
    pushd "%APPDIR%"
    if not exist node_modules call npm install
    call npm run build
    if errorlevel 1 (
        echo [banden] ERROR: Frontend build failed. Is Node.js installed?
        popd
        pause
        exit /b 1
    )
    popd
)

echo [banden] Building the application, please wait...
cargo build --release -p banden-app -p banden-watchdog --features custom-protocol
if errorlevel 1 (
    echo [banden] ERROR: Build failed.
    pause
    exit /b 1
)

:launch
echo [banden] Launching BanDen...
start "" "%EXE%"
ping -n 3 127.0.0.1 >nul
tasklist /FI "IMAGENAME eq banden-app.exe" 2>nul | findstr /I "banden-app.exe" >nul
if errorlevel 1 (
    echo [banden] ERROR: The app did not start. Try running the exe directly for details.
    pause
    exit /b 1
)
echo [banden] BanDen is running.
ping -n 2 127.0.0.1 >nul
exit /b 0
