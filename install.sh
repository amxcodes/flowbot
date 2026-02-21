#!/bin/bash
set -euo pipefail
IFS=$'\n\t'

trap 'printf "\033[0;31m%s\033[0m\n" "Installer failed at line $LINENO"' ERR
trap cleanup_temp_clone EXIT

CYAN="\033[0;36m"
GREEN="\033[0;32m"
YELLOW="\033[0;33m"
RED="\033[0;31m"
NC="\033[0m"

log_info() { printf "${CYAN}%s${NC}\n" "$1"; }
log_ok() { printf "${GREEN}%s${NC}\n" "$1"; }
log_warn() { printf "${YELLOW}%s${NC}\n" "$1"; }
log_err() { printf "${RED}%s${NC}\n" "$1"; }

contains_pkg() {
    local needle="$1"
    shift || true
    local item
    for item in "$@"; do
        if [ "$item" = "$needle" ]; then
            return 0
        fi
    done
    return 1
}

has_cmd() {
    command -v "$1" >/dev/null 2>&1
}

sha256_cmd() {
    if has_cmd sha256sum; then
        echo "sha256sum"
        return 0
    fi
    if has_cmd shasum; then
        echo "shasum"
        return 0
    fi
    if has_cmd openssl; then
        echo "openssl"
        return 0
    fi
    return 1
}

calc_sha256() {
    local file="$1"
    local tool
    tool=$(sha256_cmd) || return 1
    case "$tool" in
        sha256sum)
            sha256sum "$file" | awk '{print $1}'
            ;;
        shasum)
            shasum -a 256 "$file" | awk '{print $1}'
            ;;
        openssl)
            openssl dgst -sha256 "$file" | awk '{print $NF}'
            ;;
    esac
}

verify_sha256() {
    local file="$1"
    local expected="$2"
    local actual
    actual=$(calc_sha256 "$file") || return 1
    expected=$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')
    actual=$(printf '%s' "$actual" | tr '[:upper:]' '[:lower:]')
    [ "$actual" = "$expected" ]
}

retry_cmd() {
    local attempts=0
    local max_attempts=3
    local delay=2

    until "$@"; do
        attempts=$((attempts + 1))
        if [ "$attempts" -ge "$max_attempts" ]; then
            return 1
        fi
        sleep "$delay"
        delay=$((delay * 2))
    done
}

SOURCE_DIR=""
TEMP_CLONE_DIR=""
AUTO_MODE=0
RUN_WIZARD_AFTER="ask"
USE_CASE_PROFILE="${NANOBOT_USE_CASE:-auto}"
INSTALL_PRESET="${NANOBOT_INSTALL_PRESET:-full}"
NANOBOT_INSTALL_METHOD="${NANOBOT_INSTALL_METHOD:-auto}"
NANOBOT_BINARY_URL="${NANOBOT_BINARY_URL:-}"
NANOBOT_BINARY_SHA256="${NANOBOT_BINARY_SHA256:-}"
NANOBOT_BINARY_SHA256_URL="${NANOBOT_BINARY_SHA256_URL:-}"
NANOBOT_BINARY_SIG_URL="${NANOBOT_BINARY_SIG_URL:-}"
NANOBOT_COSIGN_PUBKEY="${NANOBOT_COSIGN_PUBKEY:-}"
NANOBOT_REQUIRE_BINARY_CHECKSUM="${NANOBOT_REQUIRE_BINARY_CHECKSUM:-1}"
NANOBOT_EXPECTED_COMMIT="${NANOBOT_EXPECTED_COMMIT:-}"

print_usage() {
    cat <<'EOF'
Usage: ./install.sh [options]

Options:
  --auto, --yes, -y   Non-interactive bootstrap mode
  --run-setup         Run setup wizard automatically at end
  --use-case <name>   Setup type (auto, general, remote-vps-channel)
  --preset <name>     Install preset (full, minimal)
  --method <name>     Install method (auto, source, binary)
  --help, -h          Show this help
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --auto|--yes|-y)
            AUTO_MODE=1
            RUN_WIZARD_AFTER="no"
            ;;
        --run-setup)
            RUN_WIZARD_AFTER="yes"
            ;;
        --use-case)
            shift
            if [ "$#" -eq 0 ]; then
                log_err "--use-case requires a value"
                print_usage
                exit 1
            fi
            USE_CASE_PROFILE="$1"
            ;;
        --use-case=*)
            USE_CASE_PROFILE="${1#*=}"
            ;;
        --preset)
            shift
            if [ "$#" -eq 0 ]; then
                log_err "--preset requires a value"
                print_usage
                exit 1
            fi
            INSTALL_PRESET="$1"
            ;;
        --preset=*)
            INSTALL_PRESET="${1#*=}"
            ;;
        --method)
            shift
            if [ "$#" -eq 0 ]; then
                log_err "--method requires a value"
                print_usage
                exit 1
            fi
            NANOBOT_INSTALL_METHOD="$1"
            ;;
        --method=*)
            NANOBOT_INSTALL_METHOD="${1#*=}"
            ;;
        --help|-h)
            print_usage
            exit 0
            ;;
        *)
            log_err "Unknown option: $1"
            print_usage
            exit 1
            ;;
    esac
    shift
done

normalize_use_case() {
    local raw="$1"
    raw=$(printf '%s' "$raw" | tr '[:upper:]' '[:lower:]')
    case "$raw" in
        ""|auto|smart|default) echo "auto" ;;
        general|normal|desktop|local|standard) echo "general" ;;
        remote-vps-channel|remote|vps|server|channels) echo "remote-vps-channel" ;;
        *) echo "" ;;
    esac
}

normalize_install_preset() {
    local raw="$1"
    raw=$(printf '%s' "$raw" | tr '[:upper:]' '[:lower:]')
    case "$raw" in
        ""|full|recommended|all|complete) echo "full" ;;
        minimal|lite|light|core) echo "minimal" ;;
        *) echo "" ;;
    esac
}

normalize_install_method() {
    local raw="$1"
    raw=$(printf '%s' "$raw" | tr '[:upper:]' '[:lower:]')
    case "$raw" in
        ""|auto|smart|default) echo "auto" ;;
        source|build) echo "source" ;;
        binary|release|prebuilt) echo "binary" ;;
        *) echo "" ;;
    esac
}

infer_use_case_profile() {
    local deploy_target
    deploy_target=$(printf '%s' "${NANOBOT_DEPLOY_TARGET:-}" | tr '[:upper:]' '[:lower:]')
    case "$deploy_target" in
        vps|server|remote)
            echo "remote-vps-channel"
            return 0
            ;;
        local|desktop)
            echo "general"
            return 0
            ;;
    esac

    if [ "${NANOBOT_SCALING_MODE:-}" = "sticky" ] && [ "${NANOBOT_REPLICA_COUNT:-1}" -gt 1 ] 2>/dev/null; then
        echo "remote-vps-channel"
        return 0
    fi

    if [ -n "${NANOBOT_STICKY_SIGNAL_HEADER:-}" ] || [ -n "${NANOBOT_REDIS_URL:-}" ]; then
        echo "remote-vps-channel"
        return 0
    fi

    if has_cmd systemctl && [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
        echo "remote-vps-channel"
        return 0
    fi

    echo "general"
}

