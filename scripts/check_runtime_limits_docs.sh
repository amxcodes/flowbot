#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel)"
DOC_PATH="$ROOT_DIR/docs/architecture/runtime-limits.md"

if [[ ! -f "$DOC_PATH" ]]; then
  echo "Missing required runtime limits document: $DOC_PATH"
  exit 1
fi

python3 - "$DOC_PATH" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text(encoding="utf-8")

required = [
    "## Single-instance scope",
    "## Horizontal scaling status",
    "## Multi-tenant isolation model",
    "## Request correlation model",
]

missing = [h for h in required if h not in text]
if missing:
    print("Runtime limits document missing required headings:")
    for h in missing:
        print(f"- {h}")
    sys.exit(1)

print("Runtime limits documentation guard passed")
PY
