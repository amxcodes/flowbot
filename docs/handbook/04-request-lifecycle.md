# How Everything Connects (Request Lifecycle)

This is the end-to-end path for a normal message.

## 1) Message enters through a channel

Example channels: Telegram, Slack, Discord, Teams, Google Chat, WebSocket.

Each channel adapter converts platform payloads into a common internal message shape.

## 2) Adapter sends `AgentMessage` to the agent loop

`AgentMessage` carries:

- `session_id`
- `tenant_id`
- user content text
- response stream sender

This lets the same core logic serve all channels.

## 3) AgentLoop loads runtime context

Before generating output, the loop:

- reads existing conversation history
- applies context limits/compaction
- fetches memory snippets when available
- loads personality/system prompt context

## 4) Provider is selected

Provider path depends on config:

- standard provider (Antigravity/OpenAI/OpenRouter/Google)
- or meta failover chain if LLM failover config is enabled

If keys rotate or a provider fails, logic can switch key/provider according to configured behavior.

## 5) Tool calls are handled (when needed)

If model output requests tools:

- tool dispatcher validates tool access policy
- argument guards and permission checks run
- confirmation may be requested (interactive policy)
- tool executes and result is fed back into the turn

## 6) Streaming response goes back to the adapter

Output is emitted as chunks (`TextDelta`, etc.) and adapter formats it for the channel.

Session data is persisted so future turns keep continuity.

## 7) Background systems continue in parallel

While messages are handled, these keep running:

- cron scheduler
- workspace watcher and memory indexing
- health/observability tasks
- channel registry and session manager

## Special paths

### Pairing flow

For locked channels (like Telegram allowlists), users can request pairing and an operator approves/rejects via CLI (`pairing` commands).

### Admin/API flow

Gateway HTTP endpoints provide settings and skills operations. Protected endpoints require admin credentials/token.

### Scheduled agent turns

Cron can trigger system events or isolated agent turns and optionally inject result summaries back into an active session.
