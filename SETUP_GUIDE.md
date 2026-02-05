# 🚀 Nanobot Setup Guide

## Interactive Setup Wizard (NEW!)

Run the setup wizard to configure everything at once:

```powershell
cd "C:\Users\AMAN ANU\Desktop\amxcodes\nanobot\nanobot-rs-clean"
.\target\release\nanobot-rs.exe setup
```

### What the Setup Wizard Does:

```
╔═══════════════════════════════════════╗
║   🤖 Nanobot Setup Wizard            ║
╚═══════════════════════════════════════╝

📋 Step 1: Provider Selection

Available providers:
  1. OpenRouter  (Recommended - Access to multiple models)
  2. Antigravity (Google AI Studio - Gemini models)
  3. OpenAI      (GPT models - requires API key)
  4. Configure multiple providers

Which would you like to set up? (1-4):
```

**If you choose OpenRouter (recommended):**
- Asks for your API key from https://openrouter.ai/keys
- Validates it starts with `sk-or-v1-`
- Saves to config.toml

**If you choose Antigravity:**
- Asks: API Key or OAuth?
- For API Key: Gets key from https://makersuite.google.com/app/apikey
- For OAuth: Tells you to run `nanobot-rs login antigravity`

**If you choose OpenAI:**
- Asks for API key from https://platform.openai.com/api-keys
- Validates and saves

**Step 2: Default Provider**
- Shows you which providers are configured
- Lets you choose the default one

**Final Step:**
- Saves config.toml
- Shows next steps

---

## Manual Setup (Alternative)

If you prefer to edit config.toml manually:

### 1. Get API Keys

**OpenRouter** (Easiest - Recommended):
1. Go to https://openrouter.ai/keys
2. Sign up (free)
3. Create an API key
4. Copy it (starts with `sk-or-v1-...`)

**Antigravity** (Google AI Studio):
1. Go to https://makersuite.google.com/app/apikey
2. Sign in with Google
3. Create API key
4. Copy it (starts with `AI...`)

### 2. Edit config.toml

Open: `C:\Users\AMAN ANU\Desktop\amxcodes\nanobot\nanobot-rs-clean\config.toml`

```toml
# Choose your default provider
default_provider = "openrouter"  # or "antigravity" or "openai"

[providers.openrouter]
api_key = "sk-or-v1-YOUR_KEY_HERE"  # ← Paste your actual key

[providers.antigravity]
api_key = "AI..."  # ← Paste if using Antigravity

[providers.openai]
api_key = ""  # ← Paste if using OpenAI API
```

### 3. Test It!

```powershell
.\target\release\nanobot-rs.exe chat
```

---

## Available Commands

Once setup is complete:

### Interactive TUI Chat (Recommended)
```powershell
nanobot-rs chat
```
- Full-screen chat interface
- Tool calling visualization
- Press Esc to exit

### CLI Agent Mode
```powershell
nanobot-rs agent -m "your message here"
```
- Single question & answer
- Good for scripting

### OAuth Login (For Antigravity OAuth)
```powershell
nanobot-rs login antigravity
```
- Opens Google OAuth flow
- Saves token for later use

### Re-run Setup
```powershell
nanobot-rs setup
```
- Run setup wizard again
- Updates existing config

---

## Example Usage

### After Running Setup:

```powershell
# Start chatting
.\target\release\nanobot-rs.exe chat

# In the chat, try:
> What files are in the current directory?

🔧 Using tool: {"tool": "list_directory", "path": ".", "max_depth": 1}

✓ Tool result:
[
  {"name": "Cargo.toml", "is_dir": false, "size": 642},
  {"name": "demo.txt", "is_dir": false, "size": 221},
  ...
]

Nanobot: I found 11 files and 3 directories in your current folder...
```

### Try These Prompts:

- "Read the demo.txt file"
- "Search for Rust async programming tutorials"
- "Run the command: cargo --version"
- "Create a file called hello.txt with 'Hello World'"

---

## Troubleshooting

**Q: Setup command not found?**
- Make sure you built the latest version: `cargo build --release`
- The nex binary is at `target/release/nanobot-rs.exe`

**Q: Invalid API key?**
- OpenRouter keys start with: `sk-or-v1-`
- Google AI Studio keys start with: `AI`
- OpenAI keys start with: `sk-proj-`

**Q: Want to change provider?**
- Run `nanobot-rs setup` again
- Or edit `config.toml` directly
- Or use `-p` flag: `nanobot-rs chat -p antigravity`

---

## Quick Start (Copy-Paste)

```powershell
# Navigate to project
cd "C:\Users\AMAN ANU\Desktop\amxcodes\nanobot\nanobot-rs-clean"

# Run setup wizard
.\target\release\nanobot-rs.exe setup

# Follow prompts to configure OpenRouter

# Start chatting!
.\target\release\nanobot-rs.exe chat
```

That's it! 🎉
