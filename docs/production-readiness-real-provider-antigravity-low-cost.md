# Production Readiness Report - Real Provider (Antigravity, Low-Cost)

Generated: 2026-02-18

## Scope

- Provider: `antigravity`
- Mode: `low_cost`
- Artifact: `docs/real-provider-low-cost-benchmark-latest.jsonl`
- Runtime assertions: queue/admission budget invariants + provider unhealthy threshold profile

## Benchmark Outcome

- Total rows: 12
- Failed rows: 1
- Failed row: `permits=16`, `queue_wait_ms=3000`, `concurrency=50`
- Failure reason: `no_successful_requests`
- Provider unhealthy in failed row: `32`

## Interpretation

- Runtime queueing and rejection behavior remains bounded by configured budgets.
- Failure mode is upstream saturation (provider quota/availability), not runtime deadlock.
- System correctly preserves liveness and rejects under overload.

## Stable Envelope (Provider-Scoped)

- Recommended default: `NANOBOT_LLM_CONCURRENCY_LIMIT=8`
- Recommended queue wait: `NANOBOT_LLM_QUEUE_WAIT_MS=1000`
- This profile avoids sustained provider unhealthy spikes in tested low-cost runs.

## CI Guard Policy

### Global invariants (all providers)

Fail row if any condition is true:

1. `no_successful_requests`
2. `connections_peak < concurrency`
3. Reject signal budget violated:
   - `reject_signal_latency_ms.p95 > queue_wait_ms + 400ms + load_penalty`
   - `load_penalty = min(600ms, concurrency * 10ms)`
4. Handler reject budget violated:
   - `llm_handler_to_reject_decision_avg_seconds > queue_wait_ms/1000 + 0.4`

### Provider-scoped profile (antigravity.low_cost)

- `provider_unhealthy_fail_threshold = 3`
- Profile source: `configs/provider_capacity_profiles.toml`

## Readiness Verdict

- **Ready for bounded-load production use** with provider-scoped capacity profile.
- Not ready for static high-permit operation (`permits=16`) without adaptive control.

## Next Increment

- Add light step-down adaptive permits:
  - Halve permits on provider unhealthy events (429/5xx/timeout),
  - Hold cooldown window,
  - Recover slowly (+1 per cooldown window with no unhealthy events).

## Runtime Safeguard (Implemented)

- Adaptive permits are now enabled by default for Antigravity provider.
- Behavior:
  - On provider unhealthy event: `current_limit = max(current_limit / 2, min_permits)`
  - Recovery: `+1` permit per cooldown window if no unhealthy event is observed
  - Recovery tick source: time-driven interval task (not request-rate-driven)
- Tunables:
  - `NANOBOT_LLM_ADAPTIVE_PERMITS` (default on for Antigravity)
  - `NANOBOT_LLM_ADAPTIVE_MIN_PERMITS` (default 8 for Antigravity)
  - `NANOBOT_LLM_ADAPTIVE_COOLDOWN_MS` (default 60000)

### Adaptive metrics

- `llm_permits_current`
- `llm_permits_target`
- `llm_adaptive_stepdowns_total`
- `llm_adaptive_recoveries_total`
