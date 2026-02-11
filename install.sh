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

    local repo_url="${NANOBOT_REPO_URL:-https://github.com/amxcodes/flowbot.git}"
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

    SOURCE_DIR="$TEMP_CLONE_DIR"
}

dep_satisfied() {
    local dep="$1"
    case "$dep" in
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
        nodejs)
            has_cmd node
            ;;
        npm)
            has_cmd npm
            ;;
        gh)
            has_cmd gh
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

    local required_report=(curl git tar bzip2 pkg-config cmake perl python3 pip3 ffmpeg)
    if [ "$OS_FAMILY" != "windows" ]; then
        required_report+=(build-tools openssl-dev)
    fi
    if [ "$OS_FAMILY" = "linux" ]; then
        required_report+=(ca-certs)
    fi

    local optional_report=(docker docker-compose chromium nodejs gh cargo-watch)

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
                nodejs) echo "nodejs npm" ;;
                npm) echo "npm" ;;
                gh) echo "gh" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
        dnf)
            case "$dep" in
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
                nodejs) echo "nodejs npm" ;;
                npm) echo "npm" ;;
                gh) echo "gh" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
        yum)
            case "$dep" in
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
                nodejs) echo "nodejs npm" ;;
                npm) echo "npm" ;;
                gh) echo "gh" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
        pacman)
            case "$dep" in
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
                nodejs) echo "nodejs npm" ;;
                npm) echo "npm" ;;
                gh) echo "github-cli" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
        apk)
            case "$dep" in
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
                nodejs) echo "nodejs npm" ;;
                npm) echo "npm" ;;
                gh) echo "gh" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
        zypper)
            case "$dep" in
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
                nodejs) echo "nodejs npm" ;;
                npm) echo "npm" ;;
                gh) echo "gh" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
        brew)
            case "$dep" in
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
                nodejs) echo "node" ;;
                npm) echo "node" ;;
                gh) echo "gh" ;;
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
                nodejs) echo "OpenJS.NodeJS.LTS" ;;
                npm) echo "OpenJS.NodeJS.LTS" ;;
                gh) echo "GitHub.cli" ;;
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
                nodejs) echo "nodejs-lts" ;;
                npm) echo "nodejs-lts" ;;
                gh) echo "gh" ;;
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
                nodejs) echo "nodejs-lts" ;;
                npm) echo "nodejs-lts" ;;
                gh) echo "gh" ;;
                cargo-watch) echo "" ;;
            esac
            ;;
    esac
}

log_info "Starting Nanobot Installer..."

OS_FAMILY=$(detect_os)
PM=$(ensure_package_manager "$OS_FAMILY")

log_info "Detected OS: $OS_FAMILY"
log_info "Detected package manager: $PM"

missing_required=()
missing_optional=()

for cmd in curl git tar bzip2 pkg-config cmake perl ffmpeg; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        missing_required+=("$cmd")
    fi
done

if ! command -v python3 >/dev/null 2>&1 && ! command -v python >/dev/null 2>&1 && ! command -v py >/dev/null 2>&1; then
    missing_required+=("python3")
fi

if ! command -v pip3 >/dev/null 2>&1 && ! python3 -m pip --version >/dev/null 2>&1 && ! python -m pip --version >/dev/null 2>&1; then
    if ! python3 -m pip --version >/dev/null 2>&1; then
        missing_required+=("pip3")
    fi
fi

if [ "$OS_FAMILY" != "windows" ]; then
    if ! command -v gcc >/dev/null 2>&1 || ! command -v make >/dev/null 2>&1; then
        missing_required+=("build-tools")
    fi
fi

if [ "$OS_FAMILY" != "windows" ]; then
    if ! pkg-config --exists openssl >/dev/null 2>&1; then
        missing_required+=("openssl-dev")
    fi
fi

if [ "$OS_FAMILY" = "linux" ]; then
    if [ ! -f /etc/ssl/certs/ca-certificates.crt ] && [ ! -d /etc/ssl/certs ]; then
        missing_required+=("ca-certs")
    fi
fi

if ! command -v docker >/dev/null 2>&1; then
    missing_optional+=("docker")
fi
if ! command -v docker-compose >/dev/null 2>&1; then
    if ! docker compose version >/dev/null 2>&1; then
        missing_optional+=("docker-compose")
    fi
fi
if ! command -v cargo-watch >/dev/null 2>&1; then
    missing_optional+=("cargo-watch")
fi
if ! command -v chromium >/dev/null 2>&1 \
    && ! command -v chromium-browser >/dev/null 2>&1 \
    && ! command -v google-chrome >/dev/null 2>&1 \
    && ! command -v google-chrome-stable >/dev/null 2>&1 \
    && ! command -v chrome >/dev/null 2>&1; then
    missing_optional+=("chromium")
fi
if ! command -v node >/dev/null 2>&1; then
    missing_optional+=("nodejs")
fi
if ! command -v gh >/dev/null 2>&1; then
    missing_optional+=("gh")
fi

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

if [ "$(uname -s)" = "Darwin" ] && ! xcode-select -p >/dev/null 2>&1; then
    log_warn "Xcode Command Line Tools are required for Rust builds on macOS."
    log_warn "Running: xcode-select --install"
    xcode-select --install || true
fi

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

install_openai_whisper_if_possible

prepare_source_dir

print_readiness_report

log_info "Building and installing Nanobot..."
(cd "$SOURCE_DIR" && cargo install --path . --force)

mkdir -p "$HOME/.nanobot"

log_ok "Installation complete."
echo
echo "Next steps:"
echo "  1) Ensure PATH includes: $HOME/.cargo/bin"
echo "  2) Run setup when ready: nanobot setup --wizard"
echo "  3) Optional offline models: nanobot setup --offline-models"
echo "  4) Optional browser tools: choose Docker or local Chromium in setup wizard"
