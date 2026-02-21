#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel)"
FIXTURE_PATH="$ROOT_DIR/target/prometheus_fixture.prom"

cargo test -p nanobot-core emit_prometheus_fixture_for_promtool -- --nocapture

if [[ ! -f "$FIXTURE_PATH" ]]; then
  echo "Prometheus fixture not found at $FIXTURE_PATH"
  exit 1
fi

promtool check metrics "$FIXTURE_PATH"
echo "Prometheus metrics validation passed"
