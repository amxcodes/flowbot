# LLM Queue Load Smoke

Generated: 2026-02-17

## Purpose

Validate runtime behavior for:

- LLM semaphore wait distribution (`llm_task_semaphore_wait_seconds`)
- Rejection policy under saturation (`llm_rejected_total{reason=semaphore_timeout}`)
- End-to-end websocket request latency under concurrent load

Important: benchmarks are meaningful only with valid provider credentials. Runs with placeholder/invalid keys are failure-mode tests, not capacity measurements.

## Command

Run with default target:

```bash
bash scripts/load_smoke_llm_queue.sh
```

Matrix run (10/50/200 concurrency):

```bash
bash scripts/load_smoke_llm_queue_matrix.sh
```

Single level run (for targeted replay):

```bash
python scripts/load_smoke_llm_queue.py --concurrency 200 --requests 400 --max-rejection-rate 0.35
```

Override knobs:

```bash
WS_URL="ws://127.0.0.1:18789/ws" \
METRICS_URL="http://127.0.0.1:18789/metrics" \
REQUESTS=80 \
CONCURRENCY=16 \
TIMEOUT=45 \
PROMPT="Summarize this in one sentence: hello" \
bash scripts/load_smoke_llm_queue.sh
```

## Output format

The script prints JSON including:

- `latency_ms.p50/p95/p99/avg`
- `metrics_delta.llm_wait_total_seconds`
- `metrics_delta.llm_wait_samples`
- `metrics_delta.llm_service_total_seconds`
- `metrics_delta.llm_service_samples`
- `metrics_delta.llm_service_avg_seconds`
- `metrics_delta.llm_rejected`
- `ok`, `busy`, `errors`

## CI behavior

- In CI (`CI=1`), if metrics are unreachable the script returns **failed** with reason `metrics_unreachable`.
- Local runs may report `skipped` for unreachable metrics.
- Set `ALLOW_OFFLINE=1` to allow non-failing offline CI runs when explicitly intended.

## Acceptance thresholds

Run levels: 10 / 50 / 200 concurrency.

- At concurrency 50:
  - websocket latency `p95 < 2000 ms`
  - websocket latency `p99 < 5000 ms`
  - rejection rate (`busy / requests`) `< 1%`
  - average semaphore wait (`llm_task_semaphore_wait_seconds_duration_seconds / llm_task_semaphore_wait_seconds_total`) `< 1.0 s`
- At concurrency 200:
  - server remains responsive (`/health` and `/metrics` reachable during/after run)
  - rejection rate may increase, but must remain bounded (`< 35%` initial ceiling) and not monotonic-runaway across repeated runs

Note: the current metrics collector exports totals/counts for wait duration, not a percentile histogram, so semaphore wait is currently gated by mean wait. Upgrading to histogram output is a follow-up for strict p95 semaphore gating.

The matrix script enforces these thresholds:

- `scripts/load_smoke_llm_queue_matrix.sh` applies gates at concurrency 50 and 200 and exits non-zero on failure.

## Latest run in this workspace

- Config used: `scripts/load-smoke-config.toml`
- Matrix output artifact: `docs/load-smoke-llm-queue-latest.jsonl`
- Concurrency 10 result: `ok` (`p95=24497.57ms`, `p99=24510.02ms`, wait avg `0.000004s`, rejection `0%`)
- Concurrency 50 result: `failed` (`reason=p95_latency_exceeded`, `p95=25132.49ms`, `p99=25143.54ms`, wait avg `3.4067s`, rejection `68%`)
- Concurrency 200 targeted result: `failed` (`reason=rejection_rate_exceeded`, see `docs/load-smoke-llm-queue-c200.json`, rejection `84%`)
- Post-stress responsiveness check: `/health` and `/metrics` both returned `200`

Conclusion from current run: **thresholds are not met at concurrency 50/200**. Rejection policy works and stays bounded, but latency and rejection rates exceed readiness gates.
