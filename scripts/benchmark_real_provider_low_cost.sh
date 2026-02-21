#!/usr/bin/env bash
set -euo pipefail

PROVIDER="${PROVIDER:-antigravity}"
STRICT="${STRICT:-1}"
PROFILE_PATH="${PROFILE_PATH:-configs/provider_capacity_profiles.toml}"
PROFILE_MODE="${PROFILE_MODE:-low_cost}"
ADMISSION_REJECT_SIGNAL_BUDGET_MS="${ADMISSION_REJECT_SIGNAL_BUDGET_MS:-400}"
ADMISSION_REJECT_SIGNAL_LOAD_PENALTY_MS_PER_CONN="${ADMISSION_REJECT_SIGNAL_LOAD_PENALTY_MS_PER_CONN:-10}"
ADMISSION_REJECT_SIGNAL_LOAD_PENALTY_CAP_MS="${ADMISSION_REJECT_SIGNAL_LOAD_PENALTY_CAP_MS:-600}"
ADMISSION_HANDLER_REJECT_BUDGET_SECONDS="${ADMISSION_HANDLER_REJECT_BUDGET_SECONDS:-0.4}"

if [[ "$PROVIDER" == "openai" ]] && [[ -z "${OPENAI_API_KEY:-}" ]]; then
  echo "ERROR: OPENAI_API_KEY is not set"
  echo "Set OPENAI_API_KEY and rerun this script"
  exit 2
fi

OUT_FILE="${OUT_FILE:-docs/real-provider-low-cost-benchmark-latest.jsonl}"
PORT="${PORT:-18789}"
WS_URL="ws://127.0.0.1:${PORT}/ws"
METRICS_URL="http://127.0.0.1:${PORT}/metrics"
PROMPT="${PROMPT:-Respond with one short sentence (<= 20 words).}"
PERMITS_LIST="${PERMITS_LIST:-}"
QUEUE_WAIT_LIST="${QUEUE_WAIT_LIST:-}"
CONCURRENCY_LIST="${CONCURRENCY_LIST:-}"
PROVIDER_UNHEALTHY_FAIL_THRESHOLD="${PROVIDER_UNHEALTHY_FAIL_THRESHOLD:-}"

profile_get() {
  python - "$PROFILE_PATH" "$PROVIDER" "$PROFILE_MODE" "$1" <<'PY'
import pathlib
import sys

profile_path = pathlib.Path(sys.argv[1])
provider = sys.argv[2]
mode = sys.argv[3]
key = sys.argv[4]

if not profile_path.exists():
    print("")
    raise SystemExit(0)

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib  # type: ignore

data = tomllib.loads(profile_path.read_text(encoding="utf-8"))
node = (
    data.get("providers", {})
    .get(provider, {})
    .get(mode, {})
)
value = node.get(key)
if value is None:
    print("")
elif isinstance(value, list):
    print(" ".join(str(v) for v in value))
else:
    print(str(value))
PY
}

if [[ -z "$PERMITS_LIST" ]]; then
  PERMITS_LIST="$(profile_get "permits_list")"
fi
if [[ -z "$QUEUE_WAIT_LIST" ]]; then
  QUEUE_WAIT_LIST="$(profile_get "queue_wait_list")"
fi
if [[ -z "$CONCURRENCY_LIST" ]]; then
  CONCURRENCY_LIST="$(profile_get "concurrency_list")"
fi
if [[ -z "$PROVIDER_UNHEALTHY_FAIL_THRESHOLD" ]]; then
  PROVIDER_UNHEALTHY_FAIL_THRESHOLD="$(profile_get "provider_unhealthy_fail_threshold")"
fi

PERMITS_LIST="${PERMITS_LIST:-4 8 16}"
QUEUE_WAIT_LIST="${QUEUE_WAIT_LIST:-1000 3000}"
CONCURRENCY_LIST="${CONCURRENCY_LIST:-10 50}"
PROVIDER_UNHEALTHY_FAIL_THRESHOLD="${PROVIDER_UNHEALTHY_FAIL_THRESHOLD:-5}"

mkdir -p "$(dirname "$OUT_FILE")"

