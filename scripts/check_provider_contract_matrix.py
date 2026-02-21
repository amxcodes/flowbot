#!/usr/bin/env python3
from __future__ import annotations

import pathlib
import re
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
AGENT_MOD = ROOT / "crates" / "nanobot-core" / "src" / "agent" / "mod.rs"

EXPECTED_PROVIDERS = {"antigravity", "google", "openai", "meta", "mock"}
REQUIRED_TESTS = {
    "provider_contract_matrix_declares_tool_behavior",
    "provider_contract_matrix_enforces_unsupported_tool_call_failure",
}


def main() -> int:
    text = AGENT_MOD.read_text(encoding="utf-8")

    provider_matches = set(
        re.findall(r'"([a-z_]+)"\s*=>\s*Some\(ProviderCapabilities\s*\{', text)
    )
    if provider_matches != EXPECTED_PROVIDERS:
        missing = sorted(EXPECTED_PROVIDERS - provider_matches)
        extra = sorted(provider_matches - EXPECTED_PROVIDERS)
        print("Provider contract matrix guard failed: capability declarations mismatch")
        if missing:
            print("Missing providers:")
            for p in missing:
                print(f"- {p}")
        if extra:
            print("Unexpected providers (update EXPECTED_PROVIDERS + tests):")
            for p in extra:
                print(f"- {p}")
        return 1

    for test_name in REQUIRED_TESTS:
        if f"fn {test_name}()" not in text:
            print(f"Provider contract matrix guard failed: missing test {test_name}")
            return 1

    print("Provider contract matrix guard passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
