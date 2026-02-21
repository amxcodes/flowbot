# Mock Provider Benchmark

Generated: 2026-02-17

## Purpose

Measure runtime overhead independent of upstream provider/network by using deterministic local mock streaming.

## Mode

- `NANOBOT_MOCK_PROVIDER=1`
- `NANOBOT_MOCK_SERVICE_MS=500`
- `NANOBOT_MOCK_CHUNKS=5`
- `NANOBOT_LLM_BENCH_MODE=1`
- `NANOBOT_LLM_BENCH_NO_PERSISTENCE=1`

## Command

```bash
bash scripts/benchmark_mock_provider_matrix.sh
```

Output artifact:

- `docs/mock-provider-benchmark-latest.jsonl`

## Captured matrix

Permits: `4 / 8 / 16`

Concurrency: `10 / 50 / 200`

## Key observations from latest run

- `llm_service_avg_seconds` is stable around `~0.55s` at low/moderate load, matching expected mock service profile.
- New split-gauge data confirms queue location: at concurrency 50 with permits 8, `llm_active_handlers_peak=50` while `llm_in_service_peak=8` (`queue_depth_peak_estimate=42`).
- At concurrency `50`, p95 is around `~3.9s` with `0%` rejection across permits.
- At concurrency `200`, rejection rises to `~61%` and p95 moves to `~6.7-8.0s`.
- Changing permits `4 -> 16` did not materially improve p95/rejection in this run, indicating a likely bottleneck above provider execution (request dispatch/worker parallelism path), not upstream LLM time.

## Interpretation

This run distinguishes failure modes:

- Upstream dependency is not the main limiter in mock mode.
- Internal runtime/request-path concurrency is likely the dominant bottleneck under high in-flight websocket load.

## Next tuning targets

- Tune queue policy with measured split:
  - `queue_wait = handler_enter -> permit_acquire`
  - `service_time = permit_acquire -> completion`
- Tune `NANOBOT_LLM_CONCURRENCY_LIMIT` and `NANOBOT_LLM_QUEUE_WAIT_MS` together against target p95/rejection SLO.
- Instrument websocket send/backpressure waits as next likely secondary contributor if p95 remains high after permit/timeout tuning.
