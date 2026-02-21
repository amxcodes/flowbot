#!/usr/bin/env python3
from __future__ import annotations

import pathlib
import re
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
GATEWAY_DIR = ROOT / "crates" / "nanobot-core" / "src" / "gateway"
GATEWAY_MOD = GATEWAY_DIR / "mod.rs"
BIN_DIR = ROOT / "crates" / "nanobot-core" / "src" / "bin"


def main() -> int:
    mod_text = GATEWAY_MOD.read_text(encoding="utf-8")
    mod_lines = mod_text.splitlines()

    helper_start = None
    helper_end = None
    for idx, line in enumerate(mod_lines, start=1):
        if "async fn send_ws_text_timed(" in line:
            helper_start = idx
            break

    if helper_start is None:
        print("WS timed send guard failed: send_ws_text_timed helper not found")
        return 1

    balance = 0
    for idx in range(helper_start - 1, len(mod_lines)):
        line = mod_lines[idx]
        balance += line.count("{") - line.count("}")
        if idx + 1 > helper_start and balance <= 0:
            helper_end = idx + 1
            break

    if helper_end is None:
        print("WS timed send guard failed: could not determine helper bounds")
        return 1

    pattern = re.compile(r"\.send\(WsMessage::Text\(")
    violations: list[str] = []

    for idx, line in enumerate(mod_lines, start=1):
        if not pattern.search(line):
            continue
        if helper_start <= idx <= helper_end:
            continue
        violations.append(f"{GATEWAY_MOD.relative_to(ROOT)}:{idx}")

    for path in GATEWAY_DIR.rglob("*.rs"):
        if path == GATEWAY_MOD:
            continue
        text = path.read_text(encoding="utf-8")
        lines = text.splitlines()
        for idx, line in enumerate(lines, start=1):
            if pattern.search(line):
                violations.append(f"{path.relative_to(ROOT)}:{idx}")

    if BIN_DIR.exists():
        for path in BIN_DIR.rglob("*.rs"):
            text = path.read_text(encoding="utf-8")
            lines = text.splitlines()
            for idx, line in enumerate(lines, start=1):
                if pattern.search(line):
                    violations.append(f"{path.relative_to(ROOT)}:{idx}")

    if violations:
        print("WS timed send guard failed: raw WsMessage::Text sends found outside send_ws_text_timed")
        for v in violations:
            print(f"- {v}")
        return 1

    print("WS timed send guard passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
