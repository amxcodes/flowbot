# Production Gates

## Gate 1: Route Policy Enforcement

- Gateway routes must be registered through the policy-aware helper in `crates/nanobot-core/src/gateway/mod.rs`.
- CI guard: `scripts/check_route_policy.sh` fails if direct `.route(...)` calls appear in gateway production routing code.

## Gate 2: Tool Capability Enforcement

- Side-effectful tool primitives (filesystem/network/process/persistence) must only be called from capability-wrapper functions in `crates/nanobot-core/src/tools/executor.rs`.
- CI guard: `scripts/check_side_effect_token_gating.py` verifies single-call wrapper ownership of these primitives.

## Gate 3: Failure-Mode Security Tests

- Webhook signature/replay/timestamp/tamper tests.
- Nonce store failure must fail closed.
- Production metrics auth requirement test.
- Production insecure WS and antigravity hardcoded-path refusal tests.

## Gate 4: Load Smoke Validation (On-demand)

- Script: `scripts/load_smoke_gateway.sh`
- Purpose: quick concurrency sanity check for ingress endpoints before deployment.
