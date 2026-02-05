# Windows Build Fix Script
# This script works around the "os error 32" file locking issue on Windows

Write-Host "🔧 Windows Build Fix for Flowbot" -ForegroundColor Cyan
Write-Host "================================`n" -ForegroundColor Cyan

# Step 1: Kill any lingering Rust processes
Write-Host "1. Killing lingering Rust processes..." -ForegroundColor Yellow
Get-Process | Where-Object { $_.ProcessName -like "*rust*" -or $_.ProcessName -like "*cargo*" } | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1

# Step 2: Clean build artifacts
Write-Host "2. Cleaning build artifacts..." -ForegroundColor Yellow
if (Test-Path "target") {
    Remove-Item -Path "target" -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Host "   ✓ Cleaned target directory" -ForegroundColor Green

# Step 3: Set environment variables
Write-Host "3. Setting build environment..." -ForegroundColor Yellow
$env:CARGO_INCREMENTAL = "0"
$env:RUSTFLAGS = "-C target-cpu=native"
Write-Host "   ✓ Disabled incremental compilation" -ForegroundColor Green
Write-Host "   ✓ Set optimal CPU flags" -ForegroundColor Green

# Step 4: Build with single job
Write-Host "4. Building (this may take 3-5 minutes)..." -ForegroundColor Yellow
Write-Host "   Using single-threaded build to avoid file locks`n" -ForegroundColor Gray

$buildCommand = "cargo build --release --jobs 1"
Invoke-Expression $buildCommand

if ($LASTEXITCODE -eq 0) {
    Write-Host "`n✅ Build successful!" -ForegroundColor Green
    Write-Host "`nBinary location: target\release\flowbot.exe" -ForegroundColor Cyan
    Write-Host "`nTest it:" -ForegroundColor Cyan
    Write-Host "  .\target\release\flowbot.exe setup --wizard" -ForegroundColor White
} else {
    Write-Host "`n❌ Build failed with error code: $LASTEXITCODE" -ForegroundColor Red
    Write-Host "`nTroubleshooting:" -ForegroundColor Yellow
    Write-Host "  1. Close VS Code and all IDEs" -ForegroundColor White
    Write-Host "  2. Run: taskkill /F /IM rust-analyzer.exe" -ForegroundColor White
    Write-Host "  3. Try again" -ForegroundColor White
    Write-Host "`nOr use WSL/VPS for reliable builds." -ForegroundColor Yellow
}
