# Real Provider Low-Cost Benchmark

Generated: 2026-02-17

## Goal

Run the same runtime instrumentation with a real upstream model (low cost settings) to separate upstream latency from internal queue/dispatch overhead.

## Requirements

- Default provider is `antigravity` (OAuth token must already be available to the runtime).
- For OpenAI fallback benchmarking, set `PROVIDER=openai` and `OPENAI_API_KEY`.
- By default, benchmark fails on the first failed sweep row (`STRICT=1`).

## Config

- Config file: `scripts/load-smoke-real-config.toml`
- Provider: `antigravity` (default)
- Bench flags: `NANOBOT_LLM_BENCH_MODE=1`, `NANOBOT_LLM_BENCH_NO_PERSISTENCE=1`
- Capacity profile: `configs/provider_capacity_profiles.toml` (`providers.<provider>.low_cost`)

## Runtime/CI Assertions

`scripts/benchmark_real_provider_low_cost.sh` enforces:

- Global invariants (all providers):
  - fail on `no_successful_requests`
  - fail when `connections_peak < concurrency`
  - fail when `reject_signal_latency_ms.p95 > queue_wait_ms + 400ms + load_penalty`
    - `load_penalty = min(600ms, concurrency * 10ms)`
  - fail when `llm_handler_to_reject_decision_avg_seconds > queue_wait_ms/1000 + 0.4`
- Provider-scoped threshold:
  - fail when `provider_unhealthy` exceeds profile threshold

## Antigravity Runtime Defaults

- When `NANOBOT_PROVIDER=antigravity` and env overrides are not set:
  - `NANOBOT_LLM_CONCURRENCY_LIMIT` defaults to `8`
  - `NANOBOT_LLM_QUEUE_WAIT_MS` defaults to `1000`

## Command

```bash
bash scripts/benchmark_real_provider_low_cost.sh
```

OpenAI mode (optional):

```bash
PROVIDER=openai bash scripts/benchmark_real_provider_low_cost.sh
```

Allow non-strict completion (collect all rows even with failures):

```bash
STRICT=0 bash scripts/benchmark_real_provider_low_cost.sh
```

## Output artifact

- `docs/real-provider-low-cost-benchmark-latest.jsonl`
