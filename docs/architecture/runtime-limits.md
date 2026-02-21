# Runtime Limits and Deployment Boundaries

This runtime is production-hardened for a single process instance.
It is not a distributed session coordinator.

## Single-instance scope

- Session routing and lifecycle are process-local.
- Active request state, pending interactive questions, and socket task ownership are in-memory.
- A process restart drops volatile in-flight state.

## Horizontal scaling status

- Horizontal scaling is not guaranteed for conversational/session continuity.
- Multiple replicas behind a load balancer can split one logical user session across nodes.
- There is no shared distributed session store, no cross-node pending question registry, and no global in-flight request map.
- Admission mode is local by default (`NANOBOT_ADMISSION_MODE=local`).
- `NANOBOT_ADMISSION_MODE=global` is reserved for future distributed admission; strict mode (`NANOBOT_ADMISSION_STRICT=1`) will refuse startup until implemented.
- Scaling mode is sticky by default (`NANOBOT_SCALING_MODE=sticky`).
- `NANOBOT_SCALING_MODE=stateless` is reserved for future bus-based cross-node streaming and currently falls back to sticky mode.
- In production, sticky mode should declare an affinity signal (`NANOBOT_STICKY_SIGNAL_HEADER`); strict mode (`NANOBOT_SCALING_STRICT=1`) enforces this at startup.
- In sticky mode with multiple replicas (`NANOBOT_REPLICA_COUNT>1`), configure a global provider limiter (`NANOBOT_GLOBAL_PROVIDER_QPS_LIMIT`) to avoid multiplied upstream burst pressure.
- In strict multi-replica sticky mode, runtime sticky drift is fail-fast: missing/conflicting sticky signal observations trigger counters (`distributed_sticky_signal_missing_total`, `distributed_sticky_signal_conflict_total`) and temporarily degrade health for `NANOBOT_STICKY_VIOLATION_GRACE_MS` (default `60000`).

## Multi-tenant isolation model

- Tenant separation is logical (session/tenant ids), not a distributed hard boundary.
- Isolation guarantees apply within one runtime process.
- For strict tenancy boundaries, deploy one runtime per tenant or add external tenancy controls.

## Request correlation model

- Request-level correlation is `request_id` scoped to events emitted by one runtime instance.
- Terminal events (`success_done`, `error_done`, `cancelled_done`) are guaranteed per accepted request in-process.
- Cross-node correlation is not provided.

## Safe deployment patterns

- Preferred: single active runtime per bot/gateway deployment.
- If running multiple replicas for availability, use sticky routing at ingress and treat session continuity as best effort.
- Do not assume cross-node pending question recovery.

## Operational consequences

- Draining/restart should account for in-flight request loss risk.
- Alerts and SLOs must use per-instance metrics aggregation, not per-session global assumptions.
- Incident playbooks should include session-reset expectations after failover.
