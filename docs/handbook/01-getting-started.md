# Getting Started

This is the shortest reliable path from clone to a working bot.

## 1) Prerequisites

- Rust toolchain installed (`rustup`, `cargo`).
- A `config.toml` in repo root.
- At least one LLM provider configured (Antigravity, OpenAI, OpenRouter, or Google).
- For Telegram: bot token and allowed user IDs.

## 2) First-time local setup

```bash
cp .env.example .env
cp config.example.toml config.toml
```

Then edit `config.toml` with real values.

If you want a plain-English variable guide, use: [Variables Made Simple](./07-variables-made-simple.md)

Recommended baseline:

- `default_provider = "antigravity"`
- Fill `[providers.antigravity]` (OAuth or API key path)
- Add `[providers.telegram]` if you plan to run gateway on Telegram

## 3) Build

```bash
cargo build
```

## 4) Run the mode you need

Interactive terminal chat:

```bash
cargo run -p nanobot-cli -- chat --tui
```

One-shot CLI message:

```bash
cargo run -p nanobot-cli -- agent -m "hello"
```

Messaging gateway (Telegram/Slack/Discord/Teams/Google Chat based on config):

```bash
cargo run -p nanobot-cli -- gateway
```

WebSocket/API server:

```bash
cargo run -p nanobot-cli -- server --port 18789
```

WebChat UI:

```bash
cargo run -p nanobot-cli -- webchat --port 8080
```

## 5) Verify quickly

- `cargo run -p nanobot-cli -- doctor`
- `cargo run -p nanobot-cli -- gateway --channel telegram` (if using Telegram)
- `cargo run -p nanobot-cli -- memory status`

## Common confusion points

- `config.toml` is required by core startup; missing file causes immediate failure.
- Some runtime state is project-local (`./.nanobot/*`), and some is in home directory (`~/.nanobot/*`).
- If a provider is configured but not actually reachable (bad key/token), startup can pass but requests will fail later.

## Where to go next

- System shape: [Architecture](./02-architecture.md)
- Full capabilities: [Feature Map](./03-feature-map.md)
- Runtime flow: [Request Lifecycle](./04-request-lifecycle.md)
- Non-technical config guide: [Variables Made Simple](./07-variables-made-simple.md)
