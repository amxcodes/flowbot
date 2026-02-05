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

# Check if built
if [ ! -f "target/release/nanobot" ]; then
    echo "🔨 Building (first time - may take 5-10 minutes)..."
    cargo build --release
    echo "✓ Build complete"
    echo ""
fi

# Check if OAuth is needed
if [ -z "$ANTIGRAVITY_API_KEY" ] && [ ! -f "$HOME/.antigravity/tokens.json" ]; then
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
