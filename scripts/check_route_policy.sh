#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel)"
GATEWAY_FILE="$ROOT_DIR/crates/nanobot-core/src/gateway/mod.rs"

if [[ ! -f "$GATEWAY_FILE" ]]; then
  echo "gateway module not found: $GATEWAY_FILE"
  exit 1
fi

TEST_START_LINE="$(grep -n "^\#\[cfg(test)\]" "$GATEWAY_FILE" | head -n1 | cut -d: -f1)"
if [[ -z "${TEST_START_LINE:-}" ]]; then
  echo "missing #[cfg(test)] section in gateway module"
  exit 1
fi

# Before tests, only the central helper is allowed to call .route(...)
INVALID_ROUTE_CALLS="$(awk -v test_start="$TEST_START_LINE" '
  NR < test_start && /\.route\(/ {
    if ($0 !~ /router\.route\(path, handler\)/) {
      print NR ":" $0
    }
  }
' "$GATEWAY_FILE")"

if [[ -n "$INVALID_ROUTE_CALLS" ]]; then
  echo "Found direct route registrations outside route-policy helper:"
  echo "$INVALID_ROUTE_CALLS"
  exit 1
fi

echo "Route policy check passed"
