# Telegram Bot Setup Guide

## Quick Start

### 1. Create a Telegram Bot

1. Open Telegram and search for [@BotFather](https://t.me/botfather)
2. Send `/newbot` command
3. Follow the prompts to name your bot
4. **Copy the token** provided (looks like: `123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11`)

### 2. Get Your User ID (Optional but Recommended)

1. Search for [@userinfobot](https://t.me/userinfobot) on Telegram
2. Start a chat and it will show your User ID
3. Save this number for security configuration

### 3. Run the Bot

```bash
# Set your bot token
export TELEGRAM_BOT_TOKEN="your_bot_token_here"

# Optional: Restrict to specific users (comma-separated)
export TELEGRAM_ALLOWED_USERS="123456789,987654321"

# Run the gateway
cargo run -- gateway
```

**Windows PowerShell:**
```powershell
$env:TELEGRAM_BOT_TOKEN="your_bot_token_here"
$env:TELEGRAM_ALLOWED_USERS="123456789"
cargo run -- gateway
```

### 4. Test Your Bot

1. Open Telegram
2. Search for your bot by username
3. Send a message: "Hello!"
4. The bot should echo back: "🤖 You said: Hello!"

---

## Architecture

```
┌──────────────┐                    ┌─────────────────┐
│  Telegram    │                    │  nanobot-rs     │
│  (Cloud)     │                    │  (Your PC)      │
│              │                    │                 │
│              │◄─── HTTP GET ──────│  Bot Polling    │
│              │     getUpdates     │   Loop          │
│              │                    │                 │
│              │──── HTTP POST ────►│  Agent          │
│              │     sendMessage    │   Processor     │
└──────────────┘                    └─────────────────┘
```

**No WebSocket needed!** Just simple HTTP polling every 1-2 seconds.

---

## How It Works

### Message Flow

1. **User sends message** on Telegram
2. **Bot polls** Telegram API (`getUpdates`)
3. **Receives messages** via HTTP GET
4. **Sends to agent** via mpsc channel
5. **Agent processes** (currently just echoes)
6. **Sends response** back via mpsc channel
7. **Bot sends to Telegram** via HTTP POST (`sendMessage`)

### Code Structure

```
src/telegram.rs
├── TelegramBot       # Handles Telegram API (polling, sending)
├── TelegramMessage   # Message from user
├── TelegramResponse  # Response to send back
├── SimpleAgent       # Processes messages (TODO: integrate real agent)
└── TelegramConfig    # Bot configuration
```

---

## Security

### User Authorization

By default, **anyone** can message your bot. To restrict access:

```bash
# Only allow specific users
export TELEGRAM_ALLOWED_USERS="123456789,987654321"
```

**Without this**, the bot will warn:
```
⚠️  No user restrictions (set TELEGRAM_ALLOWED_USERS for security)
```

### Unauthorized Access

If an unauthorized user messages your bot:
```
Bot: ⛔ You are not authorized to use this bot.
```

---

## Next Steps

### TODO: Integrate Real Agent

Currently, the `SimpleAgent` just echoes messages. Next:

1. **Connect to Antigravity** provider
2. **Add tool calling** (read/write files, exec commands)
3. **Session management** (remember conversation history)
4. **Streaming responses** (chunk messages for long responses)

### Example Integration

```rust
// In telegram.rs SimpleAgent::run()
while let Some(msg) = self.telegram_rx.recv().await {
    // Instead of echo, call real agent:
    let antigravity = AntigravityClient::from_env().await?;
    let agent = antigravity.agent("gemini-2.5-flash").build();
    
    let response_text = agent.prompt(&msg.text).await?;
    
    let response = TelegramResponse {
        chat_id: msg.chat_id,
        text: response_text,
    };
    
    self.response_tx.send(response).await?;
}
```

---

## Commands Reference

```bash
# Run Telegram bot
cargo run -- gateway

# Run one-time agent (CLI)
cargo run -- agent -m "Hello" -p antigravity

# Interactive chat (TUI)
cargo run -- chat -p antigravity

# Setup OAuth
cargo run -- login antigravity
```

---

## Troubleshooting

### "TELEGRAM_BOT_TOKEN not found"

**Solution**: Set the environment variable
```bash
export TELEGRAM_BOT_TOKEN="your_token_here"
```

### Bot doesn't respond

1. Check the token is correct
2. Make sure the bot is running (`cargo run -- gateway`)
3. Check your user ID is in `TELEGRAM_ALLOWED_USERS`
4. Look at the terminal output for errors

### "You are not authorized"

Add your user ID to allowed users:
```bash
export TELEGRAM_ALLOWED_USERS="your_user_id"
```

---

## Comparison: Telegram vs Other Channels

| Feature | Telegram | WhatsApp | Discord |
|---------|----------|----------|---------|
| **Setup** | ✅ Easy (BotFather) | 😐 Medium (QR code) | ✅ Easy (Bot token) |
| **Protocol** | HTTP polling | HTTP/Node bridge | HTTP webhooks |
| **Public IP needed** | ❌ No | ❌ No | ❌ No |
| **Code complexity** | ✅ ~100 lines | 😐 ~200 lines | ✅ ~120 lines |

**Telegram is the easiest to set up!** 🎉

---

## What's Next?

After testing the Telegram bot:

1. ✅ **Telegram bot** - DONE (you are here)
2. ⏳ **Integrate Antigravity agent** - Next
3. ⏳ **Add tools** (file read/write, exec) - Soon
4. ⏳ **Session storage** (SQLite) - Soon
5. ⏳ **WhatsApp bridge** - Later

**You now have a working Telegram bot!** 🤖
