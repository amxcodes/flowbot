# Sticky Scaling Deployment Checklist

Use this checklist when deploying multiple runtime replicas in sticky mode.

## Required mode and replica flags

- `NANOBOT_SCALING_MODE=sticky`
- `NANOBOT_REPLICA_COUNT=<actual replica count>`

Note: for `NANOBOT_REPLICA_COUNT>1`, startup now hard-fails if sticky/redis prerequisites are not met (independent of strict flags).

## Sticky routing signal

- Set `NANOBOT_STICKY_SIGNAL_HEADER` to the ingress affinity header your LB injects.
- Ensure websocket and related HTTP/webhook session traffic use the same affinity policy.

## Global provider limiter (multi-replica)

For `NANOBOT_REPLICA_COUNT>1`:

- `NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT=<qps budget>`
- `NANOBOT_PROVIDER_LIMITER_BACKEND=redis`
- `NANOBOT_PROVIDER_LIMITER_FAILURE_MODE=closed`
- `NANOBOT_REDIS_URL=<redis endpoint>`
- Build/runtime includes `distributed-redis` feature when using redis-backed limiter/stores.

## Distributed store backend and dedupe

- `NANOBOT_DISTRIBUTED_STORE_BACKEND=redis` (required for multi-replica sticky)
- Optional:
  - `NANOBOT_REDIS_KEY_PREFIX=<namespace>`
  - `NANOBOT_PENDING_QUESTION_TTL_SECS=<seconds>`
  - `NANOBOT_TERMINAL_DEDUPE_TTL_SECS=<seconds>`

## Validate before go-live

- Startup logs show requested and effective scaling mode.
- Startup logs show distributed backend selection.
- No strict-mode startup guard failures.
- No multi-replica startup readiness failures (`redis reachability` check must pass).
- Metrics scrape includes:
  - `provider_global_limiter_checks_total`
  - `provider_global_limiter_errors_total`
  - `distributed_scaling_mode_guard_warnings_total`
  - `distributed_store_backend_fallback_total`
