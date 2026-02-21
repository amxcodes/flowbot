#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel)"
GATEWAY_FILE="$ROOT_DIR/crates/nanobot-core/src/gateway/mod.rs"

if [[ ! -f "$GATEWAY_FILE" ]]; then
  echo "Gateway file not found: $GATEWAY_FILE"
  exit 1
fi

matches="$(grep -n "rusqlite::Connection::open(" "$GATEWAY_FILE" || true)"
if [[ -z "$matches" ]]; then
  echo "Gateway sqlite sync guard passed"
  exit 0
fi

while IFS= read -r entry; do
  [[ -z "$entry" ]] && continue
  line_no="${entry%%:*}"

  start=$((line_no - 12))
  if (( start < 1 )); then
    start=1
  fi
  end=$((line_no + 4))

  context="$(sed -n "${start},${end}p" "$GATEWAY_FILE")"
  if ! echo "$context" | grep -Eq "spawn_blocking|crate::blocking::sqlite"; then
    echo "Found rusqlite::Connection::open outside blocking boundary near line $line_no"
    exit 1
  fi
done <<< "$matches"

echo "Gateway sqlite sync guard passed"