resolve_use_case_profile() {
    local normalized
    normalized=$(normalize_use_case "$USE_CASE_PROFILE")
    if [ -z "$normalized" ]; then
        log_err "Unsupported --use-case: $USE_CASE_PROFILE"
        log_err "Supported values: auto, general, remote-vps-channel"
        exit 1
    fi

    if [ "$normalized" = "auto" ]; then
        local inferred
        inferred=$(infer_use_case_profile)
        USE_CASE_PROFILE="$inferred"
    else
        USE_CASE_PROFILE="$normalized"
    fi
}

resolve_use_case_profile

resolve_install_preset() {
    local normalized
    normalized=$(normalize_install_preset "$INSTALL_PRESET")
    if [ -z "$normalized" ]; then
        log_err "Unsupported --preset: $INSTALL_PRESET"
        log_err "Supported values: full, minimal"
        exit 1
    fi

    INSTALL_PRESET="$normalized"
}

resolve_install_preset

resolve_install_method() {
    local normalized
    normalized=$(normalize_install_method "$NANOBOT_INSTALL_METHOD")
    if [ -z "$normalized" ]; then
        log_err "Unsupported --method: $NANOBOT_INSTALL_METHOD"
        log_err "Supported values: auto, source, binary"
        exit 1
    fi

    if [ "$normalized" = "auto" ]; then
        if [ -n "$NANOBOT_BINARY_URL" ]; then
            NANOBOT_INSTALL_METHOD="binary"
        else
            NANOBOT_INSTALL_METHOD="source"
        fi
    else
        NANOBOT_INSTALL_METHOD="$normalized"
    fi
}

resolve_install_method

use_case_label() {
    case "$USE_CASE_PROFILE" in
        remote-vps-channel) echo "Remote VPS/Server" ;;
        *) echo "Personal Computer" ;;
    esac
}

install_preset_label() {
    case "$INSTALL_PRESET" in
        minimal) echo "Lightweight" ;;
        *) echo "Recommended (Full)" ;;
    esac
}

install_method_label() {
    case "$NANOBOT_INSTALL_METHOD" in
        binary) echo "Prebuilt binary (with checksum)" ;;
        *) echo "Build from source" ;;
    esac
}

requires_source_build() {
    [ "$NANOBOT_INSTALL_METHOD" = "source" ]
}

cleanup_temp_clone() {
    if [ -n "${TEMP_CLONE_DIR:-}" ] && [ -d "$TEMP_CLONE_DIR" ]; then
        rm -rf "$TEMP_CLONE_DIR" || true
    fi
}

prepare_source_dir() {
    if [ -f "Cargo.toml" ]; then
        SOURCE_DIR="$(pwd)"
        return 0
    fi

    local repo_url="${NANOBOT_REPO_URL:-https://github.com/amxcodes/nanobot-rs-clean.git}"
    local repo_ref="${NANOBOT_REPO_REF:-main}"

    if ! has_cmd git; then
        log_err "Git is required to fetch source when running installer remotely."
        exit 1
    fi

    TEMP_CLONE_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t nanobot-install)"
    log_info "Fetching source from: ${repo_url} (${repo_ref})"

    if ! git clone --depth 1 --branch "$repo_ref" "$repo_url" "$TEMP_CLONE_DIR"; then
        log_warn "Could not clone branch '$repo_ref', retrying default branch..."
        git clone --depth 1 "$repo_url" "$TEMP_CLONE_DIR"
    fi

    if [ ! -f "$TEMP_CLONE_DIR/Cargo.toml" ]; then
        log_err "Fetched source does not contain Cargo.toml at repo root."
        exit 1
    fi

    if [ -n "$NANOBOT_EXPECTED_COMMIT" ]; then
        local actual_commit
        actual_commit=$(git -C "$TEMP_CLONE_DIR" rev-parse HEAD 2>/dev/null || true)
        if [ -z "$actual_commit" ] || [ "$actual_commit" != "$NANOBOT_EXPECTED_COMMIT" ]; then
            log_err "Source verification failed: expected commit $NANOBOT_EXPECTED_COMMIT, got ${actual_commit:-unknown}"
            exit 1
        fi
        log_ok "Verified source commit: $actual_commit"
    fi

    SOURCE_DIR="$TEMP_CLONE_DIR"
}

install_prebuilt_binary_if_configured() {
    if [ "$NANOBOT_INSTALL_METHOD" != "binary" ]; then
        return 1
    fi

    if [ -z "$NANOBOT_BINARY_URL" ]; then
        log_err "Install method is binary, but NANOBOT_BINARY_URL is not set."
        exit 1
    fi

    if ! has_cmd curl; then
        log_err "curl is required for binary install method."
        exit 1
    fi

    local tmp_dir archive_file checksum_text expected_checksum signature_file
    tmp_dir="$(mktemp -d 2>/dev/null || mktemp -d -t nanobot-binary)"
    archive_file="$tmp_dir/nanobot-download"

    log_info "Downloading prebuilt binary..."
    curl -fsSL "$NANOBOT_BINARY_URL" -o "$archive_file"

    expected_checksum="$NANOBOT_BINARY_SHA256"
    if [ -z "$expected_checksum" ] && [ -n "$NANOBOT_BINARY_SHA256_URL" ]; then
        checksum_text=$(curl -fsSL "$NANOBOT_BINARY_SHA256_URL" || true)
        expected_checksum=$(printf '%s' "$checksum_text" | awk '{print $1}')
    fi

    if [ -z "$expected_checksum" ] && [ "$NANOBOT_REQUIRE_BINARY_CHECKSUM" = "1" ]; then
        log_err "Binary checksum required but not provided. Set NANOBOT_BINARY_SHA256 or NANOBOT_BINARY_SHA256_URL."
        rm -rf "$tmp_dir" || true
        exit 1
    fi

    if [ -n "$expected_checksum" ]; then
        if ! verify_sha256 "$archive_file" "$expected_checksum"; then
            log_err "Checksum verification failed for downloaded binary."
            rm -rf "$tmp_dir" || true
            exit 1
        fi
        log_ok "Checksum verified"
    fi

    if [ -n "$NANOBOT_BINARY_SIG_URL" ]; then
        if ! has_cmd cosign; then
            log_err "Signature URL provided, but 'cosign' is not installed."
            rm -rf "$tmp_dir" || true
            exit 1
        fi
        if [ -z "$NANOBOT_COSIGN_PUBKEY" ]; then
            log_err "Signature verification requires NANOBOT_COSIGN_PUBKEY path."
            rm -rf "$tmp_dir" || true
            exit 1
        fi
        signature_file="$tmp_dir/nanobot-download.sig"
        curl -fsSL "$NANOBOT_BINARY_SIG_URL" -o "$signature_file"
        cosign verify-blob --key "$NANOBOT_COSIGN_PUBKEY" --signature "$signature_file" "$archive_file" >/dev/null
        log_ok "Signature verified"
    fi

    local out_dir target candidate
    out_dir="$HOME/.cargo/bin"
    mkdir -p "$out_dir"
    target="$out_dir/nanobot"

    case "$NANOBOT_BINARY_URL" in
        *.tar.gz|*.tgz)
            tar -xzf "$archive_file" -C "$tmp_dir"
            for candidate in "$tmp_dir/nanobot" "$tmp_dir/nanobot-rs" "$tmp_dir/bin/nanobot" "$tmp_dir/bin/nanobot-rs"; do
                if [ -f "$candidate" ]; then
                    cp "$candidate" "$target"
                    chmod +x "$target"
                    rm -rf "$tmp_dir" || true
                    log_ok "Installed prebuilt binary to $target"
                    return 0
                fi
            done
            log_err "Could not locate nanobot executable in archive."
            rm -rf "$tmp_dir" || true
            exit 1
            ;;
        *)
            cp "$archive_file" "$target"
            chmod +x "$target"
            rm -rf "$tmp_dir" || true
            log_ok "Installed prebuilt binary to $target"
            return 0
            ;;
    esac
}

