# Quick incremental build fixer
# Run this if you get "os error 32" during development

Write-Host "🔄 Fixing incremental build lock..." -ForegroundColor Cyan

# Kill rust-analyzer and cargo processes
Get-Process | Where-Object { 
    $_.ProcessName -eq "rust-analyzer" -or 
    $_.ProcessName -eq "cargo" -or
    $_.ProcessName -eq "rustc"
} | Stop-Process -Force -ErrorAction SilentlyContinue

# Wait a moment
Start-Sleep -Seconds 2

# Remove incremental cache
if (Test-Path "target/debug/incremental") {
    Remove-Item -Path "target/debug/incremental" -Recurse -Force
    Write-Host "✓ Cleared incremental cache" -ForegroundColor Green
}

if (Test-Path "target/release/incremental") {
    Remove-Item -Path "target/release/incremental" -Recurse -Force
    Write-Host "✓ Cleared release incremental cache" -ForegroundColor Green
}

Write-Host "`n✅ Ready to build again!" -ForegroundColor Green
Write-Host "Run: .\build-windows.ps1" -ForegroundColor Cyan
