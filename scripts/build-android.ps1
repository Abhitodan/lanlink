# Build the Android native library (arm64-v8a + x86_64) and the APKs.
# Requires: Android SDK + NDK 28.2, JDK 17+, cargo-ndk.
param([switch]$Release)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
if (-not $env:ANDROID_HOME) { $env:ANDROID_HOME = Join-Path $env:LOCALAPPDATA "Android\Sdk" }
if (-not $env:ANDROID_NDK_HOME) { $env:ANDROID_NDK_HOME = Join-Path $env:ANDROID_HOME "ndk\28.2.13676358" }
cargo ndk -t arm64-v8a -t x86_64 -P 29 -o android/app/src/main/jniLibs build --profile release-android -p lanlink-android
Push-Location android
try {
    if ($Release) { .\gradlew.bat --no-daemon :app:assembleRelease }
    else { .\gradlew.bat --no-daemon :app:assembleDebug }
} finally { Pop-Location }