dep_satisfied() {
    local dep="$1"
    case "$dep" in
        sh)
            has_cmd sh || has_cmd bash
            ;;
        ssh)
            has_cmd ssh
            ;;
        redis-cli)
            has_cmd redis-cli
            ;;
        redis-server)
            has_cmd redis-server
            ;;
        python3)
            has_cmd python3 || has_cmd python || has_cmd py
            ;;
        pip3)
            has_cmd pip3 || python3 -m pip --version >/dev/null 2>&1 || python -m pip --version >/dev/null 2>&1
            ;;
        docker-compose)
            has_cmd docker-compose || docker compose version >/dev/null 2>&1
            ;;
        chromium)
            has_cmd chromium || has_cmd chromium-browser || has_cmd google-chrome || has_cmd google-chrome-stable || has_cmd chrome || has_cmd msedge
            ;;
        chromium-runtime-libs)
            if [ "$OS_FAMILY" != "linux" ]; then
                return 0
            fi
            if has_cmd ldconfig; then
                ldconfig -p 2>/dev/null | grep -Eq 'libnss3|libgtk-3|libgbm'
            else
                [ -d /etc/ssl/certs ]
            fi
            ;;
        nodejs)
            has_cmd node
            ;;
        npm)
            has_cmd npm
            ;;
        gh)
            has_cmd gh
            ;;
        deno)
            has_cmd deno
            ;;
        gog)
            has_cmd gog
            ;;
        openssl-dev)
            pkg-config --exists openssl >/dev/null 2>&1
            ;;
        ca-certs)
            if [ "$OS_FAMILY" = "linux" ]; then
                [ -f /etc/ssl/certs/ca-certificates.crt ] || [ -d /etc/ssl/certs ]
            else
                return 0
            fi
            ;;
        build-tools)
            if [ "$OS_FAMILY" = "windows" ]; then
                return 0
            fi
            has_cmd gcc && has_cmd make
            ;;
        *)
            has_cmd "$dep"
            ;;
    esac
}

optional_dep_list() {
    local deps=()

    # Baseline runtime for agent + common channels/skills.
    for dep in deno nodejs npm gh gog; do
        if ! contains_pkg "$dep" "${deps[@]}"; then
            deps+=("$dep")
        fi
    done

    if [ "$INSTALL_PRESET" = "full" ]; then
        for dep in cargo-watch docker docker-compose chromium chromium-runtime-libs redis-cli redis-server; do
            if ! contains_pkg "$dep" "${deps[@]}"; then
                deps+=("$dep")
            fi
        done
    else
        if [ "$USE_CASE_PROFILE" = "general" ]; then
            deps+=("cargo-watch")
        fi

        if should_enable_browser_stack; then
            for dep in docker docker-compose chromium chromium-runtime-libs; do
                if ! contains_pkg "$dep" "${deps[@]}"; then
                    deps+=("$dep")
                fi
            done
        fi

        if should_enable_distributed_redis; then
            for dep in redis-cli redis-server; do
                if ! contains_pkg "$dep" "${deps[@]}"; then
                    deps+=("$dep")
                fi
            done
        fi
    fi

    if should_enable_browser_stack; then
        for dep in docker docker-compose; do
            if ! contains_pkg "$dep" "${deps[@]}"; then
                deps+=("$dep")
            fi
        done
    fi

    if [ "$USE_CASE_PROFILE" = "remote-vps-channel" ]; then
        for dep in systemctl journalctl ssh; do
            if ! contains_pkg "$dep" "${deps[@]}"; then
                deps+=("$dep")
            fi
        done
    fi

    printf '%s\n' "${deps[*]}"
}

is_truthy() {
    local raw="${1:-}"
    raw=$(printf '%s' "$raw" | tr '[:upper:]' '[:lower:]')
    case "$raw" in
        1|true|yes|y|on|enabled) return 0 ;;
        *) return 1 ;;
    esac
}

should_enable_browser_stack() {
    if [ "$INSTALL_PRESET" = "full" ]; then
        return 0
    fi

    if [ "$USE_CASE_PROFILE" = "general" ]; then
        return 0
    fi

    if is_truthy "${NANOBOT_ENABLE_BROWSER_TOOLS:-}"; then
        return 0
    fi

    if is_truthy "${NANOBOT_USE_BROWSER_DOCKER:-}"; then
        return 0
    fi

    local mode="${NANOBOT_BROWSER_MODE:-}"
    mode=$(printf '%s' "$mode" | tr '[:upper:]' '[:lower:]')
    case "$mode" in
        docker|local|enabled|on|true|1)
            return 0
            ;;
    esac

    return 1
}

should_enable_distributed_redis() {
    if [ "$INSTALL_PRESET" = "full" ]; then
        return 0
    fi

    if is_truthy "${NANOBOT_ENABLE_DISTRIBUTED_REDIS:-}"; then
        return 0
    fi

    local replicas="${NANOBOT_REPLICA_COUNT:-1}"
    if [ "$replicas" -gt 1 ] 2>/dev/null; then
        return 0
    fi

    if [ "${NANOBOT_DISTRIBUTED_STORE_BACKEND:-}" = "redis" ]; then
        return 0
    fi

    if [ "${NANOBOT_PROVIDER_LIMITER_BACKEND:-}" = "redis" ]; then
        return 0
    fi

    if [ -n "${NANOBOT_REDIS_URL:-}" ]; then
        return 0
    fi

    return 1
}

dep_requested() {
    local needle="$1"
    local dep
    for dep in $(optional_dep_list); do
        if [ "$dep" = "$needle" ]; then
            return 0
        fi
    done
    return 1
}

resolve_env_file_path() {
    if [ -n "${NANOBOT_ENV_FILE:-}" ]; then
        printf '%s\n' "$NANOBOT_ENV_FILE"
        return 0
    fi

    if [ -d "$PWD" ] && [ -w "$PWD" ]; then
        printf '%s\n' "$PWD/.env"
        return 0
    fi

    printf '%s\n' ""
}

