#!/usr/bin/env bash
set -euo pipefail

OUT_FILE="${OUT_FILE:-docs/mock-provider-benchmark-latest.jsonl}"
PORT="${PORT:-18789}"
WS_URL="ws://127.0.0.1:${PORT}/ws"
METRICS_URL="http://127.0.0.1:${PORT}/metrics"
PROMPT="${PROMPT:-Respond with exactly five short chunks.}"
MOCK_SERVICE_MS="${MOCK_SERVICE_MS:-500}"
MOCK_CHUNKS="${MOCK_CHUNKS:-5}"
PERMITS_LIST="${PERMITS_LIST:-4 8 16}"
CONCURRENCY_LIST="${CONCURRENCY_LIST:-10 50 200}"

mkdir -p "$(dirname "$OUT_FILE")"

cleanup() {
  taskkill //IM nanobot.exe //F > /dev/null 2>&1 || true

  if [[ -f scripts/load-smoke-server.pid ]]; then
    pid="$(cat scripts/load-smoke-server.pid || true)"
    if [[ -n "$pid" ]]; then
      python - <<'PY'
import os, signal
from pathlib import Path
p=Path('scripts/load-smoke-server.pid')
if p.exists():
    try:
        pid=int(p.read_text().strip())
    except Exception:
        pid=0
    if pid>0:
        try:
            os.kill(pid, signal.SIGTERM)
        except Exception:
            pass
PY
    fi
    rm -f scripts/load-smoke-server.pid
  fi
}

trap cleanup EXIT

python - <<'PY'
import sqlite3
conn=sqlite3.connect('.nanobot/context_tree.db')
conn.execute('CREATE TABLE IF NOT EXISTS cron_jobs (id TEXT PRIMARY KEY, name TEXT, schedule_kind TEXT NOT NULL, schedule_data TEXT NOT NULL, payload_kind TEXT NOT NULL, payload_data TEXT NOT NULL, session_target TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL)')
conn.commit()
conn.close()
PY

echo "Mock provider benchmark $(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$OUT_FILE"
echo "service_ms=$MOCK_SERVICE_MS chunks=$MOCK_CHUNKS" >> "$OUT_FILE"

for PERMITS in $PERMITS_LIST; do
  cleanup
  NANOBOT_CONFIG_PATH="scripts/load-smoke-config.toml" \
  NANOBOT_MOCK_PROVIDER=1 \
  NANOBOT_MOCK_SERVICE_MS="$MOCK_SERVICE_MS" \
  NANOBOT_MOCK_CHUNKS="$MOCK_CHUNKS" \
  NANOBOT_LLM_BENCH_MODE=1 \
  NANOBOT_LLM_BENCH_NO_PERSISTENCE=1 \
  NANOBOT_LLM_CONCURRENCY_LIMIT="$PERMITS" \
  cargo run -p nanobot-cli -- server --port "$PORT" > "scripts/load-smoke-server.log" 2>&1 &
  echo "$!" > scripts/load-smoke-server.pid

  python - <<PY
import time, urllib.request, sys
url='http://127.0.0.1:${PORT}/health'
for _ in range(90):
    try:
        with urllib.request.urlopen(url, timeout=2) as r:
            if r.status == 200:
                sys.exit(0)
    except Exception:
        pass
    time.sleep(1)
sys.exit(1)
PY

  echo "--- permits=$PERMITS ---" >> "$OUT_FILE"

  for C in $CONCURRENCY_LIST; do
    REQ=$((C * 2))
    if [[ "$REQ" -lt 40 ]]; then
      REQ=40
    fi
    echo "--- concurrency=$C requests=$REQ ---" >> "$OUT_FILE"
    python scripts/load_smoke_llm_queue.py \
      --ws-url "$WS_URL" \
      --metrics-url "$METRICS_URL" \
      --requests "$REQ" \
      --concurrency "$C" \
      --timeout 60 \
      --prompt "$PROMPT" >> "$OUT_FILE"
  done
done

echo "Wrote $OUT_FILE"
