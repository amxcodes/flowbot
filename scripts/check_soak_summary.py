#!/usr/bin/env python3
import json
import math
import os
import re
import sys
from typing import NoReturn


def fail(message: str) -> NoReturn:
    print(f"[soak-summary-guard] ERROR: {message}")
    sys.exit(1)


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: check_soak_summary.py <soak-log-file>")

    log_file = sys.argv[1]
    try:
        with open(log_file, "r", encoding="utf-8") as f:
            lines = f.read().splitlines()
    except OSError as exc:
        fail(f"cannot read log file '{log_file}': {exc}")

    prefix = "SOAK_SUMMARY "
    summaries = [line[len(prefix) :].strip() for line in lines if line.strip().startswith(prefix)]

    if len(summaries) != 1:
        fail(f"expected exactly one SOAK_SUMMARY line, found {len(summaries)}")

    verdict_prefix = "SOAK_VERDICT "
    verdicts = [line[len(verdict_prefix) :].strip() for line in lines if line.strip().startswith(verdict_prefix)]
    if len(verdicts) != 1:
        fail(f"expected exactly one SOAK_VERDICT line, found {len(verdicts)}")

    verdict = verdicts[0]
    schema_match = re.search(r"\bschema=(\d+)\b", verdict)
    ok_match = re.search(r"\bok=(\d+)\b", verdict)
    reasons_match = re.search(r"\breasons=(\[[^\]]*\])", verdict)
    if schema_match is None or schema_match.group(1) != "1":
        fail("SOAK_VERDICT must include schema=1")
    if ok_match is None or ok_match.group(1) != "1":
        fail("SOAK_VERDICT must include ok=1")
    if reasons_match is None or reasons_match.group(1) != "[]":
        fail("SOAK_VERDICT must include reasons=[]")

    try:
        payload = json.loads(summaries[0])
    except json.JSONDecodeError as exc:
        fail(f"invalid SOAK_SUMMARY JSON: {exc}")

    required_keys = [
        "schema",
        "scenario",
        "status",
        "start_epoch_ms",
        "end_epoch_ms",
        "duration_secs",
        "elapsed_ms",
        "iterations",
        "session_pool",
        "ops_per_sec",
        "pending_key_count",
        "pending_index_count",
        "pending_index_peak",
        "correlation_key_count",
        "correlation_key_peak",
        "terminal_dedupe_key_count",
        "terminal_dedupe_key_peak",
        "terminal_dedupe_ttl_secs",
    ]
    missing = [k for k in required_keys if k not in payload]
    if missing:
        fail(f"missing required summary keys: {', '.join(missing)}")

    if payload.get("scenario") != "redis_store_soak_stability":
        fail("unexpected soak scenario value")
    if payload.get("status") != "pass":
        fail("soak summary status must be 'pass'")
    if payload.get("schema") != 1:
        fail("soak summary schema must be 1")

    iterations = payload.get("iterations")
    if not isinstance(iterations, (int, float)) or iterations <= 0:
        fail("iterations must be > 0")

    duration_secs = payload.get("duration_secs")
    if not isinstance(duration_secs, (int, float)) or duration_secs <= 0:
        fail("duration_secs must be > 0")

    elapsed_ms = payload.get("elapsed_ms")
    if not isinstance(elapsed_ms, (int, float)) or elapsed_ms <= 0:
        fail("elapsed_ms must be > 0")

    expected_duration = os.getenv("NANOBOT_REDIS_SOAK_DURATION_SECS")
    expected_secs = duration_secs
    if expected_duration is not None and expected_duration.strip() != "":
        try:
            expected_secs = float(expected_duration)
        except ValueError:
            fail("NANOBOT_REDIS_SOAK_DURATION_SECS must be numeric when set")

    elapsed_secs = float(elapsed_ms) / 1000.0
    lower = float(expected_secs) * 0.9
    upper = float(expected_secs) * 1.1
    if not (lower <= elapsed_secs <= upper):
        fail(
            "elapsed duration out of soft bound: "
            f"elapsed={elapsed_secs:.2f}s expected={float(expected_secs):.2f}s "
            f"bound=[{lower:.2f}s, {upper:.2f}s]"
        )

    start_ms = payload.get("start_epoch_ms")
    end_ms = payload.get("end_epoch_ms")
    if not isinstance(start_ms, (int, float)) or not isinstance(end_ms, (int, float)):
        fail("start_epoch_ms/end_epoch_ms must be numeric")
    if end_ms <= start_ms:
        fail("end_epoch_ms must be greater than start_epoch_ms")

    session_pool = payload.get("session_pool")
    if not isinstance(session_pool, (int, float)) or session_pool <= 0:
        fail("session_pool must be > 0")

    pending_key_count = payload.get("pending_key_count")
    pending_index_count = payload.get("pending_index_count")
    pending_index_peak = payload.get("pending_index_peak")
    correlation_key_count = payload.get("correlation_key_count")
    correlation_key_peak = payload.get("correlation_key_peak")
    terminal_dedupe_key_count = payload.get("terminal_dedupe_key_count")
    terminal_dedupe_key_peak = payload.get("terminal_dedupe_key_peak")
    terminal_dedupe_ttl_secs = payload.get("terminal_dedupe_ttl_secs")
    ops_per_sec = payload.get("ops_per_sec")

    numeric_fields = {
        "pending_key_count": pending_key_count,
        "pending_index_count": pending_index_count,
        "pending_index_peak": pending_index_peak,
        "correlation_key_count": correlation_key_count,
        "correlation_key_peak": correlation_key_peak,
        "terminal_dedupe_key_count": terminal_dedupe_key_count,
        "terminal_dedupe_key_peak": terminal_dedupe_key_peak,
        "terminal_dedupe_ttl_secs": terminal_dedupe_ttl_secs,
        "ops_per_sec": ops_per_sec,
    }
    for key, value in numeric_fields.items():
        if not isinstance(value, (int, float)):
            fail(f"{key} must be numeric")

    if int(pending_key_count) > 1:
        fail("pending_key_count must be <= 1")
    if int(pending_index_count) != 0:
        fail("pending_index_count must be 0")
    if int(correlation_key_count) != 0:
        fail("correlation_key_count must be 0")
    if int(terminal_dedupe_key_count) != 0:
        fail("terminal_dedupe_key_count must be 0")

    pending_peak_bound = int(session_pool) + 2
    if int(pending_index_peak) > pending_peak_bound:
        fail(f"pending_index_peak exceeds bound: {pending_index_peak} > {pending_peak_bound}")

    correlation_peak_bound = int(session_pool) * 2 + 4
    if int(correlation_key_peak) > correlation_peak_bound:
        fail(
            f"correlation_key_peak exceeds bound: {correlation_key_peak} > {correlation_peak_bound}"
        )

    dedupe_peak_bound = math.ceil(float(ops_per_sec) * float(terminal_dedupe_ttl_secs) * 1.5) + 50
    if int(terminal_dedupe_key_peak) > dedupe_peak_bound:
        fail(
            "terminal_dedupe_key_peak exceeds bound: "
            f"{terminal_dedupe_key_peak} > {dedupe_peak_bound}"
        )

    print("[soak-summary-guard] OK: soak summary present and valid")


if __name__ == "__main__":
    main()
