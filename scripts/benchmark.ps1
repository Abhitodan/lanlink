# Capture + encode measurement on this PC (no phone needed). Move windows
# or play a video during the run so frames are produced.
param([int]$Seconds = 10, [int]$Fps = 60, [int]$Width = 1920, [int]$Kbps = 12000)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
New-Item -ItemType Directory -Force benchmarks\out | Out-Null
cargo run --release -p encoder --example capture_encode -- --seconds $Seconds --fps $Fps --width $Width --kbps $Kbps --out benchmarks\out\bench.h264