upsert_env_var() {
    local env_file="$1"
    local key="$2"
    local value="$3"

    if [ -z "$env_file" ]; then
        return 1
    fi

    mkdir -p "$(dirname "$env_file")" >/dev/null 2>&1 || true
    if [ ! -f "$env_file" ]; then
        touch "$env_file" || return 1
    fi

    local tmp_file
    tmp_file="${env_file}.tmp.$$"

    awk -v k="$key" -v v="$value" '
        BEGIN { replaced = 0 }
        $0 ~ "^[[:space:]]*" k "=" {
            if (!replaced) {
                print k "=" v
                replaced = 1
            }
            next
        }
        { print }
        END {
            if (!replaced) {
                print k "=" v
            }
        }
    ' "$env_file" > "$tmp_file" && mv "$tmp_file" "$env_file"
}

apply_redis_env_defaults() {
    if ! should_enable_distributed_redis; then
        return 0
    fi

    local redis_url="${NANOBOT_REDIS_URL:-}"
    if [ -z "$redis_url" ]; then
        redis_url="redis://127.0.0.1:6379/"
        export NANOBOT_REDIS_URL="$redis_url"
        log_info "Redis URL not set. Using local default: $redis_url"
    fi

    local env_file
    env_file=$(resolve_env_file_path)
    if [ -n "$env_file" ]; then
        upsert_env_var "$env_file" "NANOBOT_REDIS_URL" "$redis_url" || true
        upsert_env_var "$env_file" "NANOBOT_DISTRIBUTED_STORE_BACKEND" "redis" || true
        upsert_env_var "$env_file" "NANOBOT_PROVIDER_LIMITER_BACKEND" "redis" || true
        upsert_env_var "$env_file" "NANOBOT_PROVIDER_LIMITER_FAILURE_MODE" "closed" || true
        if [ "$USE_CASE_PROFILE" = "remote-vps-channel" ]; then
            upsert_env_var "$env_file" "NANOBOT_SCALING_MODE" "sticky" || true
        fi
        log_ok "Updated Redis defaults in: $env_file"
    else
        log_warn "Could not find writable path for .env defaults. Export NANOBOT_REDIS_URL manually."
    fi
}

verify_deps_or_fail() {
    local label="$1"
    shift
    local deps=("$@")
    local missing=()
    local dep

    for dep in "${deps[@]}"; do
        if ! dep_satisfied "$dep"; then
            missing+=("$dep")
        fi
    done

    if [ "${#missing[@]}" -gt 0 ]; then
        log_err "$label missing after installation attempts: ${missing[*]}"
        log_err "Install the missing dependencies manually, then rerun install.sh"
        exit 1
    fi
}

verify_optional_deps() {
    local deps=("$@")
    local missing=()
    local dep

    for dep in "${deps[@]}"; do
        if ! dep_satisfied "$dep"; then
            missing+=("$dep")
        fi
    done

    if [ "${#missing[@]}" -gt 0 ]; then
        log_warn "Optional dependencies still missing: ${missing[*]}"
    else
        log_ok "Optional dependency set is ready."
    fi
}

install_openai_whisper_if_possible() {
    if has_cmd whisper; then
        return 0
    fi

    local py=""
    if has_cmd python3; then
        py="python3"
    elif has_cmd python; then
        py="python"
    elif has_cmd py; then
        py="py -3"
    fi

    if [ -z "$py" ]; then
        log_warn "Python not found; skipping openai-whisper install"
        return 0
    fi

    log_info "Installing Python package: openai-whisper"
    # shellcheck disable=SC2086
    if $py -m pip install -U openai-whisper; then
        log_ok "openai-whisper installed"
    else
        log_warn "Could not install openai-whisper automatically"
    fi
}

install_deno_if_possible() {
    if has_cmd deno; then
        return 0
    fi

    log_info "Installing Deno (recommended runtime for skills)..."
    if has_cmd curl; then
        if curl -fsSL https://deno.land/install.sh | sh; then
            export PATH="$HOME/.deno/bin:$PATH"
            if has_cmd deno; then
                log_ok "Deno installed"
                return 0
            fi
        fi
    fi

    log_warn "Could not install Deno automatically. Install from https://deno.com/manual/getting_started/installation"
    return 1
}

install_cargo_watch_if_possible() {
    if has_cmd cargo-watch; then
        return 0
    fi
    if ! has_cmd cargo; then
        log_warn "cargo not available yet; skipping cargo-watch install"
        return 1
    fi

    log_info "Installing cargo-watch (optional dev helper)..."
    if cargo install cargo-watch >/dev/null 2>&1; then
        log_ok "cargo-watch installed"
        return 0
    fi

    log_warn "Could not install cargo-watch automatically"
    return 1
}

install_gog_if_possible() {
    if has_cmd gog; then
        return 0
    fi

    if ! has_cmd npm; then
        log_warn "npm not found; skipping gog install"
        return 1
    fi

    log_info "Trying to install gog CLI (Google Workspace skill helper)..."

    if npm view gog version >/dev/null 2>&1 && npm install -g gog >/dev/null 2>&1; then
        log_ok "gog installed via npm package 'gog'"
        return 0
    fi

    if npm view @steipete/gogcli version >/dev/null 2>&1 && npm install -g @steipete/gogcli >/dev/null 2>&1; then
        log_ok "gog installed via npm package '@steipete/gogcli'"
        return 0
    fi

    log_warn "Could not install gog automatically. Install gog manually if you use Google Workspace skills."
    return 1
}

ensure_docker_service_ready() {
    if [ "$OS_FAMILY" != "linux" ]; then
        return 0
    fi
    if ! has_cmd docker; then
        return 0
    fi
    if docker info >/dev/null 2>&1; then
        return 0
    fi

    log_info "Docker detected but daemon is not ready. Attempting to start Docker service..."

    local sudo_cmd=""
    if [ "$(id -u)" -ne 0 ] && has_cmd sudo; then
        sudo_cmd="sudo"
    fi

    if has_cmd systemctl; then
        $sudo_cmd systemctl enable --now docker >/dev/null 2>&1 || $sudo_cmd systemctl start docker >/dev/null 2>&1 || true
    elif has_cmd service; then
        $sudo_cmd service docker start >/dev/null 2>&1 || true
    fi

    if [ "$(id -u)" -ne 0 ] && [ -n "${USER:-}" ] && has_cmd usermod; then
        if getent group docker >/dev/null 2>&1; then
            if ! id -nG "$USER" | tr ' ' '\n' | grep -qx "docker"; then
                $sudo_cmd usermod -aG docker "$USER" >/dev/null 2>&1 || true
                log_warn "Added '$USER' to docker group. Log out/in to use docker without sudo."
            fi
        fi
    fi

    if docker info >/dev/null 2>&1; then
        log_ok "Docker service is ready"
    else
        log_warn "Docker is installed but daemon is still unreachable. Start Docker manually if needed."
    fi
}

