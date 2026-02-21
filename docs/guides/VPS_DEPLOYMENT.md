# VPS Deployment Guide

Complete guide for deploying nanobot-rs on a VPS (Ubuntu/Debian/CentOS).

---

## Quick Start (Automated)

```bash
# One-command installation (as root)
curl -sSL https://raw.githubusercontent.com/yourusername/nanobot-rs/main/install.sh | sudo bash

# Configure environment
sudo nano /opt/nanobot/.env

# Login to Antigravity (OAuth)
sudo -u nanobot /opt/nanobot/nanobot-rs login antigravity

# Start service
sudo systemctl start nanobot
sudo systemctl enable nanobot  # Auto-start on boot
```

**Done!** Your bot is running 24/7.

---

## Manual Installation

### 1. Install Dependencies

**Ubuntu/Debian:**
```bash
sudo apt update
sudo apt install -y curl build-essential pkg-config libssl-dev git
```

**CentOS/RHEL:**
```bash
sudo yum install -y curl gcc openssl-devel pkg-config git
```

### 2. Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

### 3. Create User & Directories

```bash
# Create dedicated user
sudo useradd -r -m -s /bin/bash nanobot

# Create directory
sudo mkdir -p /opt/nanobot
sudo chown -R nanobot:nanobot /opt/nanobot
```

### 4. Clone & Build

```bash
cd /opt/nanobot
sudo -u nanobot git clone https://github.com/yourusername/nanobot-rs-clean.git
cd nanobot-rs-clean

# Build release binary
sudo -u nanobot cargo build --release

# Copy binary
sudo cp target/release/nanobot-rs /opt/nanobot/nanobot-rs
```

### 5. Configure Environment

```bash
# Copy template
sudo cp .env.example /opt/nanobot/.env
sudo chown nanobot:nanobot /opt/nanobot/.env
sudo chmod 600 /opt/nanobot/.env

# Edit configuration
sudo nano /opt/nanobot/.env
```

Add your credentials:
```bash
TELEGRAM_BOT_TOKEN=123456:ABC-DEF...
TELEGRAM_ALLOWED_USERS=your_user_id
```

### 6. Setup Antigravity (OAuth Recommended)

```bash
# Switch to nanobot user
sudo su - nanobot

# Run OAuth login
/opt/nanobot/nanobot-rs login antigravity
# Follow browser prompts to authorize

# Exit back to root
exit
```

**Alternative: API Key**
```bash
# Edit .env and add:
ANTIGRAVITY_API_KEY=AIza...
```

### 7. Install Systemd Service

```bash
# Copy service file
sudo cp /opt/nanobot/nanobot-rs-clean/nanobot.service /etc/systemd/system/

# Reload systemd
sudo systemctl daemon-reload

# Enable auto-start
sudo systemctl enable nanobot

# Start service
sudo systemctl start nanobot
```

---

## Service Management

### Start/Stop/Restart

```bash
sudo systemctl start nanobot    # Start
sudo systemctl stop nanobot     # Stop
sudo systemctl restart nanobot  # Restart
sudo systemctl status nanobot   # Check status
```

### View Logs

```bash
# Real-time logs
sudo journalctl -u nanobot -f

# Last 100 lines
sudo journalctl -u nanobot -n 100

# Logs since boot
sudo journalctl -u nanobot -b
```

### Auto-start on Boot

```bash
sudo systemctl enable nanobot   # Enable
sudo systemctl disable nanobot  # Disable
```

---

## Configuration

### Environment Variables

Edit `/opt/nanobot/.env`:

```bash
sudo nano /opt/nanobot/.env
```

**Required:**
- `TELEGRAM_BOT_TOKEN` - From @BotFather

**Recommended:**
- `TELEGRAM_ALLOWED_USERS` - User ID whitelist

**Optional:**
- `ANTIGRAVITY_API_KEY` - If not using OAuth
- `RUST_LOG=info` - Logging level

### Reload After Changes

```bash
sudo systemctl restart nanobot
```

---

## Updating

### Update Code

```bash
cd /opt/nanobot/nanobot-rs-clean
sudo -u nanobot git pull
sudo -u nanobot cargo build --release
sudo cp target/release/nanobot-rs /opt/nanobot/nanobot-rs
sudo systemctl restart nanobot
```

### Quick Update Script

Create `/opt/nanobot/update.sh`:
```bash
#!/bin/bash
cd /opt/nanobot/nanobot-rs-clean
git pull
cargo build --release
cp target/release/nanobot-rs /opt/nanobot/nanobot-rs
systemctl restart nanobot
echo "✅ Updated and restarted"
```

```bash
sudo chmod +x /opt/nanobot/update.sh
sudo /opt/nanobot/update.sh
```

---

## Security Hardening

### Firewall (UFW)

```bash
# Allow SSH
sudo ufw allow 22/tcp

# Enable firewall
sudo ufw enable

# Bot uses outbound HTTPS only (no inbound ports needed)
```

