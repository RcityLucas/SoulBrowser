#!/usr/bin/env bash
set -euo pipefail

BACKEND_URL="${BACKEND_URL:-http://127.0.0.1:8789}"
ITERATIONS="${ITERATIONS:-20}"
MODE="${MODE:-structural}"
URL="${URL:-https://example.com}"
POOL_FLAG="${DISABLE_POOL:-0}"

function run_once() {
  local label="$1"
  echo "[benchmark] ${label}: running ${ITERATIONS} perceives against ${URL} (${MODE})"
  for i in $(seq 1 "${ITERATIONS}"); do
    curl -sS -X POST "${BACKEND_URL}/api/perceive" \
      -H 'Content-Type: application/json' \
      -d "{\"url\":\"${URL}\",\"mode\":\"${MODE}\",\"timeout\":60}" >/dev/null || {
      echo "request ${i} failed" >&2
      exit 1
    }
  done
  curl -sS "${BACKEND_URL}/api/perceive/metrics" | jq .
}

export SOULBROWSER_DISABLE_PERCEPTION_POOL="${POOL_FLAG}"
run_once "pool_flag=${POOL_FLAG}"