cleanup() {
  taskkill //IM nanobot.exe //F > /dev/null 2>&1 || true
  if [[ -f scripts/load-smoke-server.pid ]]; then
    python - <<'PY'
import os, signal
from pathlib import Path
p = Path('scripts/load-smoke-server.pid')
if p.exists():
    try:
        pid = int(p.read_text().strip())
    except Exception:
        pid = 0
    if pid > 0:
        try:
            os.kill(pid, signal.SIGTERM)
        except Exception:
            pass
    p.unlink(missing_ok=True)
PY
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

echo "Real provider low-cost benchmark $(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$OUT_FILE"
echo "provider=$PROVIDER" >> "$OUT_FILE"
echo "strict=$STRICT" >> "$OUT_FILE"
echo "profile=$PROFILE_PATH:$PROVIDER.$PROFILE_MODE" >> "$OUT_FILE"
echo "provider_unhealthy_fail_threshold=$PROVIDER_UNHEALTHY_FAIL_THRESHOLD" >> "$OUT_FILE"
echo "admission_reject_signal_budget_ms=$ADMISSION_REJECT_SIGNAL_BUDGET_MS" >> "$OUT_FILE"
echo "admission_reject_signal_load_penalty_ms_per_conn=$ADMISSION_REJECT_SIGNAL_LOAD_PENALTY_MS_PER_CONN" >> "$OUT_FILE"
echo "admission_reject_signal_load_penalty_cap_ms=$ADMISSION_REJECT_SIGNAL_LOAD_PENALTY_CAP_MS" >> "$OUT_FILE"
echo "admission_handler_reject_budget_seconds=$ADMISSION_HANDLER_REJECT_BUDGET_SECONDS" >> "$OUT_FILE"

failures=0
total_runs=0

for PERMITS in $PERMITS_LIST; do
  for QUEUE_WAIT_MS in $QUEUE_WAIT_LIST; do
    cleanup

    env \
      NANOBOT_CONFIG_PATH="scripts/load-smoke-real-config.toml" \
      NANOBOT_PROVIDER="$PROVIDER" \
      NANOBOT_LLM_BENCH_MODE=1 \
      NANOBOT_LLM_BENCH_NO_PERSISTENCE=1 \
      NANOBOT_LLM_CONCURRENCY_LIMIT="$PERMITS" \
      NANOBOT_LLM_QUEUE_WAIT_MS="$QUEUE_WAIT_MS" \
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

    echo "--- permits=$PERMITS queue_wait_ms=$QUEUE_WAIT_MS ---" >> "$OUT_FILE"

    for C in $CONCURRENCY_LIST; do
      total_runs=$((total_runs + 1))
      REQ=$((C * 2))
      if [[ "$REQ" -lt 40 ]]; then
        REQ=40
      fi
      LOAD_REJECT_SIGNAL_PENALTY_MS=$((C * ADMISSION_REJECT_SIGNAL_LOAD_PENALTY_MS_PER_CONN))
      if (( LOAD_REJECT_SIGNAL_PENALTY_MS > ADMISSION_REJECT_SIGNAL_LOAD_PENALTY_CAP_MS )); then
        LOAD_REJECT_SIGNAL_PENALTY_MS=$ADMISSION_REJECT_SIGNAL_LOAD_PENALTY_CAP_MS
      fi
      MAX_REJECT_SIGNAL_P95_MS=$((QUEUE_WAIT_MS + ADMISSION_REJECT_SIGNAL_BUDGET_MS + LOAD_REJECT_SIGNAL_PENALTY_MS))
      MAX_HANDLER_REJECT_AVG_SECONDS="$(python - <<PY
qms = float(${QUEUE_WAIT_MS})
extra = float(${ADMISSION_HANDLER_REJECT_BUDGET_SECONDS})
print(f"{qms/1000.0 + extra:.6f}")
PY
)"
      echo "--- concurrency=$C requests=$REQ ---" >> "$OUT_FILE"
      set +e
      python scripts/load_smoke_llm_queue.py \
        --ws-url "$WS_URL" \
        --metrics-url "$METRICS_URL" \
        --requests "$REQ" \
        --concurrency "$C" \
        --timeout 90 \
        --max-provider-unhealthy "$PROVIDER_UNHEALTHY_FAIL_THRESHOLD" \
        --max-reject-signal-p95-ms "$MAX_REJECT_SIGNAL_P95_MS" \
        --max-handler-reject-avg-seconds "$MAX_HANDLER_REJECT_AVG_SECONDS" \
        --prompt "$PROMPT" >> "$OUT_FILE"
      run_rc=$?
      set -e

      if [[ "$run_rc" -ne 0 ]]; then
        failures=$((failures + 1))
        echo "run_failed=1 permits=$PERMITS queue_wait_ms=$QUEUE_WAIT_MS concurrency=$C" >> "$OUT_FILE"
        if [[ "$STRICT" == "1" ]]; then
          echo "Benchmark failed early in strict mode. See $OUT_FILE"
          exit "$run_rc"
        fi
      fi
    done
  done
done

echo "Wrote $OUT_FILE"
echo "runs_total=$total_runs runs_failed=$failures" >> "$OUT_FILE"

if [[ "$failures" -gt 0 ]]; then
  echo "Completed with failures ($failures/$total_runs). See $OUT_FILE"
  exit 1
fi
