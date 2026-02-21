#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel)"
AGENT_FILE="$ROOT_DIR/crates/nanobot-core/src/agent/mod.rs"
GATEWAY_FILE="$ROOT_DIR/crates/nanobot-core/src/gateway/mod.rs"

if [[ ! -f "$AGENT_FILE" || ! -f "$GATEWAY_FILE" ]]; then
  echo "Required source files not found"
  exit 1
fi

PROCESS_STREAMING_BLOCK="$(awk '
  /async fn process_streaming\(/ {in_block=1}
  in_block {print}
  /async fn flush_stream_assistant_buffer\(/ {exit}
' "$AGENT_FILE")"

if [[ -n "$PROCESS_STREAMING_BLOCK" ]] && echo "$PROCESS_STREAMING_BLOCK" | grep -q "\.start_message("; then
  echo "Found sync streaming persistence call in process_streaming hot path: start_message"
  exit 1
fi

if [[ -n "$PROCESS_STREAMING_BLOCK" ]] && echo "$PROCESS_STREAMING_BLOCK" | grep -q "\.append_message_content("; then
  echo "Found sync streaming persistence call in process_streaming hot path: append_message_content"
  exit 1
fi

if [[ -n "$PROCESS_STREAMING_BLOCK" ]] && echo "$PROCESS_STREAMING_BLOCK" | grep -q "std::fs::"; then
  echo "Found std::fs usage inside process_streaming async hot path"
  exit 1
fi

if grep -q "std::process::Command::new(" "$GATEWAY_FILE"; then
  echo "Found direct std::process::Command usage in gateway module"
  exit 1
fi

echo "Async hot path sync-call guard passed"
