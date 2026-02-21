#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel)"
AGENT_FILE="$ROOT_DIR/crates/nanobot-core/src/agent/mod.rs"

DISALLOWED_FILES=(
  "$ROOT_DIR/crates/nanobot-core/src/gateway/mod.rs"
  "$ROOT_DIR/crates/nanobot-core/src/tools/executor.rs"
  "$ROOT_DIR/crates/nanobot-core/src/server/admin.rs"
)

for f in "${DISALLOWED_FILES[@]}"; do
  if [[ -f "$f" ]] && grep -q "spawn_blocking(" "$f"; then
    echo "spawn_blocking is not allowed directly in request-path file: $f"
    exit 1
  fi
done

if [[ ! -f "$AGENT_FILE" ]]; then
  echo "Agent file not found: $AGENT_FILE"
  exit 1
fi

matches="$(grep -n "spawn_blocking(" "$AGENT_FILE" || true)"
if [[ -z "$matches" ]]; then
  echo "No spawn_blocking found in agent file"
  exit 0
fi

while IFS= read -r entry; do
  [[ -z "$entry" ]] && continue
  line_no="${entry%%:*}"

  start=$((line_no - 30))
  if (( start < 1 )); then
    start=1
  fi
  context="$(sed -n "${start},${line_no}p" "$AGENT_FILE")"

  if ! echo "$context" | grep -q "PERSISTENCE_BLOCKING_SEMAPHORE"; then
    echo "spawn_blocking near line $line_no missing persistence semaphore boundary"
    exit 1
  fi

  if ! echo "$context" | grep -q "\.acquire()"; then
    echo "spawn_blocking near line $line_no missing semaphore acquire"
    exit 1
  fi
done <<< "$matches"

echo "spawn_blocking semaphore-boundary guard passed"