run_setup_wizard_if_available() {
    if command -v nanobot >/dev/null 2>&1; then
        nanobot setup --wizard || log_warn "Setup wizard exited early. You can rerun: nanobot setup --wizard"
    elif [ -x "$HOME/.cargo/bin/nanobot" ]; then
        "$HOME/.cargo/bin/nanobot" setup --wizard || log_warn "Setup wizard exited early. You can rerun: nanobot setup --wizard"
    else
        log_warn "nanobot binary not found in PATH yet. Open a new shell and run: nanobot setup --wizard"
    fi
}

install_rust_toolchain() {
    if has_cmd cargo; then
        return 0
    fi

    log_info "Rust/Cargo not found. Installing Rust toolchain..."

    case "$OS_FAMILY" in
        windows)
            if [ "$PM" = "winget" ]; then
                winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements || true
            elif [ "$PM" = "choco" ]; then
                choco install -y rustup.install || true
            elif [ "$PM" = "scoop" ]; then
                scoop install rustup || true
            fi

            if has_cmd rustup-init.exe; then
                rustup-init.exe -y || true
            elif has_cmd rustup-init; then
                rustup-init -y || true
            fi
            ;;
        *)
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
            ;;
    esac

    if [ -f "$HOME/.cargo/env" ]; then
        # shellcheck disable=SC1090
        source "$HOME/.cargo/env"
    fi
    export PATH="$HOME/.cargo/bin:$PATH"
}

print_readiness_report() {
    echo
    log_info "Environment readiness report"
    log_info "Setup type: $(use_case_label)"
    log_info "Install mode: $(install_preset_label)"

    local required_report=(curl tar bzip2 python3 pip3 ffmpeg)
    if requires_source_build; then
        required_report+=(git pkg-config cmake perl)
    fi
    if [ "$OS_FAMILY" != "windows" ]; then
        required_report+=(ssh sh)
    fi
    if [ "$OS_FAMILY" != "windows" ] && requires_source_build; then
        required_report+=(build-tools openssl-dev)
    fi
    if [ "$OS_FAMILY" = "linux" ]; then
        required_report+=(ca-certs)
    fi

    local optional_report=()
    # shellcheck disable=SC2207
    optional_report=($(optional_dep_list))

    local dep
    for dep in "${required_report[@]}"; do
        if dep_satisfied "$dep"; then
            printf "  [required] %-16s %s\n" "$dep" "READY"
        else
            printf "  [required] %-16s %s\n" "$dep" "MISSING"
        fi
    done

    for dep in "${optional_report[@]}"; do
        if dep_satisfied "$dep"; then
            printf "  [optional] %-16s %s\n" "$dep" "READY"
        else
            printf "  [optional] %-16s %s\n" "$dep" "MISSING"
        fi
    done
}

detect_pm() {
    local os_family="$1"

    if [ "$os_family" = "windows" ]; then
        if command -v winget >/dev/null 2>&1; then
            echo "winget"
        elif command -v choco >/dev/null 2>&1; then
            echo "choco"
        elif command -v scoop >/dev/null 2>&1; then
            echo "scoop"
        else
            echo "unknown"
        fi
        return
    fi

    if command -v apt-get >/dev/null 2>&1; then
        echo "apt"
    elif command -v dnf >/dev/null 2>&1; then
        echo "dnf"
    elif command -v yum >/dev/null 2>&1; then
        echo "yum"
    elif command -v pacman >/dev/null 2>&1; then
        echo "pacman"
    elif command -v apk >/dev/null 2>&1; then
        echo "apk"
    elif command -v zypper >/dev/null 2>&1; then
        echo "zypper"
    elif command -v brew >/dev/null 2>&1; then
        echo "brew"
    else
        echo "unknown"
    fi
}

detect_os() {
    local uname_s
    uname_s=$(uname -s 2>/dev/null || echo "")
    case "$uname_s" in
        Linux*) echo "linux" ;;
        Darwin*) echo "macos" ;;
        MINGW*|MSYS*|CYGWIN*|Windows_NT*) echo "windows" ;;
        *) echo "unknown" ;;
    esac
}

ensure_package_manager() {
    local os_family="$1"
    local pm
    pm=$(detect_pm "$os_family")
    if [ "$pm" != "unknown" ]; then
        echo "$pm"
        return 0
    fi

    case "$os_family" in
        macos)
            if command -v curl >/dev/null 2>&1; then
                log_warn "No package manager detected. Installing Homebrew..."
                /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)" || {
                    log_err "Failed to install Homebrew automatically."
                    exit 1
                }
                if [ -x "/opt/homebrew/bin/brew" ]; then
                    eval "$(/opt/homebrew/bin/brew shellenv)"
                elif [ -x "/usr/local/bin/brew" ]; then
                    eval "$(/usr/local/bin/brew shellenv)"
                fi
                pm=$(detect_pm "$os_family")
                if [ "$pm" != "unknown" ]; then
                    echo "$pm"
                    return 0
                fi
            fi
            log_err "No package manager detected on macOS. Install Homebrew and rerun."
            exit 1
            ;;
        windows)
            log_warn "No Windows package manager detected (winget/choco/scoop)."
            log_warn "Attempting to install Chocolatey automatically..."
            if command -v powershell.exe >/dev/null 2>&1; then
                powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "Set-ExecutionPolicy Bypass -Scope Process -Force; [System.Net.ServicePointManager]::SecurityProtocol = [System.Net.ServicePointManager]::SecurityProtocol -bor 3072; iwr https://community.chocolatey.org/install.ps1 -UseBasicParsing | iex" || true
                export PATH="$PATH:/c/ProgramData/chocolatey/bin"
                pm=$(detect_pm "$os_family")
                if [ "$pm" != "unknown" ]; then
                    echo "$pm"
                    return 0
                fi
            fi
            log_err "Could not bootstrap a Windows package manager automatically. Install winget/choco/scoop and rerun."
            exit 1
            ;;
        linux)
            log_err "No supported package manager detected on Linux. Install one of: apt, dnf, yum, pacman, apk, zypper."
            exit 1
            ;;
        *)
            log_err "Unsupported OS for automatic dependency installation."
            exit 1
            ;;
    esac
}

