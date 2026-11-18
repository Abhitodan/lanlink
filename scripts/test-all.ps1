# Every automated check that runs on a development PC.
# Live tests need an unlocked interactive desktop.
$ErrorActionPreference = "Stop"
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $env:PATH = "$(Join-Path $env:USERPROFILE '.cargo\bin');$env:PATH"
}
Set-Location (Join-Path $PSScriptRoot "..")
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -- --test-threads=1
if (-not $env:ANDROID_HOME) { $env:ANDROID_HOME = Join-Path $env:LOCALAPPDATA "Android\Sdk" }
Push-Location android
try { .\gradlew.bat --no-daemon :app:testDebugUnitTest :app:lintDebug } finally { Pop-Location }
Write-Host "All automated checks passed."
