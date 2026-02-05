# Windows Build Lock Fix (OS Error 32)

## Root Cause
Windows file locking occurs when:
1. **Antivirus** scans `.exe`/`.dll` files during build
2. **rust-analyzer** (VS Code/Cursor) holds locks on intermediate files
3. **cargo** itself has lingering child processes

## Permanent Solutions

### 1. Add Windows Defender Exclusion (RECOMMENDED)
```powershell
# Run as Administrator
Add-MpPreference -ExclusionPath "C:\Users\AMAN ANU\Desktop\amxcodes\nanobot\nanobot-rs-clean"
Add-MpPreference -ExclusionPath "$env:USERPROFILE\.cargo"
```

### 2. Disable rust-analyzer
In VS Code/Cursor, add to `.vscode/settings.json`:
```json
{
  "rust-analyzer.enable": false
}
```
Then reload window (`Ctrl+Shift+P` → "Reload Window").

### 3. Use ramdisk (Advanced)
Install [ImDisk](https://www.ltr-data.se/opencode.html/#ImDisk) and set:
```toml
# .cargo/config.toml
[build]
target-dir = "R:/nanobot-target"  # R: is your ramdisk
```

## Quick Workarounds

### A. Clean Build (Nuclear Option)
```powershell
powershell -File fix-build-locks.ps1
cargo clean
cargo build
```

### B. Single-Threaded Build
```powershell
cargo build -j 1  # Slower, but less locking
```

### C. Incremental Build Off
```powershell
$env:CARGO_INCREMENTAL = "0"
cargo build
```

## Verification
If none work, check:
```powershell
handle.exe nanobot-rs-clean  # from Sysinternals
# Shows exactly which process is locking files
```
