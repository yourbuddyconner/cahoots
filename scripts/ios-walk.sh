#!/usr/bin/env bash
# Hold a ceremony through the relay between two simulators, then chat, through the app's screens.
# A local cahoots-station is the relay. The UI test RelayWalkUITests drives each side; the two
# hand each other the offer and join codes through files, read off each screen and pasted.
#
#   scripts/ios-walk.sh [OUT]    screenshots and the words each side saw go to OUT
#
# Needs two booted simulators: "iPhone 15" and a clone of it named "iPhone 15 Walk B"
# (xcrun simctl clone "iPhone 15" "iPhone 15 Walk B"; xcrun simctl boot each).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
OUT=${1:-$(mktemp -d)}
rm -rf "$OUT"; mkdir -p "$OUT"
id() { xcrun simctl list devices | grep -F "    $1 (" | head -1 | sed -E 's/.*\(([0-9A-F-]{36})\).*/\1/'; }
A=$(id "iPhone 15"); B=$(id "iPhone 15 Walk B")
[ -n "$A" ] && [ -n "$B" ] || { echo "need simulators 'iPhone 15' and 'iPhone 15 Walk B'"; exit 1; }

cargo build -q -p cahoots-station
PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
"$TARGET/debug/cahoots-station" --listen "127.0.0.1:$PORT" --log "$OUT/station.jsonl" \
  --posts-per-minute 0 --reads-per-minute 0 2>"$OUT/station.log" &
STATION=$!
trap 'kill $STATION 2>/dev/null' EXIT

DD="$OUT/derived"
xcodebuild -project ios/Cahoots.xcodeproj -scheme Cahoots -destination "id=$A" -derivedDataPath "$DD" \
  build-for-testing >"$OUT/build.log" 2>&1
side() {
  TEST_RUNNER_CAHOOTS_WALK_DIR="$OUT" TEST_RUNNER_CAHOOTS_STATION="http://127.0.0.1:$PORT" \
    xcodebuild -project ios/Cahoots.xcodeproj -scheme Cahoots -derivedDataPath "$DD" -destination "id=$1" \
    test-without-building -only-testing:"CahootsUITests/RelayWalkUITests/$2" >"$OUT/$2.log" 2>&1
}
side "$A" testHost & H=$!
side "$B" testJoiner & J=$!
ok=0
wait $H || ok=1
wait $J || ok=1
grep -hE "Test Case .*(passed|failed)|error:" "$OUT/testHost.log" "$OUT/testJoiner.log" || true
echo "host:   $(cat "$OUT/host-words.txt" 2>/dev/null)"
echo "joiner: $(cat "$OUT/joiner-words.txt" 2>/dev/null)"
cmp -s "$OUT/host-words.txt" "$OUT/joiner-words.txt" && echo "the six words match" || { echo "the words differ"; ok=1; }
echo "relay entries: $(wc -l <"$OUT/station.jsonl")"
echo "out: $OUT"
exit $ok
