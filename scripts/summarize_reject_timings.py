#!/usr/bin/env python3
import argparse
import json
import re
import statistics
from typing import Dict, List


FIELDS = [
    "recv_to_handler_start_ms",
    "handler_start_to_deadline_expired_ms",
    "deadline_expired_to_reject_emit_ms",
    "reject_emit_to_ws_send_complete_ms",
    "total_recv_to_ws_send_ms",
]


def percentile(values: List[float], p: float) -> float:
    if not values:
        return 0.0
    if len(values) == 1:
        return values[0]
    values = sorted(values)
    k = (len(values) - 1) * p
    f = int(k)
    c = min(f + 1, len(values) - 1)
    if f == c:
        return values[f]
    return values[f] * (c - k) + values[c] * (k - f)


def extract_json_objects(log_text: str) -> List[Dict[str, object]]:
    # Look for JSON payloads logged on reject path
    objs: List[Dict[str, object]] = []
    for line in log_text.splitlines():
        if "reject_timing" not in line:
            continue
        m = re.search(r"(\{\"deadline_expired_to_reject_emit_ms\".*\})", line)
        if not m:
            m = re.search(r"(\{.*\})", line)
        if not m:
            continue
        raw = m.group(1)
        try:
            obj = json.loads(raw)
        except json.JSONDecodeError:
            continue
        if obj.get("event") == "reject_timing":
            objs.append(obj)
    return objs


def render_table(rows: Dict[str, List[float]]) -> str:
    out = []
    out.append("Segment                              p50(ms)  p95(ms)  avg(ms)")
    out.append("---------------------------------------------------------------")
    for field in FIELDS:
        vals = rows.get(field, [])
        out.append(
            f"{field:<36} {percentile(vals, 0.50):>7.2f} {percentile(vals, 0.95):>8.2f} {statistics.mean(vals) if vals else 0.0:>8.2f}"
        )
    return "\n".join(out)


def main() -> int:
    parser = argparse.ArgumentParser(description="Summarize sampled reject timing logs")
    parser.add_argument("--log", default="scripts/load-smoke-server.log")
    parser.add_argument("--out")
    args = parser.parse_args()

    with open(args.log, "r", encoding="utf-8", errors="replace") as f:
        text = f.read()

    objs = extract_json_objects(text)
    rows: Dict[str, List[float]] = {k: [] for k in FIELDS}
    for obj in objs:
        for field in FIELDS:
            try:
                rows[field].append(float(obj.get(field, 0.0)))
            except (TypeError, ValueError):
                pass

    report = []
    report.append(f"sampled_reject_events={len(objs)}")
    report.append(render_table(rows))
    output = "\n".join(report)

    if args.out:
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(output)
            f.write("\n")

    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
