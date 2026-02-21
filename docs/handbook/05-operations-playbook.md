# Operations Playbook

This is for day-2 operations: running, checking, and fixing.

## Startup profiles

Local developer loop:

```bash
cargo run -p nanobot-cli -- chat --tui
```

Gateway mode (channels):

```bash
cargo run -p nanobot-cli -- gateway
```

API server mode:

```bash
cargo run -p nanobot-cli -- server --port 18789
```

WebChat mode:

```bash
cargo run -p nanobot-cli -- webchat --port 8080
```

## First checks when something looks wrong

1. `cargo run -p nanobot-cli -- doctor`
2. confirm `config.toml` exists and provider creds are non-empty
3. verify channel-specific tokens/webhooks
4. run only the channel you are debugging (`gateway --channel telegram`, etc.)

## Core operational commands

Health/wiring:

```bash
cargo run -p nanobot-cli -- doctor
cargo run -p nanobot-cli -- doctor --wiring
```

Memory:

```bash
cargo run -p nanobot-cli -- memory status
cargo run -p nanobot-cli -- memory reindex
cargo run -p nanobot-cli -- memory clean --force
```

Cron:

```bash
cargo run -p nanobot-cli -- cron list
cargo run -p nanobot-cli -- cron status
cargo run -p nanobot-cli -- cron runs <job-id>
```

Pairing moderation:

```bash
cargo run -p nanobot-cli -- pairing list telegram
cargo run -p nanobot-cli -- pairing approve telegram <code>
cargo run -p nanobot-cli -- pairing reject telegram <code>
```

Service lifecycle:

```bash
cargo run -p nanobot-cli -- service status
cargo run -p nanobot-cli -- service restart
```

Full uninstall (service + local data + installed binary):

```bash
cargo run -p nanobot-cli -- uninstall --yes
```

## Expected state locations

- Project local runtime state: `./.nanobot/`
- User-level runtime state: `~/.nanobot/`
- Workspace-driven content: `workspace/` and `skills/`

If state appears stale, check both project-local and home-level folders.

## Change safety notes

- Keep `InteractionPolicy` at `Interactive` unless you intentionally want headless behavior.
- Use `HeadlessAllowLog` only when you explicitly accept autonomous tool execution.
- Treat admin token and provider secrets as sensitive; never commit them.

## When debugging a production incident

Use this order:

1. Validate process is up (service status / process health).
2. Validate provider auth (token/key still valid).
3. Validate channel auth (token/webhook/app_id).
4. Validate policy gates (tool denial or confirmation pending).
5. Validate persistence corruption or disk permission issues.
