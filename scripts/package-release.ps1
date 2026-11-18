# Build and package a complete LANLink production release bundle under dist\.
# Produces:
#   dist\lanlink-<Version>-windows-x64.zip
#   dist\lanlink-<Version>-android.apk
#   dist\SHA256SUMS.txt
param(
    [string]$Version = "v0.1.0",
    [switch]$SkipRustNativeIfPresent
)
$ErrorActionPreference = "Stop"
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $env:PATH = "$(Join-Path $env:USERPROFILE '.cargo\bin');$env:PATH"
}
$root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $root

if (-not $env:ANDROID_HOME) { $env:ANDROID_HOME = Join-Path $env:LOCALAPPDATA "Android\Sdk" }
if (-not $env:ANDROID_NDK_HOME) { $env:ANDROID_NDK_HOME = Join-Path $env:ANDROID_HOME "ndk\28.2.13676358" }

$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root "target" }

Write-Host "==> Building Windows release binaries (lanlink-host.exe, lanlink-host-console.exe)..."
cargo build --release -p lanlink-host -p lanlink-host-console

$arm64So = Join-Path $root "android\app\src\main\jniLibs\arm64-v8a\liblanlink.so"
$arm32So = Join-Path $root "android\app\src\main\jniLibs\armeabi-v7a\liblanlink.so"
$x64So = Join-Path $root "android\app\src\main\jniLibs\x86_64\liblanlink.so"
if (-not ($SkipRustNativeIfPresent -and (Test-Path $arm64So) -and (Test-Path $arm32So) -and (Test-Path $x64So))) {
    Write-Host "==> Building Android native libraries (arm64-v8a, armeabi-v7a, x86_64)..."
    cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -P 29 -o android/app/src/main/jniLibs build --profile release-android -p lanlink-android
}

Write-Host "==> Building Android release APK..."
Push-Location (Join-Path $root "android")
try {
    .\gradlew.bat --no-daemon :app:assembleRelease
} finally {
    Pop-Location
}

$dist = Join-Path $root "dist"
if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
New-Item -ItemType Directory -Force -Path $dist | Out-Null

$winStage = Join-Path $dist "lanlink-$Version-windows-x64"
New-Item -ItemType Directory -Force -Path $winStage | Out-Null
Copy-Item (Join-Path $targetDir "release\lanlink-host.exe") $winStage
Copy-Item (Join-Path $targetDir "release\lanlink-host-console.exe") $winStage
Copy-Item "scripts\firewall-allow.ps1" $winStage
Copy-Item "README.md", "CHANGELOG.md", "LICENSE-MIT", "LICENSE-APACHE" $winStage

$winZip = Join-Path $dist "lanlink-$Version-windows-x64.zip"
Compress-Archive -Path "$winStage\*" -DestinationPath $winZip -Force
Remove-Item -Recurse -Force $winStage

$apkSource = Get-ChildItem "android\app\build\outputs\apk\release\*.apk" | Select-Object -First 1
if (-not $apkSource) { throw "Release APK not found under android\app\build\outputs\apk\release\" }
$apkDest = Join-Path $dist "lanlink-$Version-android.apk"
Copy-Item $apkSource.FullName $apkDest -Force

$sumsFile = Join-Path $dist "SHA256SUMS.txt"
$lines = @()
foreach ($file in @($winZip, $apkDest)) {
    $hash = (Get-FileHash -Path $file -Algorithm SHA256).Hash.ToLowerInvariant()
    $name = Split-Path $file -Leaf
    $lines += "$hash  $name"
}
$lines | Set-Content -Path $sumsFile -Encoding utf8NoBOM

Write-Host "==> Release bundle ready in $dist`:"
Get-ChildItem $dist | Select-Object Name, Length
Get-Content $sumsFile
