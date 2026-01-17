#!/usr/bin/env bash
set -euo pipefail
PROMPT=${1:-"我想看下现在办理最多的案件是那种"}
OUTPUT_ROOT=${2:-"soulbrowser-output/demo"}
mkdir -p "${OUTPUT_ROOT}"
ARTIFACTS_PATH="${OUTPUT_ROOT}/artifacts.json"
RUN_PATH="${OUTPUT_ROOT}/run.json"
cargo run --quiet --bin soul_cli -- chat \
  --prompt "${PROMPT}" \
  --current-url https://www.baidu.com \
  --execute \
  --max-replans 1 \
  --artifacts-path "${ARTIFACTS_PATH}" \
  --save-run "${RUN_PATH}" \
  --planner rule
