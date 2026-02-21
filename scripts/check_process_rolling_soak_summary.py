#!/usr/bin/env python3
import json
import re
import sys
from typing import NoReturn


def fail(message: str) -> NoReturn:
    print(f"[process-rolling-soak-guard] ERROR: {message}")
    sys.exit(1)


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: check_process_rolling_soak_summary.py <log-file>")

    log_file = sys.argv[1]
    try:
        with open(log_file, "r", encoding="utf-8") as f:
            lines = f.read().splitlines()
    except OSError as exc:
        fail(f"cannot read log file '{log_file}': {exc}")

    summary_prefix = "PROCESS_ROLLING_SOAK_SUMMARY "
    summaries = [line[len(summary_prefix) :].strip() for line in lines if line.strip().startswith(summary_prefix)]
    if len(summaries) != 1:
        fail(f"expected exactly one PROCESS_ROLLING_SOAK_SUMMARY line, found {len(summaries)}")

    verdict_prefix = "PROCESS_ROLLING_SOAK_VERDICT "
    verdicts = [line[len(verdict_prefix) :].strip() for line in lines if line.strip().startswith(verdict_prefix)]
    if len(verdicts) != 1:
        fail(f"expected exactly one PROCESS_ROLLING_SOAK_VERDICT line, found {len(verdicts)}")

    verdict = verdicts[0]
    schema_match = re.search(r"\bschema=(\d+)\b", verdict)
    ok_match = re.search(r"\bok=(\d+)\b", verdict)
    reasons_match = re.search(r"\breasons=(\[[^\]]*\])", verdict)
    if schema_match is None or schema_match.group(1) != "1":
        fail("PROCESS_ROLLING_SOAK_VERDICT must include schema=1")
    if ok_match is None or ok_match.group(1) != "1":
        fail("PROCESS_ROLLING_SOAK_VERDICT must include ok=1")
    if reasons_match is None or reasons_match.group(1) != "[]":
        fail("PROCESS_ROLLING_SOAK_VERDICT must include reasons=[]")

    try:
        payload = json.loads(summaries[0])
    except json.JSONDecodeError as exc:
        fail(f"invalid PROCESS_ROLLING_SOAK_SUMMARY JSON: {exc}")

    required = [
        "schema",
        "scenario",
        "status",
        "start_epoch_ms",
        "end_epoch_ms",
        "elapsed_ms",
        "total_waves",
        "requests_per_wave",
        "qps_limit",
        "node_a_down_waves",
        "node_a_restart_events",
        "total_requests",
        "total_allowed",
        "total_denied",
        "stuck_requests",
        "duplicate_terminal_ids",
        "terminal_violations",
        "malformed_terminals",
        "amplification_violation_count",
        "retry_to_node_b_count",
    ]
    missing = [k for k in required if k not in payload]
    if missing:
        fail(f"missing required keys: {', '.join(missing)}")

    if payload.get("schema") != 1:
        fail("summary schema must be 1")
    if payload.get("scenario") != "process_kill_rolling_restart_soak":
        fail("unexpected scenario value")
    if payload.get("status") != "pass":
        fail("status must be pass")

    numeric = [
        "start_epoch_ms",
        "end_epoch_ms",
        "elapsed_ms",
        "total_waves",
        "requests_per_wave",
        "qps_limit",
        "node_a_down_waves",
        "node_a_restart_events",
        "total_requests",
        "total_allowed",
        "total_denied",
        "stuck_requests",
        "duplicate_terminal_ids",
        "terminal_violations",
        "malformed_terminals",
        "amplification_violation_count",
        "retry_to_node_b_count",
    ]
    for key in numeric:
        if not isinstance(payload.get(key), (int, float)):
            fail(f"{key} must be numeric")

    if payload["end_epoch_ms"] <= payload["start_epoch_ms"]:
        fail("end_epoch_ms must be greater than start_epoch_ms")
    if int(payload["node_a_down_waves"]) <= 0:
        fail("node_a_down_waves must be > 0")
    if int(payload["node_a_restart_events"]) != 1:
        fail("node_a_restart_events must be exactly 1")
    if int(payload["total_requests"]) != int(payload["total_allowed"]) + int(payload["total_denied"]):
        fail("total_requests must equal total_allowed + total_denied")
    if int(payload["stuck_requests"]) != 0:
        fail("stuck_requests must be 0")
    if int(payload["duplicate_terminal_ids"]) != 0:
        fail("duplicate_terminal_ids must be 0")
    if int(payload["terminal_violations"]) != 0:
        fail("terminal_violations must be 0")
    if int(payload["malformed_terminals"]) != 0:
        fail("malformed_terminals must be 0")
    if int(payload["amplification_violation_count"]) != 0:
        fail("amplification_violation_count must be 0")

    print("[process-rolling-soak-guard] OK: process rolling soak summary present and valid")


if __name__ == "__main__":
    main()