install_packages() {
    local pm="$1"
    shift
    local pkgs=("$@")

    if [ "${#pkgs[@]}" -eq 0 ]; then
        return 0
    fi

    local sudo_cmd=""
    case "$pm" in
        apt|dnf|yum|pacman|apk|zypper)
            if [ "$(id -u)" -ne 0 ]; then
                if command -v sudo >/dev/null 2>&1; then
                    sudo_cmd="sudo"
                else
                    log_err "Need root privileges or sudo to install dependencies."
                    exit 1
                fi
            fi
            ;;
    esac

    log_info "Installing missing dependencies: ${pkgs[*]}"

    case "$pm" in
        apt)
            retry_cmd $sudo_cmd apt-get update -y
            retry_cmd $sudo_cmd apt-get install -y --no-install-recommends "${pkgs[@]}"
            ;;
        dnf)
            retry_cmd $sudo_cmd dnf install -y "${pkgs[@]}"
            ;;
        yum)
            retry_cmd $sudo_cmd yum install -y "${pkgs[@]}"
            ;;
        pacman)
            retry_cmd $sudo_cmd pacman -Sy --noconfirm "${pkgs[@]}"
            ;;
        apk)
            retry_cmd $sudo_cmd apk add --no-cache "${pkgs[@]}"
            ;;
        zypper)
            retry_cmd $sudo_cmd zypper --non-interactive install "${pkgs[@]}"
            ;;
        brew)
            retry_cmd brew install "${pkgs[@]}"
            ;;
        winget)
            local pkg
            for pkg in "${pkgs[@]}"; do
                winget install --id "$pkg" -e --accept-source-agreements --accept-package-agreements || true
            done
            ;;
        choco)
            choco install -y "${pkgs[@]}"
            ;;
        scoop)
            scoop install "${pkgs[@]}"
            ;;
        *)
            log_err "Unsupported package manager. Install dependencies manually and rerun."
            exit 1
            ;;
    esac
}

install_packages_best_effort() {
    local pm="$1"
    shift
    local pkgs=("$@")
    if [ "${#pkgs[@]}" -eq 0 ]; then
        return 0
    fi

    if install_packages "$pm" "${pkgs[@]}"; then
        return 0
    fi

    log_warn "Could not install some optional dependencies automatically: ${pkgs[*]}"
    return 1
}

