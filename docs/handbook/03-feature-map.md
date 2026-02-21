# Feature Map

This file answers: what features exist today, where they live, and what they depend on.

## Command surface (from `nanobot-cli`)

Primary command definitions are in `crates/nanobot-cli/src/main.rs`.

- `setup`: bootstrap config, optional wizard, optional service install.
- `workspace`: inspect/edit/reset workspace assets.
- `doctor`: environment and wiring checks.
- `config`: update runtime config values.
- `clawhub`: list/search/install/configure skills.
- `chat`: interactive TUI chat.
- `agent`: one-shot message mode.
- `login`: OAuth login flow.
- `gateway`: channel gateways runtime.
- `pairing`: approve/reject pending channel pairing requests.
- `cron`: scheduled jobs management.
- `server`: API/WebSocket server.
- `security`: security audits.
- `service`: install/start/stop/restart daemon.
- `uninstall`: full local uninstall (service + local state + binary).
- `memory`: status/clean/reindex memory store.
- `run`: run an agent from manifest.
- `admin`, `admin-token`, `console`: admin API and management.
- `dev`: auto-rebuild development mode.
- `webchat`: web chat frontend server.

## Channel features

Adapters under `crates/nanobot-core/src/gateway/`:

- Telegram (`telegram_adapter.rs`)
- Slack (`slack_adapter.rs`)
- Discord (`discord_adapter.rs`)
- Teams (`teams_adapter.rs`)
- Google Chat (`google_chat_adapter.rs`)
- Web channel (`web_adapter.rs` + gateway WS)

Related support:

- channel registry (`registry.rs`)
- channel router (`router.rs`)
- onboarding and pairing (`onboarding.rs`, `pairing/*`)

## LLM/provider features

Provider integration happens in `agent/mod.rs` and provider modules:

- Antigravity (`antigravity.rs`)
- Google (`google.rs`)
- OpenAI/OpenRouter via `rig` OpenAI-compatible client
- Meta provider failover (`llm/meta_provider.rs`, `llm/config.rs`)

Capabilities:

- default provider selection
- API key rotation
- provider failover chain

## Tooling features

Tool modules are in `crates/nanobot-core/src/tools/`.

Current capabilities include:

- file read/write/edit/list
- command execution
- web fetch/search
- process spawn/read/write/kill
- batch execution helpers
- policy gates + permission checks
- channel and CLI confirmation workflows

## Skills and extensibility

Skills system modules:

- skill loading and scanning (`skills/*`)
- config and enable/disable behavior
- gateway endpoints for listing/installing/configuring/testing skills

The project also has MCP plumbing (`mcp/*`) for external tool discovery.

## Memory and context

Core modules:

- conversation context tree (`context/*`)
- vector memory and embeddings (`memory/*`)
- workspace watcher for reindex triggers (`memory/watcher.rs`)
- persistence and sessions (`persistence.rs`, `sessions.rs`)

## Scheduling and background automation

Cron modules:

- scheduler and job model (`cron/*`)
- isolated agent execution for scheduled turns (`cron/isolated_agent.rs`)
- run logs (`cron/run_log.rs`)

## Security and operational controls

Security/ops modules:

- interaction policy in config (`config/mod.rs`)
- proactive security (`proactive_security.rs`)
- security audit command path (`security/*`)
- service manager (`service/*`)
- observability/logging/metrics (`observability.rs`, `logging.rs`, `metrics.rs`)
