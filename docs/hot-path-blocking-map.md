# Hot-Path Blocking Map

Generated: 2026-02-17

## Scope

- Gateway HTTP handlers (`crates/nanobot-core/src/gateway/mod.rs`)
- WS message handler (`crates/nanobot-core/src/gateway/mod.rs`)
- Adapter inbound handlers (`crates/nanobot-core/src/gateway/*adapter*.rs`)
- Admin endpoints (`crates/nanobot-core/src/server/admin.rs`, `crates/nanobot-cli/src/web/mod.rs`)
- Tool execution entrypoint (`crates/nanobot-core/src/tools/executor.rs`)
- Agent streaming path (`crates/nanobot-core/src/agent/mod.rs`)

## Controlled Blocking Boundaries

- `crate::blocking::fs(...)` (semaphore + `spawn_blocking`) in `crates/nanobot-core/src/blocking.rs`
- `crate::blocking::sqlite(...)` (semaphore + `spawn_blocking`) in `crates/nanobot-core/src/blocking.rs`
- `crate::blocking::process_output*` (process semaphore) in `crates/nanobot-core/src/blocking.rs`
- Agent persistence boundary (`PERSISTENCE_BLOCKING_SEMAPHORE` + `tokio::task::spawn_blocking`) in `crates/nanobot-core/src/agent/mod.rs`

## Semaphore Acquisition Points

- `LLM_TASK_SEMAPHORE.acquire()` in `process_streaming` (`crates/nanobot-core/src/agent/mod.rs`)
- `PERSISTENCE_BLOCKING_SEMAPHORE.acquire()` in persistence helpers (`crates/nanobot-core/src/agent/mod.rs`)
- FS/SQLite pool acquires in `crates/nanobot-core/src/blocking.rs`
- Process pool acquire in `crates/nanobot-core/src/blocking.rs`

## Path-by-Path Findings

### 1) Gateway HTTP handlers

Routes are centrally registered in `register_gateway_routes` (`crates/nanobot-core/src/gateway/mod.rs`).

- Sync filesystem/process/sqlite use on handler paths is wrapped by `crate::blocking`:
  - config load / command probes via async wrappers
  - webhook nonce sqlite path via `crate::blocking::sqlite(...)`
  - skill install/test filesystem operations via `crate::blocking::fs(...)`
  - skill dependency command execution via `crate::blocking::process_output_in_dir(...)`

Status: **Compliant** (gateway request path sync I/O is behind controlled blocking boundary).

### 2) WebSocket handler

Entry: `ws_handler` -> `handle_socket` in `crates/nanobot-core/src/gateway/mod.rs`.

- Per-request session token signing now uses preloaded `gateway_session_secret` from `Gateway::new(...)`.
- `handle_socket` no longer calls `get_or_create_session_secrets()` on request path.

Status: **Compliant**.

### 3) Adapter inbound handlers (telegram/slack/discord/web)

Files audited:

- `crates/nanobot-core/src/gateway/discord_adapter.rs`
- `crates/nanobot-core/src/gateway/slack_adapter.rs`
- `crates/nanobot-core/src/gateway/telegram_adapter.rs`
- `crates/nanobot-core/src/gateway/web_adapter.rs`

- No direct `std::fs`, `rusqlite::Connection::open`, `std::process::Command::new`, or `spawn_blocking` found in adapter inbound paths.

Status: **Compliant**.

### 4) Admin endpoints

- Core admin routes (`/health`, `/state`, `/tools`, `/eval`) in `crates/nanobot-core/src/server/admin.rs`
- CLI web admin endpoints in `crates/nanobot-cli/src/web/mod.rs`

- No direct sync FS/SQLite/process operations in these async handlers.
- `/eval` transitively executes tools through `execute_tool(...)`; tool-path findings apply.

Status: **Compliant** (with tool entrypoint controls below).

### 5) Tool execution entrypoint

Entry: `execute_tool(...)` in `crates/nanobot-core/src/tools/executor.rs`.

- Previously sync hot spots were removed from request path:
  - command existence checks moved to async blocking wrapper (`crate::blocking::command_exists`)
  - Node help capability probe moved to `crate::blocking::process_output`
  - npm dependency bootstrap moved to `crate::blocking::process_output_in_dir`
  - agents listing switched from `std::fs::read_dir` to `tokio::fs::read_dir`

Status: **Compliant**.

### 6) Agent streaming path

Entry: `process_streaming(...)` in `crates/nanobot-core/src/agent/mod.rs`.

- Global LLM concurrency gate is enforced at ingress (`LLM_TASK_SEMAPHORE`).
- Saturation/backpressure policy is active: queue-wait timeout rejects request and increments `llm_rejected_total{reason=semaphore_timeout}`.
- Streaming assistant persistence is buffered and flushed on token/time/end thresholds.
- Flush writes run via persistence blocking boundary helpers (`start_stream_message`, `append_stream_message_content`) and `spawn_blocking`.

Status: **Compliant for async hot path** (all sync DB/fs work in controlled blocking boundaries).

## Final Statement

For audited request paths, synchronous filesystem/SQLite/process work is no longer directly performed inline in async handlers.

- Gateway/WS/adapter/admin/tool request paths: sync operations are either absent or wrapped through controlled blocking boundaries.
- Agent streaming: sync persistence work is isolated behind semaphore + blocking task boundaries.
