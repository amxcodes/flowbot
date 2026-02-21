#!/usr/bin/env bash
set -euo pipefail

WS_URL="${WS_URL:-ws://127.0.0.1:18789/ws}"
METRICS_URL="${METRICS_URL:-http://127.0.0.1:18789/metrics}"
TIMEOUT="${TIMEOUT:-30}"
PROMPT="${PROMPT:-Say hello in exactly one short sentence.}"
LEVELS="${LEVELS:-10 50 200}"

echo "Running LLM queue load matrix: levels=$LEVELS"

for C in $LEVELS; do
  REQ="$((C * 2))"
  if [[ "$REQ" -lt 40 ]]; then
    REQ=40
  fi

  echo "--- concurrency=$C requests=$REQ ---"
  EXTRA_ARGS=()
  if [[ "$C" == "50" ]]; then
    EXTRA_ARGS+=(--max-p95-ms 2000 --max-p99-ms 5000 --max-rejection-rate 0.01 --max-avg-wait-seconds 1.0)
  elif [[ "$C" == "200" ]]; then
    EXTRA_ARGS+=(--max-rejection-rate 0.35)
  fi

  python scripts/load_smoke_llm_queue.py \
    --ws-url "$WS_URL" \
    --metrics-url "$METRICS_URL" \
    --requests "$REQ" \
    --concurrency "$C" \
    --timeout "$TIMEOUT" \
    --prompt "$PROMPT" \
    "${EXTRA_ARGS[@]}"
done