pkg_for() {
    local pm="$1"
    local dep="$2"
    case "$pm" in
        apt)
            case "$dep" in
                ssh) echo "openssh-client" ;;
                sh) echo "bash" ;;
                systemctl|journalctl) echo "systemd" ;;
                curl) echo "curl" ;;
                git) echo "git" ;;
                tar) echo "tar" ;;
                bzip2) echo "bzip2" ;;
                pkg-config) echo "pkg-config" ;;
                build-tools) echo "build-essential" ;;
                cmake) echo "cmake" ;;
                perl) echo "perl" ;;
                openssl-dev) echo "libssl-dev" ;;
                ca-certs) echo "ca-certificates" ;;
                python3) echo "python3" ;;
                pip3) echo "python3-pip" ;;
                ffmpeg) echo "ffmpeg" ;;
                docker) echo "docker.io" ;;
                docker-compose) echo "docker-compose-plugin" ;;
                chromium) echo "chromium-browser" ;;
                chromium-runtime-libs) echo "libnss3 libatk-bridge2.0-0 libxkbcommon0 libxcomposite1 libxdamage1 libxrandr2 libgbm1 libasound2 libxshmfence1 libgtk-3-0 fonts-liberation" ;;
                nodejs) echo "nodejs npm" ;;
                npm) echo "npm" ;;
                gh) echo "gh" ;;
                redis-cli) echo "redis-tools" ;;
                redis-server) echo "redis-server" ;;
                deno) echo "" ;;
                gog) echo "" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
        dnf)
            case "$dep" in
                ssh) echo "openssh-clients" ;;
                sh) echo "bash" ;;
                systemctl|journalctl) echo "systemd" ;;
                curl) echo "curl" ;;
                git) echo "git" ;;
                tar) echo "tar" ;;
                bzip2) echo "bzip2" ;;
                pkg-config) echo "pkgconf-pkg-config" ;;
                build-tools) echo "gcc gcc-c++ make" ;;
                cmake) echo "cmake" ;;
                perl) echo "perl" ;;
                openssl-dev) echo "openssl-devel" ;;
                ca-certs) echo "ca-certificates" ;;
                python3) echo "python3" ;;
                pip3) echo "python3-pip" ;;
                ffmpeg) echo "ffmpeg" ;;
                docker) echo "docker" ;;
                docker-compose) echo "docker-compose-plugin" ;;
                chromium) echo "chromium" ;;
                chromium-runtime-libs) echo "nss atk at-spi2-atk libXcomposite libXdamage libXrandr mesa-libgbm alsa-lib libxshmfence gtk3 liberation-fonts" ;;
                nodejs) echo "nodejs npm" ;;
                npm) echo "npm" ;;
                gh) echo "gh" ;;
                redis-cli) echo "redis" ;;
                redis-server) echo "redis" ;;
                deno) echo "" ;;
                gog) echo "" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
        yum)
            case "$dep" in
                ssh) echo "openssh-clients" ;;
                sh) echo "bash" ;;
                systemctl|journalctl) echo "systemd" ;;
                curl) echo "curl" ;;
                git) echo "git" ;;
                tar) echo "tar" ;;
                bzip2) echo "bzip2" ;;
                pkg-config) echo "pkgconfig" ;;
                build-tools) echo "gcc gcc-c++ make" ;;
                cmake) echo "cmake" ;;
                perl) echo "perl" ;;
                openssl-dev) echo "openssl-devel" ;;
                ca-certs) echo "ca-certificates" ;;
                python3) echo "python3" ;;
                pip3) echo "python3-pip" ;;
                ffmpeg) echo "ffmpeg" ;;
                docker) echo "docker" ;;
                docker-compose) echo "docker-compose-plugin" ;;
                chromium) echo "chromium" ;;
                chromium-runtime-libs) echo "nss atk at-spi2-atk libXcomposite libXdamage libXrandr mesa-libgbm alsa-lib libxshmfence gtk3 liberation-fonts" ;;
                nodejs) echo "nodejs npm" ;;
                npm) echo "npm" ;;
                gh) echo "gh" ;;
                redis-cli) echo "redis" ;;
                redis-server) echo "redis" ;;
                deno) echo "" ;;
                gog) echo "" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
        pacman)
            case "$dep" in
                ssh) echo "openssh" ;;
                sh) echo "bash" ;;
                systemctl|journalctl) echo "systemd" ;;
                curl) echo "curl" ;;
                git) echo "git" ;;
                tar) echo "tar" ;;
                bzip2) echo "bzip2" ;;
                pkg-config) echo "pkgconf" ;;
                build-tools) echo "base-devel" ;;
                cmake) echo "cmake" ;;
                perl) echo "perl" ;;
                openssl-dev) echo "openssl" ;;
                ca-certs) echo "ca-certificates" ;;
                python3) echo "python" ;;
                pip3) echo "python-pip" ;;
                ffmpeg) echo "ffmpeg" ;;
                docker) echo "docker" ;;
                docker-compose) echo "docker-compose" ;;
                chromium) echo "chromium" ;;
                chromium-runtime-libs) echo "nss atk at-spi2-atk libxcomposite libxdamage libxrandr mesa-libgbm alsa-lib libxshmfence gtk3 ttf-liberation" ;;
                nodejs) echo "nodejs npm" ;;
                npm) echo "npm" ;;
                gh) echo "github-cli" ;;
                redis-cli) echo "redis" ;;
                redis-server) echo "redis" ;;
                deno) echo "" ;;
                gog) echo "" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
        apk)
            case "$dep" in
                ssh) echo "openssh-client" ;;
                sh) echo "bash" ;;
                curl) echo "curl" ;;
                git) echo "git" ;;
                tar) echo "tar" ;;
                bzip2) echo "bzip2" ;;
                pkg-config) echo "pkgconf" ;;
                build-tools) echo "build-base" ;;
                cmake) echo "cmake" ;;
                perl) echo "perl" ;;
                openssl-dev) echo "openssl-dev" ;;
                ca-certs) echo "ca-certificates" ;;
                python3) echo "python3" ;;
                pip3) echo "py3-pip" ;;
                ffmpeg) echo "ffmpeg" ;;
                docker) echo "docker" ;;
                docker-compose) echo "docker-cli-compose" ;;
                chromium) echo "chromium" ;;
                chromium-runtime-libs) echo "nss atk at-spi2-atk libxcomposite libxdamage libxrandr mesa-gbm alsa-lib libxshmfence gtk+3.0 ttf-freefont" ;;
                nodejs) echo "nodejs npm" ;;
                npm) echo "npm" ;;
                gh) echo "gh" ;;
                redis-cli) echo "redis" ;;
                redis-server) echo "redis" ;;
                deno) echo "" ;;
                gog) echo "" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
        zypper)
            case "$dep" in
                ssh) echo "openssh" ;;
                sh) echo "bash" ;;
                systemctl|journalctl) echo "systemd" ;;
                curl) echo "curl" ;;
                git) echo "git" ;;
                tar) echo "tar" ;;
                bzip2) echo "bzip2" ;;
                pkg-config) echo "pkgconf-pkg-config" ;;
                build-tools) echo "gcc gcc-c++ make" ;;
                cmake) echo "cmake" ;;
                perl) echo "perl" ;;
                openssl-dev) echo "libopenssl-devel" ;;
                ca-certs) echo "ca-certificates" ;;
                python3) echo "python3" ;;
                pip3) echo "python3-pip" ;;
                ffmpeg) echo "ffmpeg" ;;
                docker) echo "docker" ;;
                docker-compose) echo "docker-compose" ;;
                chromium) echo "chromium" ;;
                chromium-runtime-libs) echo "libnss3 libatk-bridge-2_0-0 libXcomposite1 libXdamage1 libXrandr2 libgbm1 alsa-lib libxshmfence1 gtk3 liberation-fonts" ;;
                nodejs) echo "nodejs npm" ;;
                npm) echo "npm" ;;
                gh) echo "gh" ;;
                redis-cli) echo "redis" ;;
                redis-server) echo "redis" ;;
                deno) echo "" ;;
                gog) echo "" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
        brew)
            case "$dep" in
                ssh) echo "openssh" ;;
                sh) echo "bash" ;;
                curl) echo "curl" ;;
                git) echo "git" ;;
                tar) echo "gnu-tar" ;;
                bzip2) echo "bzip2" ;;
                pkg-config) echo "pkg-config" ;;
                build-tools) echo "" ;;
                cmake) echo "cmake" ;;
                perl) echo "perl" ;;
                openssl-dev) echo "openssl@3" ;;
                ca-certs) echo "" ;;
                python3) echo "python" ;;
                pip3) echo "python" ;;
                ffmpeg) echo "ffmpeg" ;;
                docker) echo "docker" ;;
                docker-compose) echo "docker-compose" ;;
                chromium) echo "chromium" ;;
                chromium-runtime-libs) echo "" ;;
                nodejs) echo "node" ;;
                npm) echo "node" ;;
                gh) echo "gh" ;;
                redis-cli) echo "redis" ;;
                redis-server) echo "redis" ;;
                deno) echo "deno" ;;
                gog) echo "steipete/tap/gogcli" ;;
                cargo-watch) echo "cargo-watch" ;;
            esac
            ;;
        winget)
            case "$dep" in
                curl) echo "cURL.cURL" ;;
                git) echo "Git.Git" ;;
                tar) echo "GnuWin32.Tar" ;;
                bzip2) echo "GnuWin32.Bzip2" ;;
                pkg-config) echo "pkgconfig.pkgconfig" ;;
                build-tools) echo "Microsoft.VisualStudio.2022.BuildTools" ;;
                cmake) echo "Kitware.CMake" ;;
                perl) echo "StrawberryPerl.StrawberryPerl" ;;
                openssl-dev) echo "ShiningLight.OpenSSL.Light" ;;
                ca-certs) echo "" ;;
                python3) echo "Python.Python.3.12" ;;
                pip3) echo "" ;;
                ffmpeg) echo "Gyan.FFmpeg" ;;
                docker) echo "Docker.DockerDesktop" ;;
                docker-compose) echo "" ;;
                chromium) echo "Chromium.Chromium" ;;
                chromium-runtime-libs) echo "" ;;
                nodejs) echo "OpenJS.NodeJS.LTS" ;;
                npm) echo "OpenJS.NodeJS.LTS" ;;
                gh) echo "GitHub.cli" ;;
                redis-cli) echo "" ;;
                redis-server) echo "" ;;
                deno) echo "DenoLand.Deno" ;;
                gog) echo "" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
        choco)
            case "$dep" in
                curl) echo "curl" ;;
                git) echo "git" ;;
                tar) echo "gnuwin32-tar" ;;
                bzip2) echo "bzip2" ;;
                pkg-config) echo "pkgconfiglite" ;;
                build-tools) echo "visualstudio2022buildtools" ;;
                cmake) echo "cmake" ;;
                perl) echo "strawberryperl" ;;
                openssl-dev) echo "openssl" ;;
                ca-certs) echo "" ;;
                python3) echo "python" ;;
                pip3) echo "" ;;
                ffmpeg) echo "ffmpeg" ;;
                docker) echo "docker-desktop" ;;
                docker-compose) echo "" ;;
                chromium) echo "chromium" ;;
                chromium-runtime-libs) echo "" ;;
                nodejs) echo "nodejs-lts" ;;
                npm) echo "nodejs-lts" ;;
                gh) echo "gh" ;;
                redis-cli) echo "redis-64" ;;
                redis-server) echo "redis-64" ;;
                deno) echo "deno" ;;
                gog) echo "" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
        scoop)
            case "$dep" in
                curl) echo "curl" ;;
                git) echo "git" ;;
                tar) echo "tar" ;;
                bzip2) echo "bzip2" ;;
                pkg-config) echo "pkg-config" ;;
                build-tools) echo "llvm" ;;
                cmake) echo "cmake" ;;
                perl) echo "perl" ;;
                openssl-dev) echo "openssl" ;;
                ca-certs) echo "" ;;
                python3) echo "python" ;;
                pip3) echo "" ;;
                ffmpeg) echo "ffmpeg" ;;
                docker) echo "docker" ;;
                docker-compose) echo "docker-compose" ;;
                chromium) echo "chromium" ;;
                chromium-runtime-libs) echo "" ;;
                nodejs) echo "nodejs-lts" ;;
                npm) echo "nodejs-lts" ;;
                gh) echo "gh" ;;
                redis-cli) echo "redis" ;;
                redis-server) echo "redis" ;;
                deno) echo "deno" ;;
                gog) echo "" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
    esac
}

