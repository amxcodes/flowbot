#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel)"
EXECUTOR_FILE="$ROOT_DIR/crates/nanobot-core/src/tools/executor.rs"

if [[ ! -f "$EXECUTOR_FILE" ]]; then
  echo "Executor file not found: $EXECUTOR_FILE"
  exit 1
fi

if grep -q "std::process::Command::new(" "$EXECUTOR_FILE"; then
  echo "Found direct std::process::Command::new in tool executor"
  exit 1
fi

if grep -q "std::fs::read_dir(" "$EXECUTOR_FILE"; then
  echo "Found direct std::fs::read_dir in tool executor"
  exit 1
fi

echo "Tool executor sync-call guard passed"
