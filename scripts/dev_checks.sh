#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")"/.. && pwd)"
cd "$ROOT_DIR"

printf '==> cargo fmt --check\n'
cargo fmt --check

printf '\n==> cargo test\n'
cargo test

printf '\n==> planner custom tool lint\n'
python3 scripts/ci/lint_plan_tools.py

if command -v rg >/dev/null 2>&1; then
  printf '\n==> dead-file check\n'
  if ! rg --files | "$ROOT_DIR"/scripts/ci/check_dead_files.sh; then
    echo "dead-file check failed" >&2
    exit 1
  fi
else
  echo "ripgrep (rg) not available; skipping dead-file check" >&2
fi

if [ -d "web-console" ]; then
  printf '\n==> npm test (web-console)\n'
  if (cd web-console && npm ls >/dev/null 2>&1); then
    (cd web-console && npm test -- --watch=false) || echo "npm test failed"
  else
    echo "web-console dependencies not installed; skipping npm test"
  fi
fi