### File Permissions

```bash
# Secure .env file
sudo chmod 600 /opt/nanobot/.env
sudo chown nanobot:nanobot /opt/nanobot/.env

# Verify
ls -la /opt/nanobot/.env
# Should show: -rw------- 1 nanobot nanobot
```

### OAuth Tokens

Stored in `/home/nanobot/.antigravity/tokens.json`:
```bash
sudo chmod 600 /home/nanobot/.antigravity/tokens.json
sudo chown nanobot:nanobot /home/nanobot/.antigravity/tokens.json
```

---

## Monitoring

### Health Check Script

Create `/opt/nanobot/healthcheck.sh`:
```bash
#!/bin/bash
if systemctl is-active --quiet nanobot; then
    echo "✅ Nanobot is running"
    exit 0
else
    echo "❌ Nanobot is down!"
    systemctl restart nanobot
    exit 1
fi
```

### Cron Job (Auto-restart if down)

```bash
sudo crontab -e

# Add:
*/5 * * * * /opt/nanobot/healthcheck.sh >> /var/log/nanobot-health.log 2>&1
```

### Resource Usage

```bash
# CPU & Memory
ps aux | grep nanobot-rs

# Detailed stats
sudo systemctl status nanobot
```

---

## Troubleshooting

### Bot Not Responding

**Check service status:**
```bash
sudo systemctl status nanobot
```

**Check logs:**
```bash
sudo journalctl -u nanobot -n 50
```

**Common issues:**

1. **"Antigravity not available"**
   ```bash
   # Run OAuth login
   sudo -u nanobot /opt/nanobot/nanobot-rs login antigravity
   ```

2. **"TELEGRAM_BOT_TOKEN not found"**
   ```bash
   # Check .env file
   sudo cat /opt/nanobot/.env
   # Ensure TELEGRAM_BOT_TOKEN is set
   ```

3. **Permission denied**
   ```bash
   # Fix ownership
   sudo chown -R nanobot:nanobot /opt/nanobot
   sudo chown -R nanobot:nanobot /home/nanobot/.antigravity
   ```

### Service Won't Start

```bash
# Check for errors
sudo journalctl -u nanobot -xe

# Manually test
sudo -u nanobot /opt/nanobot/nanobot-rs gateway
```

### High CPU Usage

```bash
# Check logs for errors
sudo journalctl -u nanobot -n 100

# Restart service
sudo systemctl restart nanobot
```

---

## Backup & Restore

### Backup

```bash
# Backup configuration & tokens
sudo tar -czf nanobot-backup.tar.gz \
    /opt/nanobot/.env \
    /home/nanobot/.antigravity/tokens.json \
    /opt/nanobot/config.toml

# Download
scp root@your-vps:/root/nanobot-backup.tar.gz .
```

### Restore

```bash
# Upload backup
scp nanobot-backup.tar.gz root@your-vps:/tmp/

# Extract
sudo tar -xzf /tmp/nanobot-backup.tar.gz -C /

# Fix permissions
sudo chown nanobot:nanobot /opt/nanobot/.env
sudo chown nanobot:nanobot /home/nanobot/.antigravity/tokens.json

# Restart
sudo systemctl restart nanobot
```

---

## Multiple Bots (Same VPS)

Run multiple bots with different tokens:

```bash
# Create second instance
sudo cp /etc/systemd/system/nanobot.service /etc/systemd/system/nanobot2.service

# Edit service file
sudo nano /etc/systemd/system/nanobot2.service
# Change EnvironmentFile to /opt/nanobot/.env2

# Create second env file
sudo cp /opt/nanobot/.env /opt/nanobot/.env2
sudo nano /opt/nanobot/.env2
# Set different TELEGRAM_BOT_TOKEN

# Start second bot
sudo systemctl start nanobot2
sudo systemctl enable nanobot2
```

---

## Performance Tuning

### Resource Limits

Edit `/etc/systemd/system/nanobot.service`:

```ini
[Service]
MemoryLimit=512M
CPUQuota=100%
TasksMax=100
```

### Logging Level

Edit `/opt/nanobot/.env`:
```bash
RUST_LOG=warn  # Less verbose (error, warn, info, debug, trace)
```

---

## Uninstall

```bash
# Stop service
sudo systemctl stop nanobot
sudo systemctl disable nanobot

# Remove service file
sudo rm /etc/systemd/system/nanobot.service
sudo systemctl daemon-reload

# Remove files
sudo rm -rf /opt/nanobot
sudo userdel -r nanobot
```

---

## Support

- GitHub Issues: https://github.com/yourusername/nanobot-rs/issues
- Documentation: `/opt/nanobot/nanobot-rs-clean/docs/guides/TELEGRAM_SETUP.md`
- Logs: `sudo journalctl -u nanobot -f`
