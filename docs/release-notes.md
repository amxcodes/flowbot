# Release Notes

## 2026-02-19 - Runtime hardening phase closure

- Enforced strict terminal event contract with typed `success_done` / `error_done` / `cancelled_done` semantics and duplicate-terminal protection.
- Added symmetric enqueue-failure terminal coverage for websocket compat and non-compat paths.
- Hardened admission with FIFO fairness, bounded queue depth, and fail-fast over-capacity rejection.
- Added per-session in-flight request cap to prevent request-correlation ambiguity.
- Standardized provider capability behavior and explicit unsupported tool-call failure paths.
- Added websocket timed-send enforcement and slow-client eviction protections.
- Implemented Prometheus format compliance plus runtime/CI label-cardinality policy enforcement.
- Completed token-gated side-effect isolation and compile-fail capability boundary checks.
- Removed shadow websocket adapter to avoid alternate unguarded lifecycle paths.
