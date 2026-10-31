#!/usr/bin/env bash
# Walk a channel between two simulators through the app's screens, photographing each step:
# the owner starts a channel on the circle the relay walk left on "iPhone 15", shares the link,
# the reader on "iPhone 15 Walk B" opens it, asks to post, is let in, and posts. The UI test
# ChannelWalkUITests drives each side; the sides hand each other the link and the reader's name
# through files in OUT.
#
#   scripts/ios-channel-walk.sh WALK [OUT]
#
# WALK is the folder scripts/ios-walk.sh wrote (its station.db and the port in station.log are
# reused, so the circles' relay address still works). A station with the channel features runs
# on that port for the walk. Screenshots go to OUT.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
WALK=${1:?the folder scripts/ios-walk.sh wrote}
OUT=${2:-$(mktemp -d)}
rm -rf "$OUT"; mkdir -p "$OUT"
id() { xcrun simctl list devices | grep -F "    $1 (" | head -1 | sed -E 's/.*\(([0-9A-F-]{36})\).*/\1/'; }
A=$(id "iPhone 15"); B=$(id "iPhone 15 Walk B")
[ -n "$A" ] && [ -n "$B" ] || { echo "need simulators 'iPhone 15' and 'iPhone 15 Walk B'"; exit 1; }
PORT=$(sed -nE 's/.*listening on http:\/\/127\.0\.0\.1:([0-9]+).*/\1/p' "$WALK/station.log" | head -1)
[ -n "$PORT" ] || { echo "no port in $WALK/station.log"; exit 1; }
# The data folder each side's app used in the relay walk.
folder() {
  local c; c=$(xcrun simctl get_app_container "$1" network.cahoots.app data)
  ls -t "$c/Library/Application Support" | grep "^$2" | head -1
}
DATA_A=$(folder "$A" walk-host); DATA_B=$(folder "$B" walk-joiner)
[ -n "$DATA_A" ] && [ -n "$DATA_B" ] || { echo "run scripts/ios-walk.sh first: no walk data on the simulators"; exit 1; }

cargo build -q -p cahoots-station --features objects,channels,gate
"$TARGET/debug/cahoots-station" --listen "127.0.0.1:$PORT" --db "$WALK/station.db" \
  --posts-per-minute 0 --reads-per-minute 0 --channel-posts-per-minute 0 2>"$OUT/station.log" &
STATION=$!
trap 'kill $STATION 2>/dev/null' EXIT

DD="$OUT/derived"
(cd ios && xcodegen generate --quiet)
xcodebuild -project ios/Cahoots.xcodeproj -scheme Cahoots -destination "id=$A" -derivedDataPath "$DD" \
  build-for-testing >"$OUT/build.log" 2>&1
side() {
  TEST_RUNNER_CAHOOTS_WALK_DIR="$OUT" TEST_RUNNER_CAHOOTS_DATA="$3" \
    xcodebuild -project ios/Cahoots.xcodeproj -scheme Cahoots -derivedDataPath "$DD" -destination "id=$1" \
    test-without-building -only-testing:"CahootsUITests/ChannelWalkUITests/$2" >"$OUT/$2.log" 2>&1
}
side "$A" testOwner "$DATA_A" & O=$!
side "$B" testReader "$DATA_B" & R=$!
ok=0
wait $O || ok=1
wait $R || ok=1
grep -hE "Test Case .*(passed|failed)|error:" "$OUT/testOwner.log" "$OUT/testReader.log" || true
ls "$OUT"/*.png 2>/dev/null | wc -l | sed 's/^/screenshots: /'
echo "out: $OUT"
exit $ok
