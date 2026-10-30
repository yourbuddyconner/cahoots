#!/usr/bin/env bash
# Build the iOS app's Rust library: `crates/phone` for iPhones and the simulator, the Swift
# bindings, and one XCFramework the Xcode project links.
#
#   scripts/ios-framework.sh
#
# Needs macOS with Xcode and both Rust targets:
#   rustup target add aarch64-apple-ios aarch64-apple-ios-sim
# Writes $TARGET/ios/CahootsCore.xcframework and $TARGET/ios/CahootsCore.swift beside it, where
# $TARGET is $CARGO_TARGET_DIR or ./target.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
OUT=$TARGET/ios
LIB=libcahoots_phone

cargo build -p cahoots-phone --lib --release --target aarch64-apple-ios --target aarch64-apple-ios-sim

rm -rf "$OUT/bindings" "$OUT/headers" "$OUT/CahootsCore.xcframework"
mkdir -p "$OUT/bindings" "$OUT/headers"
# The interface is read from the compiled library's metadata, so the bindings match it exactly.
cargo run -p cahoots-phone --bin uniffi-bindgen -- generate \
  --library "$TARGET/aarch64-apple-ios/release/$LIB.dylib" \
  --language swift --out-dir "$OUT/bindings"

cp "$OUT"/bindings/*.h "$OUT/headers/"
# Xcode looks for a module map by this name inside the framework's headers.
cp "$OUT"/bindings/*.modulemap "$OUT/headers/module.modulemap"

xcodebuild -create-xcframework \
  -library "$TARGET/aarch64-apple-ios/release/$LIB.a" -headers "$OUT/headers" \
  -library "$TARGET/aarch64-apple-ios-sim/release/$LIB.a" -headers "$OUT/headers" \
  -output "$OUT/CahootsCore.xcframework"
cp "$OUT"/bindings/*.swift "$OUT/"

echo "built $OUT/CahootsCore.xcframework"
ls "$OUT/CahootsCore.xcframework"
