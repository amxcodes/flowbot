#!/usr/bin/env python3
import json
import re
import sys
from typing import NoReturn


def fail(message: str) -> NoReturn:
    print(f"[process-dedupe-race-guard] ERROR: {message}")
    sys.exit(1)


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: check_process_dedupe_race_summary.py <log-file>")

    log_file = sys.argv[1]
    try:
        with open(log_file, "r", encoding="utf-8") as f:
            lines = f.read().splitlines()
    except OSError as exc:
        fail(f"cannot read log file '{log_file}': {exc}")

    summary_prefix = "PROCESS_DEDUPE_RACE_SUMMARY "
    summaries = [line[len(summary_prefix) :].strip() for line in lines if line.strip().startswith(summary_prefix)]
    if len(summaries) != 1:
        fail(f"expected exactly one PROCESS_DEDUPE_RACE_SUMMARY line, found {len(summaries)}")

    verdict_prefix = "PROCESS_DEDUPE_RACE_VERDICT "
    verdicts = [line[len(verdict_prefix) :].strip() for line in lines if line.strip().startswith(verdict_prefix)]
    if len(verdicts) != 1:
        fail(f"expected exactly one PROCESS_DEDUPE_RACE_VERDICT line, found {len(verdicts)}")

    verdict = verdicts[0]
    schema_match = re.search(r"\bschema=(\d+)\b", verdict)
    ok_match = re.search(r"\bok=(\d+)\b", verdict)
    reasons_match = re.search(r"\breasons=(\[[^\]]*\])", verdict)
    if schema_match is None or schema_match.group(1) != "1":
        fail("PROCESS_DEDUPE_RACE_VERDICT must include schema=1")
    if ok_match is None or ok_match.group(1) != "1":
        fail("PROCESS_DEDUPE_RACE_VERDICT must include ok=1")
    if reasons_match is None or reasons_match.group(1) != "[]":
        fail("PROCESS_DEDUPE_RACE_VERDICT must include reasons=[]")

    try:
        payload = json.loads(summaries[0])
    except json.JSONDecodeError as exc:
        fail(f"invalid PROCESS_DEDUPE_RACE_SUMMARY JSON: {exc}")

    required = [
        "schema",
        "scenario",
        "status",
        "request_id",
        "winner_count",
        "duplicate_counter_increase",
        "loser_text_delta_count",
    ]
    missing = [k for k in required if k not in payload]
    if missing:
        fail(f"missing required keys: {', '.join(missing)}")

    if payload.get("schema") != 1:
        fail("summary schema must be 1")
    if payload.get("scenario") != "process_ws_dual_emit_dedupe_race":
        fail("unexpected scenario value")
    if payload.get("status") != "pass":
        fail("status must be pass")
    if not isinstance(payload.get("request_id"), str) or not payload.get("request_id"):
        fail("request_id must be non-empty string")
    if int(payload.get("winner_count", 0)) != 1:
        fail("winner_count must be exactly 1")
    if float(payload.get("duplicate_counter_increase", 0.0)) < 1.0:
        fail("duplicate_counter_increase must be >= 1.0")
    if int(payload.get("loser_text_delta_count", 0)) != 0:
        fail("loser_text_delta_count must be 0")

    print("[process-dedupe-race-guard] OK: process dedupe race summary present and valid")


if __name__ == "__main__":
    main()
