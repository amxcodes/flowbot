#!/usr/bin/env bash
set -euo pipefail

WS_URL="${WS_URL:-ws://127.0.0.1:18789/ws}"
METRICS_URL="${METRICS_URL:-http://127.0.0.1:18789/metrics}"
REQUESTS="${REQUESTS:-40}"
CONCURRENCY="${CONCURRENCY:-8}"
TIMEOUT="${TIMEOUT:-30}"
PROMPT="${PROMPT:-Say hello in exactly one short sentence.}"

python scripts/load_smoke_llm_queue.py \
  --ws-url "$WS_URL" \
  --metrics-url "$METRICS_URL" \
  --requests "$REQUESTS" \
  --concurrency "$CONCURRENCY" \
  --timeout "$TIMEOUT" \
  --prompt "$PROMPT"
