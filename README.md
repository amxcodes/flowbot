# Nanobot-rs: Production-Ready AI Telegram Bot

A high-performance Rust-based AI assistant with Telegram integration, Antigravity (Gemini) support, and VPS-ready deployment.

## ✨ Features

- ✅ **Telegram Bot** - 24/7 AI assistant via Telegram
- ✅ **Antigravity (Gemini)** - Google's latest models via OAuth or API key  
- ✅ **VPS Ready** - One-command deployment with systemd
- ✅ **TUI Mode** - Interactive terminal chat  
- ✅ **Multi-Provider** - Antigravity, OpenAI, OpenRouter
- ✅ **Lightweight** - Single Rust binary (~15MB)

---

## 🚀 Quick Start (VPS Deployment)

### Automated Installation

```bash
# One command to rule them all (Ubuntu/Debian/CentOS)
curl -sSL https://raw.githubusercontent.com/yourusername/nanobot-rs/main/install.sh | sudo bash

# Configure
sudo nano /opt/nanobot/.env  # Add TELEGRAM_BOT_TOKEN

# Login to Antigravity
sudo -u nanobot /opt/nanobot/nanobot-rs login antigravity

# Start
sudo systemctl start nanobot
sudo systemctl enable nanobot
```

**Done!** Your bot is live 24/7.

📚 **Full guide**: [VPS deployment guide](docs/guides/VPS_DEPLOYMENT.md)

---

## 💻 Local Development

### 1. Clone & Build

```bash
git clone https://github.com/yourusername/nanobot-rs-clean.git
cd nanobot-rs-clean
cargo build --release
```

### 2. Configure

```bash
# Copy environment template
cp .env.example .env

# Edit configuration
nano .env
# Add:
#   TELEGRAM_BOT_TOKEN=your_token_here
#   TELEGRAM_ALLOWED_USERS=your_user_id
```

### 3. Login to Antigravity

```bash
cargo run --release -- login antigravity
# Follow OAuth flow in browser
```

### 4. Run

```bash
# Telegram gateway
cargo run --release -- gateway

# Or use quick-start
chmod +x quickstart.sh
./quickstart.sh
```

---

## 📱 Usage Modes

### Telegram Bot (Production)
```bash
cargo run -- gateway
```
24/7 AI assistant accessible from Telegram.

### Interactive TUI (Development)
```bash
cargo run -- chat -p antigravity
```
Terminal-based chat interface.

### CLI Agent (One-off)
```bash
cargo run -- agent -m "Explain quantum computing" -p antigravity
```
Single message mode.

---

## ⚙️ Configuration

### Telegram Bot Setup

1. Create bot via [@BotFather](https://t.me/botfather)
2. Get your user ID from [@userinfobot](https://t.me/userinfobot)
3. Add to `.env`:
   ```bash
   TELEGRAM_BOT_TOKEN=123456:ABC-DEF...
   TELEGRAM_ALLOWED_USERS=your_user_id
   ```

📚 **Full guide**: [Telegram setup guide](docs/guides/TELEGRAM_SETUP.md)

### Antigravity (Gemini) Setup

**Option 1: OAuth (Recommended)**
```bash
cargo run -- login antigravity
# Tokens saved to ~/.antigravity/tokens.json
```

**Option 2: API Key**
```bash
# Get from: https://makersuite.google.com/app/apikey
# Add to .env:
ANTIGRAVITY_API_KEY=AIza...
```

---

## 📋 Commands Reference

```bash
# Telegram bot
cargo run -- gateway

# OAuth login
cargo run -- login antigravity

# TUI chat
cargo run -- chat -p antigravity

# Single message
cargo run -- agent -m "Hello" -p antigravity

# Interactive setup
cargo run -- setup
```

---

## 🏗️ Architecture

```
src/
├── main.rs          # CLI entry point
├── telegram.rs      # Telegram bot integration
├── antigravity.rs   # Antigravity provider
├── tui.rs           # Terminal UI
├── oauth.rs         # OAuth flows
├── config.rs        # Configuration
└── tools/           # Agent tools
```

**Dependencies:**
- `teloxide` - Telegram bot framework
- `rig-core` - AI agent framework
- `tokio` - Async runtime
- `ratatui` - Terminal UI

---

## 🐳 Deployment Options

### VPS (Recommended)
- Systemd service
- Auto-restart
- Log management
- See [VPS deployment guide](docs/guides/VPS_DEPLOYMENT.md)

### Docker (Coming Soon)
```bash
docker run -d \
  -e TELEGRAM_BOT_TOKEN=your_token \
  -e TELEGRAM_ALLOWED_USERS=123 \
  nanobot-rs:latest
```

### Binary Release (Coming Soon)
```bash
# Pre-built binaries for Linux/macOS/Windows
curl -sSL https://get.nanobot.ai | bash
```

---

## 🔧 Service Management (VPS)

```bash
# Start/stop
sudo systemctl start nanobot
sudo systemctl stop nanobot

# Logs
sudo journalctl -u nanobot -f

# Status
sudo systemctl status nanobot
```

---

## 📚 Documentation

- [VPS Deployment Guide](docs/guides/VPS_DEPLOYMENT.md) - Production setup
- [Telegram Setup](docs/guides/TELEGRAM_SETUP.md) - Bot configuration
- [Telegram Testing](docs/guides/TELEGRAM_TESTING.md) - Usage & examples
- [Provider Setup](docs/guides/PROVIDER_SETUP.md) - OAuth & API keys
- [Project Handbook](docs/handbook/README.md) - Architecture, feature map, and ops playbook
- [24x7 Bootstrap Setup](docs/handbook/06-bootstrap-24x7.md) - Non-interactive install and service runbook
- [Parity Gates](docs/parity-gates.md) - OpenClaw parity scoring and thresholds
- [Runtime Limits](docs/architecture/runtime-limits.md) - Single-instance boundaries and scaling constraints
- [Hardening Phase Closure](docs/hardening/phase-closure.md) - Enforced runtime invariants and closure criteria
- [Release Notes](docs/release-notes.md) - Runtime hardening release entry
- [Sticky Scaling Checklist](docs/deployment/sticky-scaling-checklist.md) - Multi-replica sticky deployment requirements
- [Consumer Production Runbook](docs/deployment/consumer-production-runbook.md) - Startup gates, health semantics, and rollback playbook
- [Single-Replica Env Template](configs/single-replica.env) - Minimal deployment baseline
- [Multi-Replica Sticky Redis Env Template](configs/multi-replica-sticky-redis.env) - Consumer multi-node baseline

---

## 🐛 Troubleshooting

### Bot not responding
```bash
# Check logs
sudo journalctl -u nanobot -n 50

# Verify configuration
cat /opt/nanobot/.env

# Restart
sudo systemctl restart nanobot
```

### Antigravity not available
```bash
# Re-run OAuth
cargo run -- login antigravity
# Or add API key to .env
```

### Build errors
```bash
cargo clean
cargo build --release
```

---

## 🚧 Roadmap

- [x] Antigravity provider with OAuth
- [x] Telegram bot integration  
- [x] VPS deployment scripts
- [ ] Session persistence (SQLite)
- [ ] Tool calling (file ops, shell)
- [ ] WhatsApp bridge
- [ ] Discord bridge
- [ ] Docker image
- [ ] Binary releases

---

## 📄 License

MIT

---

## 🤝 Contributing

Pull requests welcome! See [CONTRIBUTING.md](CONTRIBUTING.md).
