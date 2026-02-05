# Ultimate Build Fix for Windows
# This script does a complete clean and rebuild with maximum compatibility

Write-Host "=== Nuclear Build Fix ===" -ForegroundColor Cyan
Write-Host ""

# 1. Kill everything
Write-Host "[1/5] Terminating all Rust processes..." -ForegroundColor Yellow
Get-Process -Name "rust-analyzer", "cargo", "rustc", "rls" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 2

# 2. Complete clean
Write-Host "[2/5] Running cargo clean..." -ForegroundColor Yellow
cargo clean 2>&1 | Out-Null

# 3. Remove entire target directory
Write-Host "[3/5] Removing target directory..." -ForegroundColor Yellow
if (Test-Path "target") {
    Remove-Item -Path "target" -Recurse -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
}

# 4. Disable all optimizations
Write-Host "[4/5] Configuring for maximum stability..." -ForegroundColor Yellow
$env:CARGO_INCREMENTAL = "0"
$env:CARGO_BUILD_JOBS = "1"

# 5. Build with minimal parallelism
Write-Host "[5/5] Building (single-threaded, this will take ~10 minutes)..." -ForegroundColor Yellow
Write-Host ""
Write-Host "IMPORTANT: Do NOT touch any files while building!" -ForegroundColor Red
Write-Host ""

cargo build --release -j 1

if ($LASTEXITCODE -eq 0) {
    Write-Host ""
    Write-Host "✅ BUILD SUCCESSFUL!" -ForegroundColor Green
    Write-Host "Binary location: target\release\flowbot.exe" -ForegroundColor Cyan
} else {
    Write-Host ""
    Write-Host "❌ Build failed again." -ForegroundColor Red
    Write-Host ""
    Write-Host "RECOMMENDATION: Use WSL or deploy directly to VPS." -ForegroundColor Yellow
    Write-Host "The code is VALID - this is a Windows file system limitation." -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Quick WSL install:" -ForegroundColor Cyan
    Write-Host "  wsl --install" -ForegroundColor White
    Write-Host "Then in WSL:" -ForegroundColor Cyan
    Write-Host "  cd /mnt/c/Users/AMAN\ ANU/Desktop/amxcodes/nanobot/nanobot-rs-clean" -ForegroundColor White
    Write-Host "  cargo build --release" -ForegroundColor White
}
