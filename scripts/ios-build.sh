#!/usr/bin/env bash
# Build the iOS app for the simulator: the Rust library and bindings, the Xcode project, the app.
#
#   scripts/ios-build.sh          build
#   scripts/ios-build.sh test     build, start a local cahoots-station, run the XCTest suites
#
# Needs Xcode, XcodeGen (`brew install xcodegen`) and the Rust iOS targets (see ios-framework.sh).
# DESTINATION overrides the simulator, default 'platform=iOS Simulator,name=iPhone 15'.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
DESTINATION=${DESTINATION:-platform=iOS Simulator,name=iPhone 15}

scripts/ios-framework.sh
# The project reads the library from ./target/ios. Point it at the real one.
if [ "$(cd "$TARGET" && pwd -P)" != "$(mkdir -p "$ROOT/target" && cd "$ROOT/target" && pwd -P)" ]; then
  rm -rf "$ROOT/target/ios"
  ln -s "$TARGET/ios" "$ROOT/target/ios"
fi
[ -f ios/Signing.xcconfig ] || cp ios/Signing.xcconfig.example ios/Signing.xcconfig
(cd ios && xcodegen generate --quiet)

if [ "${1:-}" != "test" ]; then
  xcodebuild -project ios/Cahoots.xcodeproj -scheme Cahoots -destination "$DESTINATION" build \
    | grep -E "error:|warning: .*\.swift|BUILD (SUCCEEDED|FAILED)" || true
  exit "${PIPESTATUS[0]}"
fi

# A station for UrlSessionHttpTests, on a free port, with an empty database, carrying channels.
cargo build -q -p cahoots-station --features objects,channels,gate
WORK=$(mktemp -d)
PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
"$TARGET/debug/cahoots-station" --listen "127.0.0.1:$PORT" --db "$WORK/station.db" \
  --posts-per-minute 0 --reads-per-minute 0 2>"$WORK/station.err" &
STATION=$!
trap 'kill $STATION 2>/dev/null; rm -rf "$WORK"' EXIT
# xcodebuild hands TEST_RUNNER_-prefixed variables to the tests without the prefix.
TEST_RUNNER_CAHOOTS_STATION="http://127.0.0.1:$PORT" \
  xcodebuild -project ios/Cahoots.xcodeproj -scheme Cahoots -destination "$DESTINATION" test \
  | grep -E "error:|Test Case .*(passed|failed|skipped)|Executed|TEST (SUCCEEDED|FAILED)" || true
exit "${PIPESTATUS[0]}"
