#!/usr/bin/env python3
from __future__ import annotations

import pathlib
import re
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
GATEWAY_FILE = ROOT / "crates" / "nanobot-core" / "src" / "gateway" / "mod.rs"

DONE_JSON_RE = re.compile(r'json!\(\{[\s\S]{0,600}?"type"\s*:\s*"done"[\s\S]{0,600}?\}\)')


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


def in_ranges(line_no: int, ranges: list[tuple[int, int]]) -> bool:
    return any(start <= line_no <= end for start, end in ranges)


def line_for_offset(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def main() -> int:
    text = GATEWAY_FILE.read_text(encoding="utf-8")
    lines = text.splitlines()
    test_ranges = find_cfg_test_ranges(lines)
    violations: list[str] = []

    for match in DONE_JSON_RE.finditer(text):
        snippet = match.group(0)
        line_no = line_for_offset(text, match.start())
        if in_ranges(line_no, test_ranges):
            continue
        if '"request_id"' not in snippet or '"status"' not in snippet:
            violations.append(
                f"{GATEWAY_FILE.relative_to(ROOT)}:{line_no}: done frame missing request_id/status"
            )

    if violations:
        print("Gateway terminal frame guard failed:")
        for item in violations:
            print(f"- {item}")
        return 1

    print("Gateway terminal frame guard passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
