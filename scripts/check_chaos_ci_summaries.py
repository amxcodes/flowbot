#!/usr/bin/env python3
import json
import sys
from collections import defaultdict


EXPECTED_SCENARIOS = {
    "slow_alive_fail_closed",
    "slow_alive_fail_open",
    "flaky_alternating",
}


def fail(message: str) -> None:
    print(f"[chaos-summary-guard] ERROR: {message}")
    sys.exit(1)


def parse_summaries(log_text: str):
    summaries = []
    prefix = "CHAOS_SUMMARY "
    for line in log_text.splitlines():
        line = line.strip()
        if not line.startswith(prefix):
            continue
        payload = line[len(prefix) :].strip()
        try:
            summaries.append(json.loads(payload))
        except json.JSONDecodeError as exc:
            fail(f"invalid JSON in summary line: {exc}")
    return summaries


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: check_chaos_ci_summaries.py <log-file>")

    log_file = sys.argv[1]
    try:
        with open(log_file, "r", encoding="utf-8") as f:
            log_text = f.read()
    except OSError as exc:
        fail(f"cannot read log file '{log_file}': {exc}")

    summaries = parse_summaries(log_text)
    if not summaries:
        fail("no CHAOS_SUMMARY lines found")

    by_scenario = defaultdict(list)
    for item in summaries:
        scenario = item.get("scenario")
        if not isinstance(scenario, str) or not scenario:
            fail(f"summary missing valid scenario: {item}")
        by_scenario[scenario].append(item)

    missing = sorted(EXPECTED_SCENARIOS - set(by_scenario.keys()))
    if missing:
        fail(f"missing required scenarios: {', '.join(missing)}")

    duplicates = sorted([name for name, items in by_scenario.items() if len(items) != 1])
    if duplicates:
        fail(f"scenarios must appear exactly once: {', '.join(duplicates)}")

    for scenario in sorted(EXPECTED_SCENARIOS):
        item = by_scenario[scenario][0]
        terminal_violations = item.get("terminal_violations")
        malformed_terminals = item.get("malformed_terminals")
        allowed = item.get("allowed")
        denied = item.get("denied")
        total_requests = item.get("total_requests")

        if not isinstance(terminal_violations, (int, float)) or terminal_violations > 0:
            fail(f"scenario '{scenario}' has terminal_violations > 0")
        if not isinstance(malformed_terminals, (int, float)) or malformed_terminals > 0:
            fail(f"scenario '{scenario}' has malformed_terminals > 0")

        if not isinstance(allowed, (int, float)) or not isinstance(denied, (int, float)):
            fail(f"scenario '{scenario}' missing allowed/denied counts")
        if not isinstance(total_requests, (int, float)):
            fail(f"scenario '{scenario}' missing total_requests")
        if int(allowed) + int(denied) != int(total_requests):
            fail(
                f"scenario '{scenario}' violates accounting: "
                f"allowed({allowed}) + denied({denied}) != total_requests({total_requests})"
            )

    print("[chaos-summary-guard] OK: all required chaos summaries present and valid")


if __name__ == "__main__":
    main()
