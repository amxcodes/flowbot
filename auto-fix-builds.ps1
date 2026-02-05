# auto-fix-builds.ps1
# Automatically applies best-effort fixes for os error 32

Write-Host "=== Auto-Fix for OS Error 32 ===" -ForegroundColor Cyan

# 1. Kill locking processes
Write-Host "[1/4] Killing rust-analyzer and cargo processes..." -ForegroundColor Yellow
Get-Process -Name "rust-analyzer", "cargo", "rustc" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1

# 2. Clear target directory
Write-Host "[2/4] Cleaning build cache..." -ForegroundColor Yellow
if (Test-Path "target") {
    try {
        cargo clean 2>&1 | Out-Null
    } catch {}
}

# 3. Disable incremental builds (reduces file churn)
Write-Host "[3/4] Disabling incremental builds..." -ForegroundColor Yellow
$env:CARGO_INCREMENTAL = "0"

# 4. Run build with fewer parallel jobs
Write-Host "[4/4] Running single-threaded build (safer)..." -ForegroundColor Yellow
Write-Host ""
Write-Host "TIP: Close VS Code/Cursor for best results." -ForegroundColor Cyan
Write-Host "Building with 'cargo build -j 1'..."
Write-Host ""

cargo build -j 1

if ($LASTEXITCODE -eq 0) {
    Write-Host ""
    Write-Host "✅ Build succeeded!" -ForegroundColor Green
} else {
    Write-Host ""
    Write-Host "❌ Build failed. Next steps:" -ForegroundColor Red
    Write-Host "1. Add Windows Defender exclusion (see WINDOWS_BUILD_FIX.md)" -ForegroundColor Cyan
    Write-Host "2. Disable rust-analyzer in your editor settings" -ForegroundColor Cyan
}
