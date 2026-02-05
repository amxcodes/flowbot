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
Write-Host "Try running: flowbot doctor"
