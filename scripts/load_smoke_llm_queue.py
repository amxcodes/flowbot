#!/usr/bin/env python3
import argparse
import asyncio
import json
import os
import re
import statistics
import time
import urllib.request
from typing import Dict, List, Optional, Tuple

import websockets


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


def read_metrics(metrics_url: str) -> str:
    with urllib.request.urlopen(metrics_url, timeout=10) as resp:
        return resp.read().decode("utf-8", errors="replace")


def is_ci() -> bool:
    value = os.getenv("CI", "").strip().lower()
    return value in {"1", "true", "yes", "on"}


def metric_value(metrics_text: str, metric_name: str) -> float:
    value = 0.0
    for line in metrics_text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith(metric_name + " "):
            try:
                value = float(line.split()[-1])
            except ValueError:
                pass
    return value


def metric_label_values(
    metrics_text: str, metric_prefix: str, label_key: str = "reason"
) -> Dict[str, float]:
    out: Dict[str, float] = {}
    pattern = re.compile(rf"^{re.escape(metric_prefix)}\{{([^}}]+)\}}\s+([0-9eE+\-.]+)$")
    for line in metrics_text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        m = pattern.match(line)
        if not m:
            continue
        labels_str = m.group(1)
        value_str = m.group(2)
        try:
            value = float(value_str)
        except ValueError:
            continue
        labels = {}
        for part in labels_str.split(","):
            if "=" not in part:
                continue
            k, v = part.split("=", 1)
            labels[k.strip()] = v.strip().strip('"')
        label_value = labels.get(label_key)
        if label_value:
            out[label_value] = value
    return out


def classify_reject_reason(error_text: str) -> str:
    low = error_text.lower()
    if "provider unavailable" in low:
        return "provider_unhealthy"
    if "upstream llm timeout" in low or "(503)" in low:
        return "provider_timeout"
    if "(429)" in low or "too many concurrent" in low or "server busy" in low:
        return "semaphore_timeout"
    return "unknown"


async def send_one(
    ws_url: str,
    prompt: str,
    timeout_s: float,
    conn_counter: Optional[Dict[str, int]] = None,
    conn_lock: Optional[asyncio.Lock] = None,
) -> Tuple[bool, bool, float, float, Optional[float], str, Optional[str]]:
    started = time.perf_counter()
    request_started = 0.0
    busy = False
    ok = False
    error_text = ""
    reject_reason: Optional[str] = None
    reject_signal_elapsed_ms: Optional[float] = None

    try:
        async with websockets.connect(ws_url, max_size=2_000_000) as ws:
            if conn_counter is not None and conn_lock is not None:
                async with conn_lock:
                    conn_counter["current"] = conn_counter.get("current", 0) + 1
                    conn_counter["peak"] = max(
                        conn_counter.get("peak", 0), conn_counter["current"]
                    )
            await ws.send("{}")
            init_raw = await asyncio.wait_for(ws.recv(), timeout=timeout_s)
            init = json.loads(init_raw)
            token = init.get("token", "")
            req_id = "req-1"
            request_started = time.perf_counter()
            await ws.send(
                json.dumps(
                    {
                        "type": "req",
                        "id": req_id,
                        "method": "send",
                        "params": {"message": prompt, "token": token},
                    }
                )
            )

            while True:
                raw = await asyncio.wait_for(ws.recv(), timeout=timeout_s)
                msg = json.loads(raw)
                msg_type = msg.get("type")

                if msg_type == "res":
                    if msg.get("id") == req_id and not msg.get("ok", False):
                        error = msg.get("error", {})
                        error_text = str(error.get("message", "request rejected"))
                        if "Server busy" in error_text or "too many concurrent" in error_text:
                            busy = True
                            reject_reason = classify_reject_reason(error_text)
                            if request_started > 0 and reject_signal_elapsed_ms is None:
                                reject_signal_elapsed_ms = (
                                    time.perf_counter() - request_started
                                ) * 1000.0
                        break
                    continue

                if msg_type == "event" and msg.get("event") == "agent.done":
                    ok = not busy
                    if busy and not error_text:
                        error_text = "Server busy"
                    if busy:
                        reject_reason = classify_reject_reason(error_text)
                        if request_started > 0 and reject_signal_elapsed_ms is None:
                            reject_signal_elapsed_ms = (
                                time.perf_counter() - request_started
                            ) * 1000.0
                    break

                if msg_type == "event" and msg.get("event") == "agent.delta":
                    payload = msg.get("payload", {})
                    delta = str(payload.get("delta", ""))
                    if "Server busy" in delta or "too many concurrent" in delta:
                        busy = True
                        reject_reason = classify_reject_reason(delta)
                        if request_started > 0 and reject_signal_elapsed_ms is None:
                            reject_signal_elapsed_ms = (
                                time.perf_counter() - request_started
                            ) * 1000.0

                if msg_type == "done":
                    ok = True
                    break

                if msg_type == "error":
                    error_text = str(msg.get("error", ""))
                    if "Server busy" in error_text or "too many concurrent" in error_text:
                        busy = True
                        reject_reason = classify_reject_reason(error_text)
                        if request_started > 0 and reject_signal_elapsed_ms is None:
                            reject_signal_elapsed_ms = (
                                time.perf_counter() - request_started
                            ) * 1000.0
                    break

                if msg_type == "text_delta":
                    delta = str(msg.get("delta", ""))
                    if "Server busy" in delta or "too many concurrent" in delta:
                        busy = True
                        reject_reason = classify_reject_reason(delta)
                        if request_started > 0 and reject_signal_elapsed_ms is None:
                            reject_signal_elapsed_ms = (
                                time.perf_counter() - request_started
                            ) * 1000.0

    except Exception as exc:
        error_text = str(exc)
    finally:
        if conn_counter is not None and conn_lock is not None:
            async with conn_lock:
                conn_counter["current"] = max(0, conn_counter.get("current", 0) - 1)

    elapsed_ms = (time.perf_counter() - started) * 1000.0
    request_elapsed_ms = (
        (time.perf_counter() - request_started) * 1000.0 if request_started > 0 else elapsed_ms
    )
    return (
        ok,
        busy,
        elapsed_ms,
        request_elapsed_ms,
        reject_signal_elapsed_ms,
        error_text,
        reject_reason,
    )


