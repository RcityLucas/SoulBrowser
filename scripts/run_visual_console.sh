#!/usr/bin/env bash
# Launch the SoulBrowser visual testing console with sensible defaults.
# Automatically builds the release binary if needed, detects Chrome/Chromium,
# and optionally attaches to an external DevTools endpoint.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
BIN_RELEASE="${ROOT_DIR}/target/release/soulbrowser"
DEFAULT_PORT=8787
PORT="$DEFAULT_PORT"
CUSTOM_WS_URL=""
AUTO_BUILD=0
EXTRA_ARGS=()

log()   { printf '\033[1;34m[INFO]\033[0m %s\n' "$1"; }
warn()  { printf '\033[1;33m[WARN]\033[0m %s\n' "$1"; }
error() { printf '\033[1;31m[ERR ]\033[0m %s\n' "$1"; }

usage() {
  cat <<USAGE
Usage: $(basename "$0") [options] [-- extra soulbrowser args]

Options:
  --port <PORT>        Port for the testing console (default: ${DEFAULT_PORT})
  --ws-url <URL>       Attach to an existing DevTools endpoint
  --build              Force rebuilding the release binary
  --help               Show this message

Examples:
  ./scripts/run_visual_console.sh
  ./scripts/run_visual_console.sh --ws-url http://127.0.0.1:9222
  ./scripts/run_visual_console.sh --port 8800 -- --log-level debug
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --port)
      PORT="$2"; shift 2 ;;
    --ws-url)
      CUSTOM_WS_URL="$2"; shift 2 ;;
    --build)
      AUTO_BUILD=1; shift ;;
    --help|-h)
      usage; exit 0 ;;
    --)
      shift
      EXTRA_ARGS=("$@")
      break ;;
    *)
      warn "Unknown option $1"; usage; exit 1 ;;
  esac
done

# Ensure binary exists (build once if missing)
if [[ $AUTO_BUILD -eq 1 || ! -x "$BIN_RELEASE" ]]; then
  log "Building soulbrowser (release)..."
  (cd "$ROOT_DIR" && cargo build --release)
fi

# Detect WSL
IS_WSL=0
if [[ -n "${WSL_DISTRO_NAME:-}" ]] || grep -qi "microsoft" /proc/version 2>/dev/null; then
  IS_WSL=1
fi

# Browser configuration
if [[ -n "$CUSTOM_WS_URL" ]]; then
  export SOULBROWSER_WS_URL="$CUSTOM_WS_URL"
  log "Using external DevTools endpoint: $CUSTOM_WS_URL"
else
  : "${SOULBROWSER_USE_REAL_CHROME:=1}"
  export SOULBROWSER_USE_REAL_CHROME
  if [[ -z "${SOULBROWSER_CHROME:-}" ]]; then
    CANDIDATES=(
      "$(command -v google-chrome-stable 2>/dev/null || true)"
      "$(command -v google-chrome 2>/dev/null || true)"
      "$(command -v chromium 2>/dev/null || true)"
      "$(command -v chromium-browser 2>/dev/null || true)"
      "/usr/bin/google-chrome"
      "/usr/bin/chromium"
      "/snap/bin/chromium"
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
      "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"
      "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"
      "/mnt/c/Program Files/Google/Chrome/Application/chrome.exe"
      "/mnt/c/Program Files (x86)/Google/Chrome/Application/chrome.exe"
      "/mnt/c/Program Files/Microsoft/Edge/Application/msedge.exe"
    )
    for path in "${CANDIDATES[@]}"; do
      if [[ -n "$path" && -x "$path" ]]; then
        export SOULBROWSER_CHROME="$path"
        break
      fi
    done
  fi
  if [[ -n "${SOULBROWSER_CHROME:-}" ]]; then
    log "Using Chrome executable: $SOULBROWSER_CHROME"
  else
    warn "No Chrome executable detected; set SOULBROWSER_CHROME manually or use --ws-url."
  fi
fi

# Sandbox defaults (WSL-friendly)
if [[ $IS_WSL -eq 1 ]]; then
  : "${SOULBROWSER_DISABLE_SANDBOX:=1}"
  export SOULBROWSER_DISABLE_SANDBOX
  log "WSL detected – sandbox disabled by default"
else
  : "${SOULBROWSER_DISABLE_SANDBOX:=1}"
  export SOULBROWSER_DISABLE_SANDBOX
fi

# Ensure port is free
if lsof -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
  error "Port $PORT is already in use. Choose another port with --port."
  exit 1
fi

CMD=("$BIN_RELEASE" "--metrics-port" "0" "serve" "--port" "$PORT")
if [[ -n "$CUSTOM_WS_URL" ]]; then
  CMD+=("--ws-url" "$CUSTOM_WS_URL")
fi
CMD+=("${EXTRA_ARGS[@]}")

log "Launching visual testing console on http://127.0.0.1:$PORT"
exec "${CMD[@]}"
