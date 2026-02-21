#!/bin/bash
# Quick start script for VPS deployment

echo "🚀 Nanobot Quick Start"
echo ""

# Check if .env exists
if [ ! -f .env ]; then
    echo "📝 Creating .env from template..."
    cp .env.example .env
    echo "⚠️  Please edit .env and add your credentials:"
    echo "   nano .env"
    echo ""
    exit 1
fi

# Check if TELEGRAM_BOT_TOKEN is set
source .env
if [ -z "$TELEGRAM_BOT_TOKEN" ] || [ "$TELEGRAM_BOT_TOKEN" = "your_bot_token_here" ]; then
    echo "❌ TELEGRAM_BOT_TOKEN not configured"
    echo "   Edit .env and add your bot token"
    exit 1
fi

echo "✓ Configuration found"
echo ""

# Runtime readiness for skills
if ! command -v deno >/dev/null 2>&1; then
    echo "⚠️  Deno not found. Install Deno for community skills (recommended)."
fi

if ! command -v gh >/dev/null 2>&1; then
    echo "⚠️  gh CLI not found. Install GitHub CLI for github skill."
fi

if ! command -v node >/dev/null 2>&1; then
    echo "ℹ️  Node.js not found. Optional, only needed for some legacy skill fallbacks."
fi

if ! command -v gog >/dev/null 2>&1; then
    echo "ℹ️  gog CLI not found. Needed for Google Workspace gog skill."
fi

echo "💡 Run 'nanobot doctor' for full dependency checks."
echo ""

# Check if built
if [ ! -f "target/release/nanobot" ]; then
    echo "🔨 Building (first time - may take 5-10 minutes)..."
    cargo build --release
    echo "✓ Build complete"
    echo ""
fi

# Check if OAuth is needed
if [ -z "${ANTIGRAVITY_API_KEY:-}" ] \
    && [ ! -f "$HOME/.nanobot/secrets.enc" ] \
    && [ ! -f "$HOME/.nanobot/tokens.json" ] \
    && [ ! -f "$HOME/.openclaw/auth/tokens.json" ]; then
    echo "🔐 Antigravity OAuth setup required"
    echo "   Run: cargo run --release -- login antigravity"
    echo ""
    read -p "Run OAuth setup now? (y/n): " choice
    if [ "$choice" = "y" ]; then
        cargo run --release -- login antigravity
    else
        echo "⚠️  Skipping OAuth - bot will run in degraded mode"
    fi
    echo ""
fi

echo "🤖 Starting Telegram bot..."
echo ""
cargo run --release -- gateway
