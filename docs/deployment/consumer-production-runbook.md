# Consumer Production Runbook

This runbook is for operating the websocket gateway in consumer-facing production.

## 1) Topology Profiles

### Single replica (minimal)

- `NANOBOT_REPLICA_COUNT=1`
- `NANOBOT_SCALING_MODE=sticky` (optional but recommended for consistency)
- Redis optional for startup, but recommended for production parity.

### Multi-replica sticky (required for horizontal scale)

- `NANOBOT_REPLICA_COUNT=<n, n>1`
- `NANOBOT_SCALING_MODE=sticky`
- `NANOBOT_STICKY_SIGNAL_HEADER=<lb affinity header>`
- `NANOBOT_DISTRIBUTED_STORE_BACKEND=redis`
- `NANOBOT_REDIS_URL=<redis endpoint>`
- `NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT=<qps budget>`
- `NANOBOT_PROVIDER_LIMITER_BACKEND=redis`
- `NANOBOT_PROVIDER_LIMITER_FAILURE_MODE=closed`

## 2) Multi-Replica Startup Hard-Fail Matrix

Startup fails with a fatal checklist if any requirement below is missing:

- sticky scaling mode
- sticky signal header configured
- provider limiter enabled (`qps>0`)
- limiter backend is redis
- limiter failure mode is closed
- distributed store backend is redis
- binary compiled with `distributed-redis`
- redis URL configured
- redis reachability check succeeds within startup timeout

Redis startup controls:

- `NANOBOT_REDIS_STARTUP_TIMEOUT_MS` (default `300`)
- `NANOBOT_REDIS_STARTUP_RETRY_COUNT` (`0` or `1`, default `0`)

## 3) Health and Degradation Signals

- Sticky drift health degradation:
  - missing sticky signal increments internal violation counters
  - conflicting sticky signal increments conflict counters
  - `/health` returns `503` during active degradation window
- Degradation auto-recovers after violation window expires if drift stops.

Operational meaning:

- `503` + sticky violation: ingress affinity is broken; fix load balancer/session policy before scaling traffic.
- limiter/redis errors in multi-replica: requests are shed (fail-closed) by design.

## 4) Redis-Down Behavior Matrix

- Provider limiter:
  - `closed`: deny on backend failure (required for multi-replica)
  - `open`: allow on backend failure (not allowed in multi-replica startup checks)
- Terminal dedupe:
  - multi-replica: fail-closed on redis errors (no permissive duplicate terminal path)
  - single-replica: may run permissive paths where configured
- Inflight tracking:
  - atomic admission via store path; backend failure rejects request before enqueue

## 5) Incident Checklist (Copy/Paste)

### A) Startup fails with multi-replica fatal checklist

1. Ensure binary includes redis feature build.
2. Set required sticky/redis limiter env vars from `configs/multi-replica-sticky-redis.env`.
3. Validate Redis connectivity from host.
4. Restart gateway and confirm no fatal checklist failures.

### B) `/health` returns 503 under sticky mode

1. Confirm LB injects `NANOBOT_STICKY_SIGNAL_HEADER` on websocket/session traffic.
2. Confirm no route/path bypasses affinity policy.
3. Wait one degradation window and verify health recovery.

### C) Redis outage in multi-replica

Expected behavior:

- request shedding (fail-closed)
- no duplicate terminal emission

Actions:

1. Restore Redis availability.
2. Check limiter and dedupe error metrics drop to baseline.
3. Confirm request success/error accounting returns to expected range.

### D) Startup fails with persistence schema error

Typical error:

- `persistence schema invalid: missing required column 'messages.<...>'`

Actions:

1. Stop rollout and keep previous binary serving traffic.
2. Back up the sqlite DB file.
3. Run the new binary in maintenance mode once to let schema init/migration apply.
4. Re-run startup and confirm schema validation passes.
5. If still failing, restore DB backup and roll back binary.

## 6) Rollback Playbook

If new release causes instability:

1. Keep Redis and ingress unchanged.
2. Roll back application binary only.
3. Verify startup checklist passes.
4. Verify persistence schema validation passes (no `persistence schema invalid` errors).
5. Verify micro-smoke CI artifacts for:
   - rolling restart
   - persistence rolling restart
6. Re-introduce traffic gradually.

## 7) Pre-Release Gate

Do not release if any of these fail in CI:

- gateway terminal contract guards
- atomic inflight admission guard
- multi-replica startup guard
- redis integration + chaos checks
- rolling restart micro-smoke
- rolling restart persistence micro-smoke
