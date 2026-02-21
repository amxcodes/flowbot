# Architecture

This repository is split into two crates:

- `crates/nanobot-core`: all runtime logic (agent, gateways, tools, config, memory, security).
- `crates/nanobot-cli`: user-facing CLI entrypoint and command routing.

## High-level diagram

```text
User/Channel
    |
    v
Gateway Adapter (telegram/slack/discord/teams/google_chat/web)
    |
    v
AgentMessage (session_id, tenant_id, content)
    |
    v
AgentLoop
  |- provider selection/failover
  |- context/history loading
  |- tool execution + policy checks
  |- streaming response chunks
    |
    v
Adapter sends response back to originating channel
```

## Main building blocks

### 1) CLI layer (`nanobot-cli`)

Entry file: `crates/nanobot-cli/src/main.rs`

The CLI parses commands and starts the right subsystem. It does not own business logic; it delegates to `nanobot-core`.

### 2) Agent runtime (`nanobot-core::agent`)

Core file: `crates/nanobot-core/src/agent/mod.rs`

`AgentLoop` is the heart of runtime behavior:

- initializes provider and config
- boots cron scheduler
- boots memory manager and workspace watcher
- loads skills and personality context
- processes incoming `AgentMessage` events
- streams chunks (`TextDelta`, `Thinking`, `ToolCall`, `ToolResult`, `Done`)

### 3) Gateways (`nanobot-core::gateway`)

Main module: `crates/nanobot-core/src/gateway/mod.rs`

Gateway responsibilities:

- expose HTTP/WebSocket endpoints
- run channel adapters
- route inbound channel messages to the agent
- provide settings/skills/admin-ish API endpoints

Adapters live under `crates/nanobot-core/src/gateway/*_adapter.rs`.

### 4) Tools subsystem (`nanobot-core::tools`)

The agent can invoke structured tools (filesystem, commands, web, process control, etc.).

Key pieces:

- registry and tool contracts (`tools/definitions.rs`)
- execution path (`tools/executor.rs`)
- policy and permissions (`tools/policy.rs`, `tools/permissions.rs`)
- confirmation flow (`tools/confirmation.rs`, adapters)

### 5) Persistence and state

State is split across:

- project-local DB/files under `./.nanobot/`
- home-level data under `~/.nanobot/`

Used for context trees, session history, memory index, logs, and tokens.

### 6) Config and security

Config model: `crates/nanobot-core/src/config/mod.rs`

Important behavior:

- `InteractionPolicy` controls tool safety mode.
- provider credentials are read from `config.toml` and env.
- OAuth tokens are persisted via encrypted storage where possible.

## Why this split works

- CLI stays thin and easy to evolve.
- Core remains reusable for future frontends (web app, daemon, integrations).
- Adapters isolate channel-specific code from agent logic.
- Tool policies give one place to tune risk posture.
