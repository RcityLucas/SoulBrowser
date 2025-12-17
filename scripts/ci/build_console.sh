#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
CONSOLE_DIR="$ROOT_DIR/web-console"
STATIC_DIR="$ROOT_DIR/static"
DIST_DIR="$CONSOLE_DIR/dist"

if [[ ! -d "$CONSOLE_DIR" ]]; then
    echo "web-console directory not found under $ROOT_DIR" >&2
    exit 1
fi

pushd "$CONSOLE_DIR" >/dev/null
if [[ -n "${CI:-}" ]]; then
    npm ci
else
    npm install
fi
npm run build
popd >/dev/null

if [[ ! -f "$DIST_DIR/index.html" ]]; then
    echo "Vite build did not produce dist/index.html" >&2
    exit 1
fi

mkdir -p "$STATIC_DIR"
rm -rf "$STATIC_DIR/assets"
cp "$DIST_DIR/index.html" "$STATIC_DIR/console.html"
cp -R "$DIST_DIR/assets" "$STATIC_DIR/assets"
if [[ -f "$DIST_DIR/vite.svg" ]]; then
    cp "$DIST_DIR/vite.svg" "$STATIC_DIR/assets/vite.svg"
else
    cat <<'SVG' > "$STATIC_DIR/assets/vite.svg"
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">
  <defs>
    <linearGradient id="grad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" stop-color="#0ea5e9"/>
      <stop offset="100%" stop-color="#6366f1"/>
    </linearGradient>
  </defs>
  <rect width="64" height="64" rx="12" fill="url(#grad)"/>
  <path d="M24 44L32 20l8 24" stroke="#fff" stroke-width="6" fill="none" stroke-linecap="round" stroke-linejoin="round"/>
</svg>
SVG
fi
python3 - "$STATIC_DIR/console.html" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
path.write_text(text.replace('/vite.svg', '/assets/vite.svg'))
PY

echo "Console assets synced to static/"
