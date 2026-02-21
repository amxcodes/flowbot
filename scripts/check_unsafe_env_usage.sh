#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(git rev-parse --show-toplevel)"

is_prod_sensitive_file() {
  local file="$1"

  if [[ "$file" =~ ^(deploy|k8s|infra|docker|helm|charts)/ ]]; then
    return 0
  fi

  if [[ "$file" =~ (^|/)Dockerfile($|[._-].*) ]]; then
    return 0
  fi

  if [[ "$file" =~ (^|/)\.env\.production([._-].*)?$ ]]; then
    return 0
  fi

  if [[ "$file" =~ (^|/)[^/]*\.production\.[^/]+$ ]]; then
    return 0
  fi

  return 1
}

line_is_commented() {
  local line="$1"
  local trimmed="${line#"${line%%[![:space:]]*}"}"
  [[ -z "$trimmed" || "$trimmed" == \#* ]]
}

violations=()

while IFS= read -r file; do
  [[ -z "$file" ]] && continue
  is_prod_sensitive_file "$file" || continue
  [[ -f "$ROOT_DIR/$file" ]] || continue

  line_no=0
  while IFS= read -r line; do
    line_no=$((line_no + 1))
    if line_is_commented "$line"; then
      continue
    fi

    if [[ "$line" =~ NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY[[:space:]]*=[[:space:]]*(1|true|"1"|"true") ]]; then
      violations+=("$file:$line_no:$line")
    fi

    if [[ "$line" =~ NANOBOT_ALLOW_UNSAFE_ANTIGRAVITY_CONFIRM[[:space:]]*=[[:space:]]*"?I_UNDERSTAND_THIS_IS_INSECURE"? ]]; then
      violations+=("$file:$line_no:$line")
    fi

    if [[ "$line" =~ NANOBOT_ALLOW_INSECURE_WS[[:space:]]*=[[:space:]]*(1|true|"1"|"true") ]]; then
      violations+=("$file:$line_no:$line")
    fi

    if [[ "$line" =~ NANOBOT_METRICS_PUBLIC[[:space:]]*=[[:space:]]*(true|"true"|1|"1") ]]; then
      violations+=("$file:$line_no:$line")
    fi
  done < "$ROOT_DIR/$file"
done < <(git -C "$ROOT_DIR" ls-files)

if [[ ${#violations[@]} -gt 0 ]]; then
  echo "Unsafe production env overrides detected in deployment-sensitive files:"
  printf ' - %s\n' "${violations[@]}"
  exit 1
fi

echo "Unsafe env usage check passed"
