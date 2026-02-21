# Mock Provider Tuning Sweep

Generated: 2026-02-17

## Goal

Tune queue behavior under deterministic mock provider load by sweeping:

- `NANOBOT_LLM_CONCURRENCY_LIMIT` (permits)
- `NANOBOT_LLM_QUEUE_WAIT_MS` (fast-reject threshold)

while holding provider behavior fixed (`500ms`, `5 chunks`).

## Command

```bash
PERMITS_LIST='8 16' \
QUEUE_WAIT_LIST='1000 5000' \
CONCURRENCY_LIST='50 200' \
OUT_FILE='docs/mock-provider-tuning-focused.jsonl' \
bash scripts/benchmark_mock_tuning.sh
```

## Artifact

- `docs/mock-provider-tuning-focused.jsonl`

## Key results

### Concurrency 50

- `permits=8, queue_wait=1000ms`: p95 `3557ms`, rejection `68%`, in-service peak `8`
- `permits=8, queue_wait=5000ms`: p95 `3925ms`, rejection `0%`, in-service peak `8`
- `permits=16, queue_wait=1000ms`: p95 `4601ms`, rejection `36%`, in-service peak `16`
- `permits=16, queue_wait=5000ms`: p95 `4129ms`, rejection `0%`, in-service peak `16`

### Concurrency 200

- `permits=8, queue_wait=1000ms`: rejection `90.25%`
- `permits=8, queue_wait=5000ms`: rejection `61.25%`
- `permits=16, queue_wait=1000ms`: rejection `86%`
- `permits=16, queue_wait=5000ms`: rejection `57.25%`

## What this confirms

- Split gauges work as intended:
  - handlers peak follows offered concurrency (50/200)
  - in-service peak follows permits (8/16)
- Queue pressure is primarily at semaphore admission (not dispatch serialization).
- Raising queue-wait timeout trades fewer rejects for higher tail latency.
- Raising permits from 8 to 16 increases in-service concurrency, but does not improve p95 enough under this websocket-heavy benchmark, indicating additional overhead outside provider service time.

Focused instrumentation snapshot (`docs/mock-active-handler-check.json`) adds separate success/reject latency and websocket-send wait:

- `success_latency_ms.p95`: ~`4224ms`
- `reject_latency_ms.p95`: ~`3756ms`
- `ws_send_wait_avg_seconds`: ~`0.00001s` with `ws_send_inflight_peak=7`

This indicates websocket channel send backpressure is negligible in this run; dominant latency remains queue/service admission dynamics before completion.

## Recommended default posture (current)

- Prefer fail-fast semantics for user experience predictability:
  - `NANOBOT_LLM_CONCURRENCY_LIMIT=8`
  - `NANOBOT_LLM_QUEUE_WAIT_MS=1000..2000`
- Keep explicit overload signaling (`429/503-style` message + `llm_rejected_total` reason tags).
- Treat high-concurrency targets (`>=200`) as scale-out territory, not single-instance tuning.

## Next tuning target

- Use success vs reject latency split to pick operational defaults:
  - If user experience favors quick feedback, keep queue wait short (higher reject rate, lower waiting).
  - If user experience favors eventual completion, raise queue wait (lower reject rate, higher p95).
- Evaluate per-session/per-tenant fairness limits to prevent queue monopolization under bursty load.

## Reject trace summary helper

Generate a compact p50/p95 segment table from sampled reject traces:

```bash
python scripts/summarize_reject_timings.py --log "scripts/load-smoke-server.log" --out "docs/reject-timing-summary.txt"
```

Latest output artifact:

- `docs/reject-timing-summary.txt`

Current sampled summary (150 events):

- `recv_to_handler_start_ms`: p95 `3ms`
- `handler_start_to_deadline_expired_ms`: p95 `1276.85ms`
- `deadline_expired_to_reject_emit_ms`: p95 `0ms`
- `reject_emit_to_ws_send_complete_ms`: p95 `0ms`
- `total_recv_to_ws_send_ms`: p95 `1276.85ms`
