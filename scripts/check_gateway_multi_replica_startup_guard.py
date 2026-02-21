#!/usr/bin/env python3
from __future__ import annotations

import pathlib
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
GATEWAY_FILE = ROOT / "crates" / "nanobot-core" / "src" / "gateway" / "mod.rs"


def main() -> int:
    text = GATEWAY_FILE.read_text(encoding="utf-8")
    needle = "crate::distributed::enforce_multi_replica_runtime_readiness().await?;"
    if needle not in text:
        print("Gateway multi-replica startup guard failed:")
        print(
            f"- {GATEWAY_FILE.relative_to(ROOT)} missing startup call to enforce_multi_replica_runtime_readiness"
        )
        return 1
    print("Gateway multi-replica startup guard passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
