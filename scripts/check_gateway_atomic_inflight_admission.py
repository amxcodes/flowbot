#!/usr/bin/env python3
from __future__ import annotations

import pathlib
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
GATEWAY_FILE = ROOT / "crates" / "nanobot-core" / "src" / "gateway" / "mod.rs"


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


def main() -> int:
    lines = GATEWAY_FILE.read_text(encoding="utf-8").splitlines()
    test_ranges = find_cfg_test_ranges(lines)

    violations: list[str] = []
    forbidden = (".inflight_count(", ".register_inflight(")
    required = ".try_register_inflight("

    for idx, line in enumerate(lines, start=1):
        if in_ranges(idx, test_ranges):
            continue
        if any(token in line for token in forbidden):
            violations.append(
                f"{GATEWAY_FILE.relative_to(ROOT)}:{idx}: direct inflight access forbidden; use try_register_inflight"
            )

    if not any(required in line and not in_ranges(i + 1, test_ranges) for i, line in enumerate(lines)):
        violations.append(
            f"{GATEWAY_FILE.relative_to(ROOT)}: missing try_register_inflight usage in runtime websocket path"
        )

    if violations:
        print("Gateway atomic inflight admission guard failed:")
        for item in violations:
            print(f"- {item}")
        return 1

    print("Gateway atomic inflight admission guard passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
