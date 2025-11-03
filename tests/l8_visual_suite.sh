#!/usr/bin/env bash
#
# L8 Visual & Perception Test Suite
#
# Mirrors the RainbowBrowser AI perception tests by driving the SoulBrowser CLI
# to capture structural, visual and semantic outputs, plus screenshots. Results
# are validated with jq to ensure key fields exist.

set -euo pipefail

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "❌ Missing dependency: $1" >&2
    exit 1
  fi
}

require jq
require cargo

info() {
  echo "ℹ️  $1"
}

IS_WSL=0
if [[ -n "${WSL_DISTRO_NAME:-}" ]] || grep -qi "microsoft" /proc/version 2>/dev/null; then
  IS_WSL=1
  export SOULBROWSER_DISABLE_SANDBOX="${SOULBROWSER_DISABLE_SANDBOX:-1}"
  info "Detected WSL/Windows subsystem; forcing SOULBROWSER_DISABLE_SANDBOX=1"
fi

REMOTE_WS=${SOULBROWSER_WS_URL:-}
if [[ -n "$REMOTE_WS" ]]; then
  info "Configured to attach to remote DevTools endpoint: $REMOTE_WS"
fi

# Ensure real Chrome is allowed
export SOULBROWSER_USE_REAL_CHROME="${SOULBROWSER_USE_REAL_CHROME:-1}"

if [[ "${SOULBROWSER_USE_REAL_CHROME,,}" != "1" && "${SOULBROWSER_USE_REAL_CHROME,,}" != "true" ]]; then
  echo "⚠️  SOULBROWSER_USE_REAL_CHROME is not enabled. Set it to 1 for reliable results." >&2
fi

detect_chrome() {
  if [[ -n "${SOULBROWSER_CHROME:-}" && -x "$SOULBROWSER_CHROME" ]]; then
    echo "ℹ️  Using Chrome executable from SOULBROWSER_CHROME: $SOULBROWSER_CHROME"
    return 0
  fi

  local candidates=()

  # Common UNIX binaries discoverable via PATH
  for bin in google-chrome-stable google-chrome chromium chromium-browser microsoft-edge beta-edge brave-browser; do
    if command -v "$bin" >/dev/null 2>&1; then
      candidates+=("$(command -v "$bin")")
    fi
  done

  # Common absolute paths (Linux/AppImage/Mac/WSL)
  candidates+=(
    "/usr/bin/google-chrome"
    "/usr/bin/chromium"
    "/usr/bin/chromium-browser"
    "/snap/bin/chromium"
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"
    "/mnt/c/Program Files/Google/Chrome/Application/chrome.exe"
    "/mnt/c/Program Files (x86)/Google/Chrome/Application/chrome.exe"
    "/mnt/c/Program Files/Microsoft/Edge/Application/msedge.exe"
  )

  for path in "${candidates[@]}"; do
    if [[ -n "$path" && -x "$path" ]]; then
      export SOULBROWSER_CHROME="$path"
      echo "ℹ️  Auto-detected Chrome executable: $SOULBROWSER_CHROME"
      return 0
    fi
  done

  echo "⚠️  Could not auto-detect a Chrome/Chromium executable. Set SOULBROWSER_CHROME manually." >&2
  return 1
}

if [[ -z "$REMOTE_WS" ]]; then
  detect_chrome || true
else
  info "Skipping local Chrome detection because SOULBROWSER_WS_URL is set"
fi

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$REPO_ROOT"

TMP_DIR=$(mktemp -d -t soulbrowser-visual-XXXX)
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

# In WSL we wrap Chrome with additional flags to improve stability
if [[ $IS_WSL -eq 1 && -z "$REMOTE_WS" && -n "${SOULBROWSER_CHROME:-}" ]]; then
  WRAPPER="$TMP_DIR/chrome-wrapper.sh"
  cat <<WRAP > "$WRAPPER"
#!/usr/bin/env bash
exec "${SOULBROWSER_CHROME}" \
  --disable-gpu \
  --disable-software-rasterizer \
  --use-angle=swiftshader \
  --disable-dev-shm-usage \
  --no-sandbox \
  --disable-setuid-sandbox \
  --disable-features=VizDisplayCompositor \
  --disable-crash-reporter \
  "\$@"
WRAP
  chmod +x "$WRAPPER"
  export SOULBROWSER_CHROME="$WRAPPER"
  info "Using Chrome wrapper with WSL-friendly flags"
fi

if [[ -z "$REMOTE_WS" ]]; then
  # Ensure no previous soulbrowser/chrome instances keep sockets open
  pkill -f soulbrowser >/dev/null 2>&1 || true
  pkill -f chrome >/dev/null 2>&1 || true
fi

