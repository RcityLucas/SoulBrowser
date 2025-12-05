#!/usr/bin/env bash
set -euo pipefail

API_BASE=${SOULBROWSER_API_BASE:-"http://127.0.0.1:8801"}
PROM_ENDPOINT=${SOULBROWSER_PROMETHEUS:-"http://127.0.0.1:9090/metrics"}
ITERATIONS=${SOULBROWSER_SOAK_ITERATIONS:-100}
PAUSE_SECONDS=${SOULBROWSER_SOAK_PAUSE:-5}
OUTPUT_DIR=${SOULBROWSER_SOAK_OUTPUT:-"soulbrowser-output"}
LOG_FILE="$OUTPUT_DIR/soak.log"

mkdir -p "$OUTPUT_DIR"

echo "# SoulBrowser soak test" >"$LOG_FILE"
echo "# Started at $(date --iso-8601=seconds)" >>"$LOG_FILE"
echo "# API_BASE=$API_BASE" >>"$LOG_FILE"
echo "# PROM_ENDPOINT=$PROM_ENDPOINT" >>"$LOG_FILE"

prompt_payload() {
  cat <<PAYLOAD
{
  "prompt": "Stage-3 soak test iteration $1",
  "execute": false,
  "capture_context": false,
  "constraints": ["keep actions simple", "log output"]
}
PAYLOAD
}

fetch_metric() {
  local metric="$1"
  if curl -fsS "$PROM_ENDPOINT" >/tmp/soak_metrics.$$; then
    grep -E "^${metric}" /tmp/soak_metrics.$$ | tail -n1 | awk '{print $2}'
    rm -f /tmp/soak_metrics.$$
  else
    echo "n/a"
  fi
}

for ((i = 1; i <= ITERATIONS; i++)); do
  payload=$(prompt_payload "$i")
  echo "[soak] iteration $i running chat request"
  response=$(curl -fsS -X POST "$API_BASE/api/chat" \
    -H 'content-type: application/json' \
    -d "$payload" 2>&1 || true)

  success=$(echo "$response" | grep -c '"success":true' || true)
  timestamp=$(date --iso-8601=seconds)
  if [[ "$success" -gt 0 ]]; then
    echo "[$timestamp] iteration $i success" >>"$LOG_FILE"
  else
    echo "[$timestamp] iteration $i failure: $response" >>"$LOG_FILE"
  fi

  hit_rate=$(fetch_metric soul_memory_hit_rate_percent)
  auto_retry=$(fetch_metric soul_self_heal_auto_retry_total)
  echo "[$timestamp] metrics hit_rate=$hit_rate auto_retry=$auto_retry" >>"$LOG_FILE"

  sleep "$PAUSE_SECONDS"
done

echo "# Completed at $(date --iso-8601=seconds)" >>"$LOG_FILE"
echo "Soak test finished. Log: $LOG_FILE"
