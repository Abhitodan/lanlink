#!/usr/bin/env bash
# Build ios/build/LanLinkCore.xcframework (device + simulator). Run on macOS
# with Xcode and the three iOS Rust targets installed.
set -euo pipefail

cd "$(dirname "$0")/.."

DEVICE=aarch64-apple-ios
SIM_ARM=aarch64-apple-ios-sim
SIM_X64=x86_64-apple-ios
LIB=liblanlink_ios.a
OUT=ios/build

for t in "$DEVICE" "$SIM_ARM" "$SIM_X64"; do
  cargo build --profile release-ios --target "$t" -p lanlink-ios
done

rm -rf "$OUT"
mkdir -p "$OUT/sim"
lipo -create \
  "target/$SIM_ARM/release-ios/$LIB" \
  "target/$SIM_X64/release-ios/$LIB" \
  -output "$OUT/sim/$LIB"

xcodebuild -create-xcframework \
  -library "target/$DEVICE/release-ios/$LIB" -headers ios/native-bridge/include \
  -library "$OUT/sim/$LIB" -headers ios/native-bridge/include \
  -output "$OUT/LanLinkCore.xcframework"
