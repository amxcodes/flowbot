#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${1:-http://127.0.0.1:18789}"
REQUESTS="${REQUESTS:-100}"
CONCURRENCY="${CONCURRENCY:-20}"

echo "Running gateway load smoke: base=$BASE_URL requests=$REQUESTS concurrency=$CONCURRENCY"

run_batch() {
  local path="$1"
  local i=0
  local active=0
  local start
  start="$(date +%s%3N)"

  while [[ "$i" -lt "$REQUESTS" ]]; do
    curl -sS -o /dev/null -w "%{http_code}" "$BASE_URL$path" >/dev/null &
    i=$((i + 1))
    active=$((active + 1))

    if [[ "$active" -ge "$CONCURRENCY" ]]; then
      wait -n
      active=$((active - 1))
    fi
  done

  wait
  local end
  end="$(date +%s%3N)"
  local elapsed_ms=$((end - start))
  echo "path=$path elapsed_ms=$elapsed_ms"
}

run_batch "/health"
run_batch "/api/bootstrap"

echo "Load smoke complete"
