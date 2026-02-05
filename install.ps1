# Flowbot Smart Installer (Windows)

Write-Host "🚀 Starting Flowbot Installation..." -ForegroundColor Cyan

# 1. Check for Rust/Cargo
if (-not (Get-Command "cargo" -ErrorAction SilentlyContinue)) {
    Write-Host "⚠️  Rust is not installed." -ForegroundColor Yellow
    Write-Host "Attempting to install Rust via Winget..." -ForegroundColor Cyan
    
    # Try Winget
    winget install Rustlang.Rustup
    
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Winget failed to install Rust. Please install manually from https://rustup.rs/"
        exit 1
    }
    
    # Refresh env vars (basic attempt, user might still need restart)
    $env:Path = [System.Environment]::GetEnvironmentVariable("Path","Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path","User")
    
    # Check again
    if (-not (Get-Command "cargo" -ErrorAction SilentlyContinue)) {
        Write-Host "⚠️  Rust installed, but not in current session PATH." -ForegroundColor Yellow
        Write-Host "Please RESTART your terminal and run this script again."
        exit 0
    }
}

# 2. Build & Install via Cargo
Write-Host "📦 Building and Installing Flowbot..." -ForegroundColor Cyan
cargo install --path . --force

if ($LASTEXITCODE -ne 0) {
    Write-Error "Installation failed. Check output above."
    exit 1
}

# 3. Setup Config Directory
$ConfigDir = "$HOME\.nanobot"
if (-not (Test-Path $ConfigDir)) {
    New-Item -ItemType Directory -Force -Path $ConfigDir | Out-Null
}

Write-Host "`n✨ Flowbot Installed Successfully!" -ForegroundColor Green
Write-Host "Try running: nanobot doctor"

# 4. Prompt for Service Installation (Task Scheduler)
Write-Host "`n📋 Service Installation (Optional)" -ForegroundColor Cyan
Write-Host "Would you like to install Nanobot as a system service?"
Write-Host "This enables 24/7 background operation with auto-restart."
$InstallService = Read-Host "Install as Task Scheduler service? [y/N]"

if ($InstallService -match "^[Yy]$") {
    Write-Host "🔧 Installing Task Scheduler service..." -ForegroundColor Cyan
    
    # Check if running as Administrator
    $IsAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    
    if (-not $IsAdmin) {
        Write-Host "⚠️  Administrator privileges required for service installation." -ForegroundColor Yellow
        Write-Host "Please run this command as Administrator:"
        Write-Host "  nanobot service install" -ForegroundColor White
    } else {
        $ServiceInstall = & nanobot service install 2>&1
        if ($LASTEXITCODE -eq 0) {
            Write-Host "✅ Service installed successfully!" -ForegroundColor Green
            Write-Host "You can now:"
            Write-Host "  - Start service: nanobot service start"
            Write-Host "  - Check status:  nanobot service status"
            Write-Host "  - View logs:     Get-Content ~\.nanobot\logs\nanobot.log -Tail 50 -Wait"
        } else {
            Write-Host "⚠️  Service installation failed." -ForegroundColor Yellow
            Write-Host "You can install it manually later with: nanobot service install"
        }
    }
} else {
    Write-Host "Skipped service installation."
    Write-Host "You can install it later with: nanobot service install"
}