TEST_URL=${TEST_URL:-"https://example.com"}
TIMEOUT=${PERCEPTION_TIMEOUT:-45}

RUN_CMD=(cargo run --quiet --)

# Enable verbose logging during perception runs for easier diagnosis
export RUST_LOG=${RUST_LOG:-debug}

PASSED=0
FAILED=0

pass() {
  echo "✅ $1"
  PASSED=$((PASSED + 1))
}

fail() {
  echo "❌ $1" >&2
  FAILED=$((FAILED + 1))
}

run_perceive() {
  local description=$1
  shift
  info "Starting perception: $description"
  set +e
  local cmd=("${RUN_CMD[@]}" perceive --url "$TEST_URL" --timeout "$TIMEOUT")
  if [[ -n "$REMOTE_WS" ]]; then
    cmd+=(--ws-url "$REMOTE_WS")
  fi
  cmd+=("$@")
  "${cmd[@]}"
  local status=$?
  set -e
  if [[ $status -eq 0 ]]; then
    pass "$description"
    return 0
  else
    fail "$description (cmd exit $status)"
    return 1
  fi
}

validate_json() {
  local file=$1
  local jq_expr=$2
  local description=$3
  if jq -e "$jq_expr" "$file" >/dev/null 2>&1; then
    pass "$description"
  else
    fail "$description (see $file)"
  fi
}

validate_file() {
  local file=$1
  local description=$2
  if [[ -s "$file" ]]; then
    pass "$description"
  else
    fail "$description"
  fi
}

echo "============================================"
echo "  SoulBrowser L8 Visual/Perception Test Run"
echo "============================================"
echo "URL:        $TEST_URL"
echo "Chrome:     ${SOULBROWSER_CHROME:-auto-detect}"
echo "Timeout:    ${TIMEOUT}s"
echo "Workspace:  $REPO_ROOT"
echo "Temp dir:   $TMP_DIR"
echo

# 1. Structural-only baseline
STRUCT_OUTPUT="$TMP_DIR/structural.json"
if run_perceive "Structural perception" --structural --output "$STRUCT_OUTPUT"; then
  validate_json "$STRUCT_OUTPUT" '.structural.dom_node_count' "Structural JSON contains dom_node_count"
  validate_json "$STRUCT_OUTPUT" '.confidence' "Structural JSON contains confidence"
fi

# 2. Visual perception + screenshot
VISUAL_OUTPUT="$TMP_DIR/visual.json"
SCREENSHOT_FILE="$TMP_DIR/visual.png"
if run_perceive "Visual perception" --visual --output "$VISUAL_OUTPUT" --screenshot "$SCREENSHOT_FILE"; then
  validate_json "$VISUAL_OUTPUT" '.visual.screenshot_id' "Visual JSON contains screenshot_id"
  validate_json "$VISUAL_OUTPUT" '.visual.avg_contrast' "Visual JSON contains avg_contrast"
  validate_file "$SCREENSHOT_FILE" "Screenshot written"
fi

# 3. All modes with insights
FULL_OUTPUT="$TMP_DIR/full.json"
if run_perceive "Full multi-modal perception" --all --insights --output "$FULL_OUTPUT"; then
  validate_json "$FULL_OUTPUT" '.structural.interactive_element_count' "Full JSON contains interactive count"
  validate_json "$FULL_OUTPUT" '.semantic.summary' "Full JSON contains semantic summary"
  validate_json "$FULL_OUTPUT" '.insights | type == "array"' "Full JSON contains insights array"
fi

# 4. Replay compatibility check (perceiver summary)
PERCEIVER_JSON="$TMP_DIR/perceiver.json"
if "${RUN_CMD[@]}" perceiver --format json --limit 5 > "$PERCEIVER_JSON" 2>/dev/null; then
  validate_json "$PERCEIVER_JSON" '.summary.resolve' "Perceiver summary exposes resolve count"
else
  echo "⚠️  Perceiver history unavailable (possibly empty state center). Skipping." >&2
fi

echo
TOTAL=$((PASSED + FAILED))
if [[ $TOTAL -eq 0 ]]; then
  echo "No tests executed. Check earlier output." >&2
  exit 1
fi
SUCCESS_RATE=$(python - <<PY
passed=$PASSED
failed=$FAILED
total=passed+failed
print(f"{(passed/total)*100:.1f}")
PY
)

echo "============================================"
echo "           TEST RUN SUMMARY"
echo "============================================"
echo "Total checks : $TOTAL"
echo "Passed       : $PASSED"
echo "Failed       : $FAILED"
echo "Success rate : ${SUCCESS_RATE}%"

echo
if [[ $FAILED -eq 0 ]]; then
  echo "🎉 All visual/perception checks passed"
  exit 0
else
  echo "❌ $FAILED check(s) failed" >&2
  exit 1
fi
