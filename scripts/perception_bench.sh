#!/usr/bin/env bash

set -euo pipefail

URL=${1:-https://example.com}
RUNS=20
OUTPUT_DIR="soulbrowser-output/perf"
CSV_PATH="$OUTPUT_DIR/perception.csv"

mkdir -p "$OUTPUT_DIR"
echo "mode,iteration,duration_ms" >"$CSV_PATH"

run_suite() {
  local mode=$1
  local disable_pool=$2
  for ((i = 1; i <= RUNS; i++)); do
    echo "[perception-bench] mode=${mode} iteration=${i} url=${URL}"
    if [[ "$disable_pool" == "true" ]]; then
      export SOULBROWSER_DISABLE_PERCEPTION_POOL=1
    else
      unset SOULBROWSER_DISABLE_PERCEPTION_POOL
    fi

    if ! output=$(cargo run --quiet -- perceive --url "$URL" --timeout 30 --structural --visual --semantic --insights 2>&1); then
      echo "$output"
      echo "Run failed for mode=${mode} iteration=${i}" >&2
      exit 1
    fi

    duration=$(printf '%s\n' "$output" | grep -m1 'PERCEPTION_DURATION_MS=' | awk -F '=' '{print $2}')
    if [[ -z "$duration" ]]; then
      printf '%s\n' "$output"
      echo "Failed to parse duration for mode=${mode} iteration=${i}" >&2
      exit 1
    fi
    echo "${mode},${i},${duration}" >>"$CSV_PATH"
  done
}

run_suite shared false
run_suite ephemeral true

unset SOULBROWSER_DISABLE_PERCEPTION_POOL
echo "Benchmark complete → $CSV_PATH"
