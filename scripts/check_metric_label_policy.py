#!/usr/bin/env python3
from __future__ import annotations

import pathlib
import re
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
SRC = ROOT / "crates" / "nanobot-core" / "src"

ALLOWED = {
    "backend",
    "channel",
    "code",
    "gateway",
    "le",
    "method",
    "op",
    "pool",
    "provider",
    "reason",
    "result",
    "route",
    "stage",
    "status",
    "tool",
    "type",
}

FORBIDDEN = {
    "request_id",
    "session_id",
    "user_id",
    "tenant_id",
    "ip",
    "url",
    "path",
    "error",
    "message",
    "stack",
    "nonce",
    "token",
    "model",
}


def iter_label_groups(text: str):
    # Match metric-like string literals: metric_name{label=value,...}
    metric_literal = re.compile(r'"[A-Za-z0-9_:.\-/]+\{([^"{}]*=[^"{}]*)\}[A-Za-z0-9_:.\-/]*"')
    for match in metric_literal.finditer(text):
        line_no = text.count("\n", 0, match.start()) + 1
        yield line_no, match.group(1)


def find_cfg_test_ranges(lines: list[str]) -> list[tuple[int, int]]:
    ranges: list[tuple[int, int]] = []
    i = 0
    while i < len(lines):
        line = lines[i]
        if "#[cfg(test)]" not in line:
            i += 1
            continue

        j = i + 1
        while j < len(lines) and not lines[j].strip():
            j += 1
        if j >= len(lines):
            break

        target = lines[j]
        if "mod tests" not in target:
            i = j + 1
            continue

        brace_balance = target.count("{") - target.count("}")
        start = j + 1
        k = j + 1
        while k < len(lines) and brace_balance > 0:
            brace_balance += lines[k].count("{") - lines[k].count("}")
            k += 1
        end = k
        ranges.append((start, end))
        i = k
    return ranges


def line_in_ranges(line_no: int, ranges: list[tuple[int, int]]) -> bool:
    return any(start <= line_no <= end for start, end in ranges)


def parse_labels(group: str):
    labels = []
    for part in group.split(','):
        part = part.strip()
        if '=' not in part:
            continue
        key, _value = part.split('=', 1)
        key = key.strip()
        if key:
            labels.append(key)
    return labels


def main() -> int:
    violations: list[str] = []
    for path in SRC.rglob("*.rs"):
        text = path.read_text(encoding="utf-8")
        lines = text.splitlines()
        test_ranges = find_cfg_test_ranges(lines)

        for line_no, labels_group in iter_label_groups(text):
            if line_in_ranges(line_no, test_ranges):
                continue
            for key in parse_labels(labels_group):
                if key in FORBIDDEN:
                    violations.append(
                        f"{path.relative_to(ROOT)}:{line_no}: forbidden metric label '{key}' in '{{{labels_group}}}'"
                    )
                elif key not in ALLOWED:
                    violations.append(
                        f"{path.relative_to(ROOT)}:{line_no}: non-allowlisted metric label '{key}' in '{{{labels_group}}}'"
                    )

    if violations:
        print("Metric label policy guard failed:")
        for v in violations:
            print(f"- {v}")
        return 1

    print("Metric label policy guard passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
