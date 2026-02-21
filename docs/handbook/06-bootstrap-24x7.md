# 24x7 Bootstrap Setup

This guide is the practical "make it run forever" path using the installer bootstrap flow.

## What "bootstrap" means in this repo

`install.sh` is the bootstrap installer.

It does all of this automatically:

- detects OS + package manager
- checks required dependencies
- installs missing required deps
- attempts optional deps (Docker/Chromium/Node/gh/deno, etc.)
- installs Rust toolchain if missing
- builds and installs `nanobot` via `cargo install --path . --force`
- prints readiness report

Non-interactive bootstrap mode is:

```bash
./install.sh --auto
```

## OS and package-manager auto-detection

Yes - bootstrap is cross-platform aware.

`install.sh` auto-detects platform and picks a package manager:

- Linux: `apt`, `dnf`, `yum`, `pacman`, `apk`, or `zypper`
- macOS: `brew` (and attempts Homebrew install if missing)
- Windows: `winget`, `choco`, or `scoop`

If no supported package manager is found, bootstrap exits with a clear error so you can install one and rerun.

### Dependency mapping behavior

The script maps the same logical dependency set to OS-specific package names.

Examples:

- OpenSSL headers -> `libssl-dev` (apt), `openssl-devel` (dnf/yum), `openssl` (pacman)
- build tools -> `build-essential` (apt), `gcc gcc-c++ make` (dnf/yum), `base-devel` (pacman)
- GitHub CLI -> `gh` (apt/dnf), `github-cli` (pacman), `GitHub.cli` (winget)

So you run one bootstrap command, and it installs the right thing per OS.

## Full 24x7 setup (recommended)

Goal: everything automated except the wizard answers.

## 1) Run bootstrap installer

From repo root:

```bash
chmod +x install.sh
./install.sh --auto
```

If you run remotely (not from repo), use:

```bash
curl -fsSL https://raw.githubusercontent.com/amxcodes/nanobot-rs-clean/main/install.sh -o install.sh
chmod +x install.sh
./install.sh --auto
```

If you want installer + wizard in one flow (wizard is still interactive):

```bash
./install.sh --auto --run-setup
```

## 2) Run guided setup

```bash
nanobot setup --wizard
```

In the wizard, make sure you configure:

- provider credentials (Antigravity/OpenAI/OpenRouter/Google)
- channel credentials (Telegram token, etc.)
- service install/start when prompted

## 3) Ensure service is installed and running

```bash
nanobot service install
nanobot service start
nanobot service status
```

On Linux with strict permissions, run with `sudo` if required:

```bash
sudo nanobot service install
sudo nanobot service start
sudo nanobot service status
```

Note: on Linux, prefer running service commands as your normal user. `sudo nanobot service install` may install the user service for `root` instead of your account.

### Enable auto-start on boot/login (platform-specific)

After `nanobot service install`, auto-start behavior is:

- Linux (systemd user service):

```bash
systemctl --user start nanobot
systemctl --user status nanobot
```

`nanobot service install` enables auto-start automatically.

It also attempts to enable `systemd` linger (so user services survive logout). If that step cannot be done automatically, run:

```bash
sudo loginctl enable-linger "$USER"
```

- macOS (launchd):
  - `nanobot service install` already loads with `RunAtLoad=true` and `KeepAlive=true`.
  - Start once: `nanobot service start`

- Windows (Task Scheduler):
  - `nanobot service install` creates boot/logon triggers automatically.
  - Start once: `nanobot service start`

## 4) Verify 24x7 behavior

- reboot the machine once and check service is back up
- verify status again: `nanobot service status`
- check logs for healthy startup

Linux logs:

```bash
journalctl --user -u nanobot -f
```

## Edge-case checklist (production hardening)

- SSH session closed and bot stops on Linux:
  - verify linger: `loginctl show-user "$USER" -p Linger`
  - if `Linger=no`, run: `sudo loginctl enable-linger "$USER"`
- `systemctl --user` fails with bus/session errors:
  - open a normal login shell (not restricted non-login shell) and retry
  - if needed, reboot once after enabling linger
- Installed with `sudo` and service is "missing":
  - you likely installed root's user-service; reinstall as your normal user
- macOS service not starting:
  - check plist exists in `~/Library/LaunchAgents`
  - reload: `launchctl unload -w ~/Library/LaunchAgents/com.nanobot.gateway.plist; launchctl load -w ~/Library/LaunchAgents/com.nanobot.gateway.plist`
- Windows task exists but bot is not running:
  - open Task Scheduler (`taskschd.msc`), run task manually once
  - verify account permissions and working directory access
- Remote bootstrap behind restricted network:
  - dependency installs can fail due to mirrors/firewall; pre-install packages manually and rerun `./install.sh --auto`
- OAuth in headless server:
  - use API key-based provider config, or run OAuth once from a machine with browser access then copy secure token state carefully
- Wrong repo/branch during remote install:
  - set `NANOBOT_REPO_URL` and `NANOBOT_REPO_REF` before running installer

## Bootstrap dependency checklist

The installer treats these as required (must pass):

- `curl`, `git`, `tar`, `bzip2`
- `pkg-config`, `cmake`, `perl`
- `python3`, `pip3`, `ffmpeg`
- compiler toolchain (`gcc` + `make`) on non-Windows
- OpenSSL dev libraries on non-Windows
- CA certificates on Linux

Optional but attempted automatically:

- Docker + docker compose
- Chromium + runtime libs
- Node.js/npm
- GitHub CLI (`gh`)
- Deno
- `gog` CLI
- `cargo-watch`

## Common failure fixes

- `nanobot: command not found`
  - add Cargo bin to PATH: `export PATH="$HOME/.cargo/bin:$PATH"`
- Deno-related skill errors
  - add Deno bin to PATH: `export PATH="$HOME/.deno/bin:$PATH"`
- service install fails
  - retry with sudo
  - check init system availability (systemd on Linux)
- provider auth errors after service starts
  - rerun setup wizard and verify `config.toml`/OAuth credentials

## Quick health commands

```bash
nanobot doctor
nanobot doctor --wiring
nanobot gateway --channel telegram
nanobot memory status
```

If you are setting up production, run `nanobot doctor` after every credential or policy change.

## Instant rollback / complete uninstall

If you want to fully remove Nanobot from a machine:

```bash
nanobot uninstall --yes
```

This command attempts to stop/uninstall service, remove local Nanobot folders, and remove the installed binary.
