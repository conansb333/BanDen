#!/usr/bin/env bash
# BanDen one-click launcher (invoked by "Run BanDen.bat").
#
# Behavior:
#   - If BanDen is already running: bring its window to the front and exit.
#   - Otherwise: launch the standalone release build (no dev server needed).
#   - If the release build does not exist yet: build it once (frontend +
#     Rust release build, several minutes), then launch.

set -u

APP_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP_DIR="$APP_ROOT/apps/desktop"
EXE="$APP_ROOT/target/release/banden-app.exe"
WATCHDOG_SRC="$APP_ROOT/target/release/banden-watchdog.exe"

info()  { printf '[banden] %s\n' "$*"; }
pause() { printf '[banden] Press any key to close... '; read -r -n 1 -s; printf '\n'; }

app_running() {
  tasklist //FI "IMAGENAME eq banden-app.exe" 2>/dev/null | grep -qi "banden-app.exe"
}

focus_app() {
  powershell -NoProfile -Command \
    "\$p = Get-Process banden-app -ErrorAction SilentlyContinue; if (\$p -and \$p.MainWindowHandle -ne 0) { Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.Interaction]::AppActivate(\$p.Id) | Out-Null }" \
    >/dev/null 2>&1
}

if app_running; then
  info "BanDen is already running - bringing it to the front."
  focus_app
  exit 0
fi

if [[ ! -f "$EXE" ]]; then
  info "First run: building BanDen (several minutes, one time only)..."

  if [[ ! -f "$DESKTOP_DIR/dist/index.html" ]]; then
    info "Building the frontend..."
    ( cd "$DESKTOP_DIR" && { [[ -d node_modules ]] || npm install; } && npm run build ) \
      || { info "Frontend build failed - is Node.js installed?"; pause; exit 1; }
  fi

  info "Building the application (release)..."
  ( cd "$APP_ROOT" && cargo build --release -p banden-app -p banden-watchdog --features custom-protocol ) \
    || { info "Rust build failed - is the Rust toolchain installed?"; pause; exit 1; }
fi

# The independent recovery watchdog must sit next to the app binary.
if [[ -f "$WATCHDOG_SRC" && "$(dirname "$EXE")" != "$APP_ROOT/target/release" ]]; then
  cp -f "$WATCHDOG_SRC" "$(dirname "$EXE")/" 2>/dev/null || true
fi

info "Launching BanDen..."
cmd //c start "" "$(cygpath -w "$EXE")" >/dev/null 2>&1
info "Done - the BanDen window should appear shortly."