log_info "Starting Nanobot Installer..."

if [ "$AUTO_MODE" -eq 1 ]; then
    log_info "Bootstrap mode enabled (--auto): running non-interactive installer flow"
fi

OS_FAMILY=$(detect_os)
PM=$(ensure_package_manager "$OS_FAMILY")

log_info "Detected OS: $OS_FAMILY"
log_info "Detected package manager: $PM"
log_info "Setup type selected: $(use_case_label)"
log_info "Install mode selected: $(install_preset_label)"
log_info "Install method selected: $(install_method_label)"
if should_enable_browser_stack; then
    log_info "Browser tools: enabled"
else
    log_info "Browser tools: skipped"
fi
if should_enable_distributed_redis; then
    log_info "Distributed Redis: enabled"
else
    log_info "Distributed Redis: skipped"
fi

apply_redis_env_defaults

missing_required=()
missing_optional=()

required_cmds=(curl tar bzip2 ffmpeg)
if requires_source_build; then
    required_cmds+=(git pkg-config cmake perl)
fi

for cmd in "${required_cmds[@]}"; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        missing_required+=("$cmd")
    fi
done

if [ "$OS_FAMILY" != "windows" ]; then
    for cmd in ssh sh; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            missing_required+=("$cmd")
        fi
    done
fi

if ! command -v python3 >/dev/null 2>&1 && ! command -v python >/dev/null 2>&1 && ! command -v py >/dev/null 2>&1; then
    missing_required+=("python3")
fi

if ! command -v pip3 >/dev/null 2>&1 && ! python3 -m pip --version >/dev/null 2>&1 && ! python -m pip --version >/dev/null 2>&1; then
    if ! python3 -m pip --version >/dev/null 2>&1; then
        missing_required+=("pip3")
    fi
fi

if [ "$OS_FAMILY" != "windows" ] && requires_source_build; then
    if ! command -v gcc >/dev/null 2>&1 || ! command -v make >/dev/null 2>&1; then
        missing_required+=("build-tools")
    fi
fi

if [ "$OS_FAMILY" != "windows" ] && requires_source_build; then
    if ! pkg-config --exists openssl >/dev/null 2>&1; then
        missing_required+=("openssl-dev")
    fi
fi

if [ "$OS_FAMILY" = "linux" ]; then
    if [ ! -f /etc/ssl/certs/ca-certificates.crt ] && [ ! -d /etc/ssl/certs ]; then
        missing_required+=("ca-certs")
    fi
fi

for dep in $(optional_dep_list); do
    if ! dep_satisfied "$dep"; then
        missing_optional+=("$dep")
    fi
done

if [ "${#missing_required[@]}" -gt 0 ]; then
    log_info "Checking and installing required dependencies..."
    packages=()
    for dep in "${missing_required[@]}"; do
        mapped=$(pkg_for "$PM" "$dep")
        if [ -n "${mapped:-}" ]; then
            for p in $mapped; do
                if ! contains_pkg "$p" "${packages[@]}"; then
                    packages+=("$p")
                fi
            done
        fi
    done

    if [ "${#packages[@]}" -gt 0 ]; then
        install_packages "$PM" "${packages[@]}"
    fi
fi

verify_deps_or_fail "Required dependencies" "${missing_required[@]}"

if [ "${#missing_optional[@]}" -gt 0 ]; then
    log_info "Optional dependencies used by some features are missing: ${missing_optional[*]}"
    optional_packages=()
    for dep in "${missing_optional[@]}"; do
        mapped=$(pkg_for "$PM" "$dep")
        if [ -n "${mapped:-}" ]; then
            for p in $mapped; do
                if ! contains_pkg "$p" "${optional_packages[@]}"; then
                    optional_packages+=("$p")
                fi
            done
        fi
    done

    if [ "${#optional_packages[@]}" -gt 0 ]; then
        install_packages_best_effort "$PM" "${optional_packages[@]}" || true
    fi
fi

verify_optional_deps "${missing_optional[@]}"

if requires_source_build && [ "$(uname -s)" = "Darwin" ] && ! xcode-select -p >/dev/null 2>&1; then
    log_warn "Xcode Command Line Tools are required for Rust builds on macOS."
    log_warn "Running: xcode-select --install"
    xcode-select --install || true
fi

if requires_source_build; then
    install_rust_toolchain

    if [ -f "$HOME/.cargo/env" ]; then
        # shellcheck disable=SC1090
        source "$HOME/.cargo/env"
    fi
    export PATH="$HOME/.cargo/bin:$PATH"

    if ! command -v cargo >/dev/null 2>&1; then
        log_err "Cargo still not found after installation. Restart shell and rerun installer."
        exit 1
    fi
fi

if dep_requested "deno"; then
    install_deno_if_possible || true
    export PATH="$HOME/.deno/bin:$PATH"
fi
if dep_requested "cargo-watch"; then
    install_cargo_watch_if_possible || true
fi
if dep_requested "gog"; then
    install_gog_if_possible || true
fi
if dep_requested "docker"; then
    ensure_docker_service_ready || true
fi

install_openai_whisper_if_possible

print_readiness_report

if install_prebuilt_binary_if_configured; then
    :
else
    prepare_source_dir

    log_info "Building and installing Nanobot..."

    requested_features="${NANOBOT_CARGO_FEATURES:-}"
    if should_enable_distributed_redis; then
        if [ -n "$requested_features" ]; then
            case " $requested_features " in
                *" distributed-redis "*) ;;
                *) requested_features="$requested_features distributed-redis" ;;
            esac
        else
            requested_features="distributed-redis"
        fi
    fi

    if [ -n "$requested_features" ]; then
        log_info "Cargo features enabled: $requested_features"
        (cd "$SOURCE_DIR" && cargo install --path . --force --features "$requested_features")
    else
        (cd "$SOURCE_DIR" && cargo install --path . --force)
    fi
fi

mkdir -p "$HOME/.nanobot"

log_ok "Installation complete."
echo
echo "Next steps:"
echo "  1) Ensure PATH includes: $HOME/.cargo/bin"
echo "  2) Ensure PATH includes: $HOME/.deno/bin (for Deno skills)"
echo "  3) Run setup when ready: nanobot setup --wizard"
echo "  4) Optional offline models: nanobot setup --offline-models"
echo "  5) Optional browser tools: choose Docker or local Chromium in setup wizard"
echo "  Tip: Fully automatic install next time: ./install.sh --auto"

if [ "$RUN_WIZARD_AFTER" = "yes" ]; then
    run_setup_wizard_if_available
elif [ "$RUN_WIZARD_AFTER" = "ask" ] && [ -t 0 ]; then
    read -r -p "Run guided setup now? [Y/n] " run_setup_now
    run_setup_now=${run_setup_now:-Y}
    if [ "$run_setup_now" = "Y" ] || [ "$run_setup_now" = "y" ]; then
        run_setup_wizard_if_available
    fi
fi
