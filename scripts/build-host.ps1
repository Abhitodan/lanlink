# Build the Windows host (release).
# Output: target\release\lanlink-host.exe (window + tray app)
#         target\release\lanlink-host-console.exe (same core in a terminal)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
cargo build --release -p lanlink-host -p lanlink-host-console
Write-Host "Built: target\release\lanlink-host.exe and target\release\lanlink-host-console.exe"