async def main_async(args: argparse.Namespace) -> int:
    ci_mode = is_ci()
    allow_offline = args.allow_offline or os.getenv("ALLOW_OFFLINE", "") == "1"

    try:
        metrics_before = read_metrics(args.metrics_url)
    except Exception as exc:
        status = "failed" if (ci_mode and not allow_offline) else "skipped"
        print(
            json.dumps(
                {
                    "status": status,
                    "reason": "metrics_unreachable",
                    "metrics_url": args.metrics_url,
                    "ci": ci_mode,
                    "allow_offline": allow_offline,
                    "error": str(exc),
                },
                indent=2,
            )
        )
        return 1 if status == "failed" else 0

    sem = asyncio.Semaphore(args.concurrency)
    results: List[Tuple[bool, bool, float, float, Optional[float], str, Optional[str]]] = []
    conn_counter: Dict[str, int] = {"current": 0, "peak": 0}
    conn_lock = asyncio.Lock()

    async def runner() -> None:
        async with sem:
            res = await send_one(
                args.ws_url,
                args.prompt,
                args.timeout,
                conn_counter=conn_counter,
                conn_lock=conn_lock,
            )
            results.append(res)

    await asyncio.gather(*(runner() for _ in range(args.requests)))

    try:
        metrics_after = read_metrics(args.metrics_url)
    except Exception as exc:
        status = "failed" if (ci_mode and not allow_offline) else "partial"
        print(
            json.dumps(
                {
                    "status": status,
                    "reason": "metrics_unreachable_after_run",
                    "metrics_url": args.metrics_url,
                    "ci": ci_mode,
                    "allow_offline": allow_offline,
                    "error": str(exc),
                },
                indent=2,
            )
        )
        return 1 if status == "failed" else 0

    ok_count = sum(1 for ok, _, _, _, _, _, _ in results if ok)
    busy_count = sum(1 for _, busy, _, _, _, _, _ in results if busy)
    err_count = sum(1 for ok, _, _, _, _, err, _ in results if not ok and err)
    latencies = [ms for _, _, ms, _, _, _, _ in results]
    request_latencies = [ms for _, _, _, ms, _, _, _ in results]
    success_latencies = [ms for ok, _, ms, _, _, _, _ in results if ok]
    success_request_latencies = [ms for ok, _, _, ms, _, _, _ in results if ok]
    reject_latencies = [ms for ok, busy, ms, _, _, _, _ in results if not ok and busy]
    reject_request_latencies = [ms for ok, busy, _, ms, _, _, _ in results if not ok and busy]
    reject_signal_latencies = [
        signal
        for ok, busy, _, _, signal, _, _ in results
        if not ok and busy and signal is not None
    ]
    reject_reason_latencies: Dict[str, List[float]] = {}
    for ok, busy, _, req_ms, _, _, reason in results:
        if ok or not busy:
            continue
        key = reason or "unknown"
        reject_reason_latencies.setdefault(key, []).append(req_ms)
    error_samples: List[str] = []
    for ok, _, _, _, _, err, _ in results:
        if not ok and err and err not in error_samples:
            error_samples.append(err)
        if len(error_samples) >= 3:
            break

    wait_total_before = metric_value(metrics_before, "llm_task_semaphore_wait_seconds_duration_seconds")
    wait_total_after = metric_value(metrics_after, "llm_task_semaphore_wait_seconds_duration_seconds")
    wait_count_before = metric_value(metrics_before, "llm_task_semaphore_wait_seconds_total")
    wait_count_after = metric_value(metrics_after, "llm_task_semaphore_wait_seconds_total")
    dispatch_total_before = metric_value(metrics_before, "llm_dispatch_delay_seconds_duration_seconds")
    dispatch_total_after = metric_value(metrics_after, "llm_dispatch_delay_seconds_duration_seconds")
    dispatch_count_before = metric_value(metrics_before, "llm_dispatch_delay_seconds_total")
    dispatch_count_after = metric_value(metrics_after, "llm_dispatch_delay_seconds_total")
    reject_emit_total_before = metric_value(metrics_before, "llm_reject_emit_delay_seconds_duration_seconds")
    reject_emit_total_after = metric_value(metrics_after, "llm_reject_emit_delay_seconds_duration_seconds")
    reject_emit_count_before = metric_value(metrics_before, "llm_reject_emit_delay_seconds_total")
    reject_emit_count_after = metric_value(metrics_after, "llm_reject_emit_delay_seconds_total")
    handler_reject_total_before = metric_value(metrics_before, "llm_handler_to_reject_decision_seconds_duration_seconds")
    handler_reject_total_after = metric_value(metrics_after, "llm_handler_to_reject_decision_seconds_duration_seconds")
    handler_reject_count_before = metric_value(metrics_before, "llm_handler_to_reject_decision_seconds_total")
    handler_reject_count_after = metric_value(metrics_after, "llm_handler_to_reject_decision_seconds_total")
    service_total_before = metric_value(metrics_before, "llm_service_time_seconds_duration_seconds")
    service_total_after = metric_value(metrics_after, "llm_service_time_seconds_duration_seconds")
    service_count_before = metric_value(metrics_before, "llm_service_time_seconds_total")
    service_count_after = metric_value(metrics_after, "llm_service_time_seconds_total")
    ws_send_total_before = metric_value(metrics_before, "ws_send_wait_seconds_duration_seconds")
    ws_send_total_after = metric_value(metrics_after, "ws_send_wait_seconds_duration_seconds")
    ws_send_count_before = metric_value(metrics_before, "ws_send_wait_seconds_total")
    ws_send_count_after = metric_value(metrics_after, "ws_send_wait_seconds_total")
    ws_send_inflight_peak_before = metric_value(metrics_before, "llm_ws_send_inflight_peak")
    ws_send_inflight_peak_after = metric_value(metrics_after, "llm_ws_send_inflight_peak")
    ws_send_inflight_current_after = metric_value(metrics_after, "llm_ws_send_inflight")
    gateway_ws_send_total_before = metric_value(metrics_before, "gateway_ws_send_wait_seconds_duration_seconds")
    gateway_ws_send_total_after = metric_value(metrics_after, "gateway_ws_send_wait_seconds_duration_seconds")
    gateway_ws_send_count_before = metric_value(metrics_before, "gateway_ws_send_wait_seconds_total")
    gateway_ws_send_count_after = metric_value(metrics_after, "gateway_ws_send_wait_seconds_total")
    gateway_recv_to_agent_total_before = metric_value(metrics_before, "gateway_ws_recv_to_agent_send_seconds_duration_seconds")
    gateway_recv_to_agent_total_after = metric_value(metrics_after, "gateway_ws_recv_to_agent_send_seconds_duration_seconds")
    gateway_recv_to_agent_count_before = metric_value(metrics_before, "gateway_ws_recv_to_agent_send_seconds_total")
    gateway_recv_to_agent_count_after = metric_value(metrics_after, "gateway_ws_recv_to_agent_send_seconds_total")
    gateway_agent_send_wait_total_before = metric_value(metrics_before, "gateway_ws_agent_send_wait_seconds_duration_seconds")
    gateway_agent_send_wait_total_after = metric_value(metrics_after, "gateway_ws_agent_send_wait_seconds_duration_seconds")
    gateway_agent_send_wait_count_before = metric_value(metrics_before, "gateway_ws_agent_send_wait_seconds_total")
    gateway_agent_send_wait_count_after = metric_value(metrics_after, "gateway_ws_agent_send_wait_seconds_total")
    handlers_peak_before = metric_value(metrics_before, "llm_active_handlers_peak")
    handlers_peak_after = metric_value(metrics_after, "llm_active_handlers_peak")
    handlers_current_after = metric_value(metrics_after, "llm_active_handlers")
    in_service_peak_before = metric_value(metrics_before, "llm_in_service_peak")
    in_service_peak_after = metric_value(metrics_after, "llm_in_service_peak")
    in_service_current_after = metric_value(metrics_after, "llm_in_service_current")
    cfg_concurrency_limit = metric_value(metrics_after, "llm_config_concurrency_limit")
    cfg_queue_wait_ms = metric_value(metrics_after, "llm_config_queue_wait_timeout_ms")
    cfg_mock_enabled = metric_value(metrics_after, "llm_config_mock_provider_enabled")
    cfg_bench_mode = metric_value(metrics_after, "llm_config_bench_mode_enabled")
    cfg_bench_no_persistence = metric_value(metrics_after, "llm_config_bench_no_persistence")
    cfg_timeout_remaining_last = metric_value(metrics_after, "llm_timeout_remaining_ms_last")
    rejected_before = metric_value(metrics_before, "llm_rejected_total{reason=semaphore_timeout}")
    rejected_after = metric_value(metrics_after, "llm_rejected_total{reason=semaphore_timeout}")
    rejected_reasons_before = metric_label_values(metrics_before, "llm_rejected_total", "reason")
    rejected_reasons_after = metric_label_values(metrics_after, "llm_rejected_total", "reason")
    timeout_paths_before = metric_label_values(metrics_before, "llm_timeout_path_total", "path")
    timeout_paths_after = metric_label_values(metrics_after, "llm_timeout_path_total", "path")
    llm_wait_samples = int(wait_count_after - wait_count_before)
    llm_service_samples = int(service_count_after - service_count_before)
    dispatch_samples = int(dispatch_count_after - dispatch_count_before)
    reject_emit_samples = int(reject_emit_count_after - reject_emit_count_before)
    ws_send_samples = int(ws_send_count_after - ws_send_count_before)
    gateway_ws_send_samples = int(gateway_ws_send_count_after - gateway_ws_send_count_before)
    handler_reject_samples = int(handler_reject_count_after - handler_reject_count_before)
    handler_reject_avg_seconds = (
        (handler_reject_total_after - handler_reject_total_before) / handler_reject_samples
        if handler_reject_samples > 0
        else 0.0
    )
    provider_unhealthy_count = int(
        rejected_reasons_after.get("provider_unhealthy", 0.0)
        - rejected_reasons_before.get("provider_unhealthy", 0.0)
    )
    gateway_recv_to_agent_samples = int(
        gateway_recv_to_agent_count_after - gateway_recv_to_agent_count_before
    )
    gateway_agent_send_wait_samples = int(
        gateway_agent_send_wait_count_after - gateway_agent_send_wait_count_before
    )

    summary: Dict[str, object] = {
        "status": "ok",
        "requests": args.requests,
        "concurrency": args.concurrency,
        "connections_used": args.requests,
        "connections_peak": conn_counter.get("peak", 0),
        "ok": ok_count,
        "busy": busy_count,
        "errors": err_count,
        "rejection_rate": round((busy_count / args.requests) if args.requests else 0.0, 6),
        "error_samples": error_samples,
        "latency_ms": {
            "p50": round(percentile(latencies, 0.50), 2),
            "p95": round(percentile(latencies, 0.95), 2),
            "p99": round(percentile(latencies, 0.99), 2),
            "avg": round(statistics.mean(latencies) if latencies else 0.0, 2),
        },
        "request_latency_ms": {
            "p50": round(percentile(request_latencies, 0.50), 2),
            "p95": round(percentile(request_latencies, 0.95), 2),
            "p99": round(percentile(request_latencies, 0.99), 2),
            "avg": round(statistics.mean(request_latencies) if request_latencies else 0.0, 2),
        },
        "success_latency_ms": {
            "p50": round(percentile(success_latencies, 0.50), 2),
            "p95": round(percentile(success_latencies, 0.95), 2),
            "p99": round(percentile(success_latencies, 0.99), 2),
            "avg": round(statistics.mean(success_latencies) if success_latencies else 0.0, 2),
        },
        "success_request_latency_ms": {
            "p50": round(percentile(success_request_latencies, 0.50), 2),
            "p95": round(percentile(success_request_latencies, 0.95), 2),
            "p99": round(percentile(success_request_latencies, 0.99), 2),
            "avg": round(
                statistics.mean(success_request_latencies)
                if success_request_latencies
                else 0.0,
                2,
            ),
        },
        "reject_latency_ms": {
            "p50": round(percentile(reject_latencies, 0.50), 2),
            "p95": round(percentile(reject_latencies, 0.95), 2),
            "avg": round(statistics.mean(reject_latencies) if reject_latencies else 0.0, 2),
        },
        "reject_request_latency_ms": {
            "p50": round(percentile(reject_request_latencies, 0.50), 2),
            "p95": round(percentile(reject_request_latencies, 0.95), 2),
            "avg": round(
                statistics.mean(reject_request_latencies)
                if reject_request_latencies
                else 0.0,
                2,
            ),
        },
        "reject_signal_latency_ms": {
            "p50": round(percentile(reject_signal_latencies, 0.50), 2),
            "p95": round(percentile(reject_signal_latencies, 0.95), 2),
            "avg": round(
                statistics.mean(reject_signal_latencies)
                if reject_signal_latencies
                else 0.0,
                2,
            ),
        },
        "reject_latency_by_reason_ms": {
            reason: {
                "p50": round(percentile(vals, 0.50), 2),
                "p95": round(percentile(vals, 0.95), 2),
                "avg": round(statistics.mean(vals), 2),
                "count": len(vals),
            }
            for reason, vals in sorted(reject_reason_latencies.items())
            if vals
        },
        "metrics_delta": {
            "llm_wait_total_seconds": round(wait_total_after - wait_total_before, 6),
            "llm_wait_samples": int(wait_count_after - wait_count_before),
            "llm_wait_avg_seconds": round(
                ((wait_total_after - wait_total_before) / llm_wait_samples)
                if llm_wait_samples > 0
                else 0.0,
                6,
            ),
            "llm_dispatch_delay_total_seconds": round(
                dispatch_total_after - dispatch_total_before,
                6,
            ),
            "llm_dispatch_delay_samples": dispatch_samples,
            "llm_dispatch_delay_avg_seconds": round(
                ((dispatch_total_after - dispatch_total_before) / dispatch_samples)
                if dispatch_samples > 0
                else 0.0,
                6,
            ),
            "llm_reject_emit_delay_total_seconds": round(
                reject_emit_total_after - reject_emit_total_before,
                6,
            ),
            "llm_reject_emit_delay_samples": reject_emit_samples,
            "llm_reject_emit_delay_avg_seconds": round(
                ((reject_emit_total_after - reject_emit_total_before) / reject_emit_samples)
                if reject_emit_samples > 0
                else 0.0,
                6,
            ),
            "llm_handler_to_reject_decision_total_seconds": round(
                handler_reject_total_after - handler_reject_total_before,
                6,
            ),
            "llm_handler_to_reject_decision_samples": handler_reject_samples,
            "llm_handler_to_reject_decision_avg_seconds": round(
                handler_reject_avg_seconds,
                6,
            ),
            "llm_service_total_seconds": round(service_total_after - service_total_before, 6),
            "llm_service_samples": llm_service_samples,
            "llm_service_avg_seconds": round(
                ((service_total_after - service_total_before) / llm_service_samples)
                if llm_service_samples > 0
                else 0.0,
                6,
            ),
            "ws_send_wait_total_seconds": round(ws_send_total_after - ws_send_total_before, 6),
            "ws_send_wait_samples": ws_send_samples,
            "ws_send_wait_avg_seconds": round(
                ((ws_send_total_after - ws_send_total_before) / ws_send_samples)
                if ws_send_samples > 0
                else 0.0,
                6,
            ),
            "gateway_ws_send_wait_total_seconds": round(
                gateway_ws_send_total_after - gateway_ws_send_total_before,
                6,
            ),
            "gateway_ws_send_wait_samples": gateway_ws_send_samples,
            "gateway_ws_send_wait_avg_seconds": round(
                ((gateway_ws_send_total_after - gateway_ws_send_total_before)
                 / gateway_ws_send_samples)
                if gateway_ws_send_samples > 0
                else 0.0,
                6,
            ),
            "gateway_ws_recv_to_agent_send_total_seconds": round(
                gateway_recv_to_agent_total_after - gateway_recv_to_agent_total_before,
                6,
            ),
            "gateway_ws_recv_to_agent_send_samples": gateway_recv_to_agent_samples,
            "gateway_ws_recv_to_agent_send_avg_seconds": round(
                ((gateway_recv_to_agent_total_after - gateway_recv_to_agent_total_before)
                 / gateway_recv_to_agent_samples)
                if gateway_recv_to_agent_samples > 0
                else 0.0,
                6,
            ),
            "gateway_ws_agent_send_wait_total_seconds": round(
                gateway_agent_send_wait_total_after - gateway_agent_send_wait_total_before,
                6,
            ),
            "gateway_ws_agent_send_wait_samples": gateway_agent_send_wait_samples,
            "gateway_ws_agent_send_wait_avg_seconds": round(
                ((gateway_agent_send_wait_total_after - gateway_agent_send_wait_total_before)
                 / gateway_agent_send_wait_samples)
                if gateway_agent_send_wait_samples > 0
                else 0.0,
                6,
            ),
            "ws_send_inflight_peak": int(ws_send_inflight_peak_after),
            "ws_send_inflight_peak_delta": int(
                max(0.0, ws_send_inflight_peak_after - ws_send_inflight_peak_before)
            ),
            "ws_send_inflight_current": int(ws_send_inflight_current_after),
            "llm_active_handlers_peak": int(handlers_peak_after),
            "llm_active_handlers_peak_delta": int(
                max(0.0, handlers_peak_after - handlers_peak_before)
            ),
            "llm_active_handlers_current": int(handlers_current_after),
            "llm_in_service_peak": int(in_service_peak_after),
            "llm_in_service_peak_delta": int(
                max(0.0, in_service_peak_after - in_service_peak_before)
            ),
            "llm_in_service_current": int(in_service_current_after),
            "llm_queue_depth_estimate": int(
                max(0.0, handlers_current_after - in_service_current_after)
            ),
            "llm_queue_depth_peak_estimate": int(
                max(0.0, handlers_peak_after - in_service_peak_after)
            ),
            "llm_rejected": int(rejected_after - rejected_before),
            "llm_rejected_by_reason": {
                reason: int(rejected_reasons_after.get(reason, 0.0) - rejected_reasons_before.get(reason, 0.0))
                for reason in sorted(set(rejected_reasons_before) | set(rejected_reasons_after))
                if int(rejected_reasons_after.get(reason, 0.0) - rejected_reasons_before.get(reason, 0.0)) != 0
            },
            "llm_timeout_path_counts": {
                path: int(timeout_paths_after.get(path, 0.0) - timeout_paths_before.get(path, 0.0))
                for path in sorted(set(timeout_paths_before) | set(timeout_paths_after))
                if int(timeout_paths_after.get(path, 0.0) - timeout_paths_before.get(path, 0.0)) != 0
            },
            "llm_rejection_rate": round(
                ((rejected_after - rejected_before) / args.requests) if args.requests else 0.0,
                6,
            ),
        },
        "runtime_config": {
            "llm_concurrency_limit": int(cfg_concurrency_limit),
            "llm_queue_wait_timeout_ms": int(cfg_queue_wait_ms),
            "mock_provider_enabled": bool(cfg_mock_enabled >= 0.5),
            "bench_mode_enabled": bool(cfg_bench_mode >= 0.5),
            "bench_no_persistence": bool(cfg_bench_no_persistence >= 0.5),
            "llm_timeout_remaining_ms_last": int(cfg_timeout_remaining_last),
        },
        "provider_unhealthy_count": provider_unhealthy_count,
    }

    avg_wait_seconds = (
        (wait_total_after - wait_total_before) / llm_wait_samples if llm_wait_samples > 0 else 0.0
    )

    if args.max_p95_ms is not None and percentile(latencies, 0.95) > args.max_p95_ms:
        summary["status"] = "failed"
        summary["reason"] = "p95_latency_exceeded"
        print(json.dumps(summary, indent=2))
        return 1

    if args.max_p99_ms is not None and percentile(latencies, 0.99) > args.max_p99_ms:
        summary["status"] = "failed"
        summary["reason"] = "p99_latency_exceeded"
        print(json.dumps(summary, indent=2))
        return 1

    rejection_rate = (rejected_after - rejected_before) / args.requests if args.requests else 0.0
    if args.max_rejection_rate is not None and rejection_rate > args.max_rejection_rate:
        summary["status"] = "failed"
        summary["reason"] = "rejection_rate_exceeded"
        print(json.dumps(summary, indent=2))
        return 1

    if args.max_avg_wait_seconds is not None and avg_wait_seconds > args.max_avg_wait_seconds:
        summary["status"] = "failed"
        summary["reason"] = "avg_wait_exceeded"
        print(json.dumps(summary, indent=2))
        return 1

    if (
        args.max_provider_unhealthy is not None
        and provider_unhealthy_count > args.max_provider_unhealthy
    ):
        summary["status"] = "failed"
        summary["reason"] = "provider_unhealthy_exceeded"
        print(json.dumps(summary, indent=2))
        return 1

    reject_signal_p95_ms = percentile(reject_signal_latencies, 0.95)
    if (
        args.max_reject_signal_p95_ms is not None
        and reject_signal_p95_ms > args.max_reject_signal_p95_ms
    ):
        summary["status"] = "failed"
        summary["reason"] = "reject_signal_budget_exceeded"
        print(json.dumps(summary, indent=2))
        return 1

    if (
        args.max_handler_reject_avg_seconds is not None
        and handler_reject_avg_seconds > args.max_handler_reject_avg_seconds
    ):
        summary["status"] = "failed"
        summary["reason"] = "handler_reject_budget_exceeded"
        print(json.dumps(summary, indent=2))
        return 1

    if ok_count == 0:
        summary["status"] = "failed"
        summary["reason"] = "no_successful_requests"
        print(json.dumps(summary, indent=2))
        return 1

    if args.enforce_connections_peak and conn_counter.get("peak", 0) < min(args.concurrency, args.requests):
        summary["status"] = "failed"
        summary["reason"] = "connection_peak_below_concurrency"
        print(json.dumps(summary, indent=2))
        return 1

    if args.require_llm_metrics and llm_wait_samples == 0:
        summary["status"] = "failed"
        summary["reason"] = "no_llm_activity_observed"
        print(json.dumps(summary, indent=2))
        return 1

    print(json.dumps(summary, indent=2))
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="LLM queue smoke load over gateway websocket")
    parser.add_argument("--ws-url", default="ws://127.0.0.1:18789/ws")
    parser.add_argument("--metrics-url", default="http://127.0.0.1:18789/metrics")
    parser.add_argument("--requests", type=int, default=40)
    parser.add_argument("--concurrency", type=int, default=8)
    parser.add_argument("--timeout", type=float, default=30.0)
    parser.add_argument("--prompt", default="Say hello in exactly one short sentence.")
    parser.add_argument("--allow-offline", action="store_true")
    parser.add_argument("--require-llm-metrics", action="store_true", default=True)
    parser.add_argument("--no-require-llm-metrics", dest="require_llm_metrics", action="store_false")
    parser.add_argument("--max-p95-ms", type=float)
    parser.add_argument("--max-p99-ms", type=float)
    parser.add_argument("--max-rejection-rate", type=float)
    parser.add_argument("--max-avg-wait-seconds", type=float)
    parser.add_argument("--max-provider-unhealthy", type=int)
    parser.add_argument("--max-reject-signal-p95-ms", type=float)
    parser.add_argument("--max-handler-reject-avg-seconds", type=float)
    parser.add_argument("--enforce-connections-peak", action="store_true", default=True)
    parser.add_argument("--no-enforce-connections-peak", dest="enforce_connections_peak", action="store_false")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    return asyncio.run(main_async(args))


if __name__ == "__main__":
    raise SystemExit(main())
