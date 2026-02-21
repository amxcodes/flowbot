#!/usr/bin/env python3
import json
import re
import sys
from typing import NoReturn


def fail(message: str) -> NoReturn:
    print(f"[rolling-soak-guard] ERROR: {message}")
    sys.exit(1)


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: check_rolling_soak_summary.py <rolling-soak-log-file>")

    log_file = sys.argv[1]
    try:
        with open(log_file, "r", encoding="utf-8") as f:
            lines = f.read().splitlines()
    except OSError as exc:
        fail(f"cannot read log file '{log_file}': {exc}")

    summary_prefix = "ROLLING_SOAK_SUMMARY "
    summaries = [
        line[len(summary_prefix) :].strip()
        for line in lines
        if line.strip().startswith(summary_prefix)
    ]
    if len(summaries) != 1:
        fail(f"expected exactly one ROLLING_SOAK_SUMMARY line, found {len(summaries)}")

    verdict_prefix = "ROLLING_SOAK_VERDICT "
    verdicts = [
        line[len(verdict_prefix) :].strip()
        for line in lines
        if line.strip().startswith(verdict_prefix)
    ]
    if len(verdicts) != 1:
        fail(f"expected exactly one ROLLING_SOAK_VERDICT line, found {len(verdicts)}")

    verdict = verdicts[0]
    schema_match = re.search(r"\bschema=(\d+)\b", verdict)
    ok_match = re.search(r"\bok=(\d+)\b", verdict)
    reasons_match = re.search(r"\breasons=(\[[^\]]*\])", verdict)
    if schema_match is None or schema_match.group(1) != "1":
        fail("ROLLING_SOAK_VERDICT must include schema=1")
    if ok_match is None or ok_match.group(1) != "1":
        fail("ROLLING_SOAK_VERDICT must include ok=1")
    if reasons_match is None or reasons_match.group(1) != "[]":
        fail("ROLLING_SOAK_VERDICT must include reasons=[]")

    try:
        payload = json.loads(summaries[0])
    except json.JSONDecodeError as exc:
        fail(f"invalid ROLLING_SOAK_SUMMARY JSON: {exc}")

    required_keys = [
        "schema",
        "scenario",
        "status",
        "start_epoch_ms",
        "end_epoch_ms",
        "elapsed_ms",
        "total_waves",
        "node_a_down_waves",
        "node_a_restart_events",
        "requests_per_wave",
        "qps_limit",
        "total_requests",
        "total_allowed",
        "total_denied",
        "total_terminals",
        "stuck_requests",
        "terminal_violations",
        "malformed_terminals",
        "amplification_violation_count",
        "dedupe_key_count_end",
        "correlation_key_count_end",
    ]
    missing = [k for k in required_keys if k not in payload]
    if missing:
        fail(f"missing required summary keys: {', '.join(missing)}")

    if payload.get("schema") != 1:
        fail("summary schema must be 1")
    if payload.get("scenario") != "redis_rolling_restart_soak":
        fail("unexpected rolling soak scenario value")
    if payload.get("status") != "pass":
        fail("rolling soak summary status must be 'pass'")

    numeric_keys = [
        "start_epoch_ms",
        "end_epoch_ms",
        "elapsed_ms",
        "total_waves",
        "node_a_down_waves",
        "node_a_restart_events",
        "requests_per_wave",
        "qps_limit",
        "total_requests",
        "total_allowed",
        "total_denied",
        "total_terminals",
        "stuck_requests",
        "terminal_violations",
        "malformed_terminals",
        "amplification_violation_count",
        "dedupe_key_count_end",
        "correlation_key_count_end",
    ]
    for key in numeric_keys:
        if not isinstance(payload.get(key), (int, float)):
            fail(f"{key} must be numeric")

    if payload["end_epoch_ms"] <= payload["start_epoch_ms"]:
        fail("end_epoch_ms must be greater than start_epoch_ms")
    if payload["total_waves"] < 10:
        fail("total_waves must be >= 10")
    if payload["node_a_down_waves"] <= 0:
        fail("node_a_down_waves must be > 0")
    if payload["node_a_restart_events"] != 1:
        fail("node_a_restart_events must be exactly 1")

    if int(payload["total_requests"]) != int(payload["total_allowed"]) + int(payload["total_denied"]):
        fail("total_requests must equal total_allowed + total_denied")
    if int(payload["total_requests"]) != int(payload["total_terminals"]):
        fail("total_requests must equal total_terminals")
    if int(payload["stuck_requests"]) != 0:
        fail("stuck_requests must be 0")
    if int(payload["terminal_violations"]) != 0:
        fail("terminal_violations must be 0")
    if int(payload["malformed_terminals"]) != 0:
        fail("malformed_terminals must be 0")
    if int(payload["amplification_violation_count"]) != 0:
        fail("amplification_violation_count must be 0")
    if int(payload["dedupe_key_count_end"]) != 0:
        fail("dedupe_key_count_end must be 0")
    if int(payload["correlation_key_count_end"]) != 0:
        fail("correlation_key_count_end must be 0")

    print("[rolling-soak-guard] OK: rolling soak summary present and valid")


if __name__ == "__main__":
    main()
