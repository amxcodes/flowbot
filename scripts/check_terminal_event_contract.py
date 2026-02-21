#!/usr/bin/env python3
from __future__ import annotations

import pathlib
import re
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
SRC_ROOT = ROOT / "crates"

DONE_CONSTRUCTOR_RE = re.compile(r"StreamChunk::Done\s*\{\s*request_id\s*:")


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


def emit_terminal_range(lines: list[str]) -> tuple[int, int] | None:
    start_idx = None
    for idx, line in enumerate(lines):
        if "async fn emit_terminal(" in line:
            start_idx = idx
            break
    if start_idx is None:
        return None

    balance = lines[start_idx].count("{") - lines[start_idx].count("}")
    i = start_idx + 1
    while i < len(lines) and balance > 0:
        balance += lines[i].count("{") - lines[i].count("}")
        i += 1
    return (start_idx + 1, i)


def main() -> int:
    violations: list[str] = []

    for path in SRC_ROOT.rglob("*.rs"):
        rel = path.relative_to(ROOT)
        text = path.read_text(encoding="utf-8")
        lines = text.splitlines()

        cfg_test_ranges = find_cfg_test_ranges(lines)
        terminal_range = emit_terminal_range(lines) if rel.as_posix() == "crates/nanobot-core/src/agent/mod.rs" else None

        for idx, line in enumerate(lines, start=1):
            if not DONE_CONSTRUCTOR_RE.search(line):
                continue

            if line_in_ranges(idx, cfg_test_ranges):
                continue

            if terminal_range is not None and terminal_range[0] <= idx <= terminal_range[1]:
                continue

            violations.append(f"{rel}:{idx}: direct StreamChunk::Done constructor outside emit_terminal")

    if violations:
        print("Terminal contract guard failed:")
        for item in violations:
            print(f"- {item}")
        return 1

    print("Terminal contract guard passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
