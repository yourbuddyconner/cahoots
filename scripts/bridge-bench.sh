#!/usr/bin/env bash
# A phone hosts a ceremony with a badge through the bridge. Two simulators are the phones: the
# host makes a circle for 3 people with 1 card in the pile, so the host is seat 1, the badge
# seat 2 and the joiner on the second simulator seat 3. A local cahoots-station is the relay.
# The laptop runs `cahoots-host bridge` with the badge on the cable, and hands the badge's join
# code to the host's simulator as a file, since a simulator has no camera. The run has one card.
#
#   scripts/bridge-bench.sh [--port /dev/cu.usbmodempico21] [--state /tmp/bridge-bench] [--no-flash]
#   scripts/bridge-bench.sh --sim [--state /tmp/bridge-bench]
#
# Without --sim, the badge is the board on --port, flashed with a bench build first unless
# --no-flash. The laptop presses its buttons over USB, as in scripts/bench.sh. With --sim, a
# simulated badge (`cahoots-host badge`) on a Unix socket stands in for the board.
#
# Needs two booted simulators, "iPhone 15" and "iPhone 15 Walk B" (see scripts/ios-walk.sh).
# Logs, screenshots and the codes the sides hand each other land in $STATE.
set -uo pipefail
PORT=/dev/cu.usbmodempico21
STATE=/tmp/bridge-bench
FLASH=1
SIM=0
while [ $# -gt 0 ]; do
  case "$1" in
    --port) PORT=$2; shift 2;;
    --state) STATE=$2; shift 2;;
    --no-flash) FLASH=0; shift;;
    --sim) SIM=1; shift;;
    *) echo "unknown argument $1"; exit 2;;
  esac
done
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
HOST=$TARGET/debug/cahoots-host
ELF=crates/rp2350/target/thumbv8m.main-none-eabihf/release/cahoots-rp2350
# Clear only what an earlier run wrote. The app build in $STATE/derived is kept.
mkdir -p "$STATE"
rm -rf "$STATE/walk" "$STATE/badge" "$STATE/station.db" "$STATE/station.db-shm" "$STATE/station.db-wal"
rm -f "$STATE"/*.log "$STATE/bridge.out" "$STATE/bridge.err"
WALK=$STATE/walk; mkdir -p "$WALK"
LOG=$STATE/bridge-bench.log
: > "$LOG"
PIDS=()
SOCK_DIR=
cleanup() {
  for p in ${PIDS[@]+"${PIDS[@]}"}; do kill "$p" 2>/dev/null; done
  [ -n "$SOCK_DIR" ] && rm -rf "$SOCK_DIR"
}
trap cleanup EXIT
say() { echo "bridge bench: $*" | tee -a "$LOG"; }
fail() { say "FAILED: $*"; exit 1; }

id() { xcrun simctl list devices | grep -F "    $1 (" | head -1 | sed -E 's/.*\(([0-9A-F-]{36})\).*/\1/'; }
A=$(id "iPhone 15"); B=$(id "iPhone 15 Walk B")
[ -n "$A" ] && [ -n "$B" ] || fail "need simulators 'iPhone 15' and 'iPhone 15 Walk B'"
xcrun simctl boot "$A" 2>/dev/null; xcrun simctl boot "$B" 2>/dev/null

wait_port() {
  i=0
  while [ ! -e "$PORT" ]; do i=$((i+1)); [ $i -gt 60 ] && fail "no port $PORT after 60 s"; sleep 1; done
}

flash() {
  picotool reboot -f -u >>"$LOG" 2>&1 || true
  i=0
  while [ ! -d /Volumes/RP2350 ]; do
    i=$((i+1))
    [ $i -eq 15 ] && say "no RP2350 drive yet: hold BOOTSEL while plugging the badge in"
    [ $i -gt 180 ] && fail "no RP2350 drive after 3 minutes"
    sleep 1
  done
  sleep 2
  picotool load -u -v -x -t elf "$ELF" >>"$LOG" 2>&1 || fail "picotool load"
  wait_port
  sleep 4
}

say "building the host, the station and the app"
cargo build -p cahoots-host -p cahoots-station >>"$LOG" 2>&1 || fail "host build"
scripts/ios-build.sh >>"$LOG" 2>&1 || fail "app build"
DD=$STATE/derived
xcodebuild -project ios/Cahoots.xcodeproj -scheme Cahoots -destination "id=$A" -derivedDataPath "$DD" \
  build-for-testing >"$STATE/build.log" 2>&1 || fail "app build for testing, see $STATE/build.log"
if [ "$SIM" = 0 ]; then
  (cd crates/rp2350 && cargo build --release --features bench) >>"$LOG" 2>&1 || fail "firmware build"
  if [ "$FLASH" = 1 ]; then
    say "flashing"
    flash
  fi
  wait_port
fi

RELAY_PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
"$TARGET/debug/cahoots-station" --listen "127.0.0.1:$RELAY_PORT" --db "$STATE/station.db" \
  --posts-per-minute 0 --reads-per-minute 0 2>"$STATE/station.log" &
PIDS+=($!)

say "the host makes a circle for 3 people with 1 card, the joiner waits for the host's code"
# Each side execs xcodebuild, so its process is the one the cleanup stops.
side() {
  export TEST_RUNNER_CAHOOTS_WALK_DIR="$WALK" TEST_RUNNER_CAHOOTS_STATION="http://127.0.0.1:$RELAY_PORT"
  export TEST_RUNNER_CAHOOTS_CARDS=$3
  exec xcodebuild -project ios/Cahoots.xcodeproj -scheme Cahoots -derivedDataPath "$DD" -destination "id=$1" \
    test-without-building -only-testing:"CahootsUITests/RelayWalkUITests/$2" >"$STATE/$2.log" 2>&1
}
side "$A" testHost 1 & H=$!
side "$B" testJoiner 0 & J=$!
PIDS+=($H $J)

i=0
while [ ! -s "$WALK/offer.txt" ]; do
  i=$((i+1)); [ $i -gt 600 ] && fail "the host showed no code in 10 minutes, see $STATE/testHost.log"
  kill -0 $H 2>/dev/null || fail "the host's test stopped before its code, see $STATE/testHost.log"
  sleep 1
done
say "the host's code is in $WALK/offer.txt"

BADGE_LOG=$STATE/badge.log
if [ "$SIM" = 1 ]; then
  # A socket path must be short, so it lives in its own folder under /tmp, not in $STATE.
  SOCK_DIR=$(mktemp -d /tmp/bridge-bench.XXXXXX)
  SOCK=$SOCK_DIR/badge.sock
  "$HOST" badge --state "$STATE/badge" --listen "$SOCK" --next - --name bench --pad-len 16384 \
    >"$BADGE_LOG" 2>&1 &
  BADGE=$!
  PIDS+=($BADGE)
  i=0
  while [ ! -S "$SOCK" ]; do
    i=$((i+1)); [ $i -gt 100 ] && fail "no simulated badge on $SOCK"
    kill -0 $BADGE 2>/dev/null || fail "the simulated badge stopped, see $BADGE_LOG"
    sleep 0.1
  done
  say "the simulated badge is on $SOCK"
  CABLE=(--sim "$SOCK")
  BRIDGE_ERR=$STATE/bridge.err
else
  # The badge's log lines come out of the bridge's port, so they are the badge's log here.
  CABLE=(--serial "$PORT")
  BRIDGE_ERR=$BADGE_LOG
  export CAHOOTS_BENCH=new-circle
fi
"$HOST" bridge "${CABLE[@]}" --offer-file "$WALK/offer.txt" >"$STATE/bridge.out" 2>"$BRIDGE_ERR" &
BRIDGE=$!
PIDS+=($BRIDGE)

# The badge's join code: the line after "Card in seat N.", handed to the host as card-N.txt.
if [ "$SIM" = 1 ]; then
  SILENT="the simulated badge said nothing in 90 s, see $BADGE_LOG"
else
  SILENT="the badge gave no code in 90 s, see $BADGE_LOG. The port may be deaf after the badge re-enumerated. Unplug it and plug it in again, or run without --no-flash"
fi
handed=0
i=0
until [ $handed = 1 ]; do
  seat=$(sed -nE 's/^Card in seat ([0-9]+)\..*/\1/p' "$STATE/bridge.out" | head -1)
  if [ -n "$seat" ]; then
    code=$(grep -A1 "^Card in seat $seat\." "$STATE/bridge.out" | sed -n 2p)
    if [ -n "$code" ]; then
      printf '%s' "$code" > "$WALK/card-$seat.tmp" && mv "$WALK/card-$seat.tmp" "$WALK/card-$seat.txt"
      say "the badge in seat $seat gave its code, handed to the host"
      handed=1
    fi
  fi
  if [ $handed = 0 ]; then
    kill -0 $BRIDGE 2>/dev/null || fail "the bridge stopped before the badge's code, see $BRIDGE_ERR"
    kill -0 $H 2>/dev/null || fail "the host's test stopped before the badge's code, see $STATE/testHost.log"
    i=$((i+1)); [ $i -gt 180 ] && fail "$SILENT"
    sleep 0.5
  fi
done

# The bridge keeps carrying until the badge is unplugged, so it should still be running here.
# One that stopped must have stopped well.
bridge_alive() {
  [ -n "$BRIDGE_STATUS" ] && return 1
  kill -0 $BRIDGE 2>/dev/null && return 0
  wait $BRIDGE; BRIDGE_STATUS=$?
  [ "$BRIDGE_STATUS" = 0 ] || fail "the bridge stopped before the badge kept its circle, see $BRIDGE_ERR"
  return 1
}
i=0
BRIDGE_STATUS=
until grep -q "\] words: " "$BADGE_LOG"; do
  i=$((i+1)); [ $i -gt 600 ] && fail "the badge showed no words in 5 minutes, see $BADGE_LOG"
  kill -0 $H 2>/dev/null || fail "the host's test stopped before the words, see $STATE/testHost.log"
  bridge_alive
  sleep 0.5
done
if [ "$SIM" = 1 ]; then
  # Keep the circle: Both on the keep prompt.
  echo B > "$STATE/badge/buttons.txt"
fi
ok=0
wait $H || ok=1
wait $J || ok=1
i=0
until grep -q "saved: " "$BADGE_LOG"; do
  i=$((i+1)); [ $i -gt 60 ] && { say "the badge did not keep the circle"; ok=1; break; }
  bridge_alive
  sleep 0.5
done
i=0
until grep -q "^every seat has signed" "$STATE/bridge.out"; do
  i=$((i+1)); [ $i -gt 60 ] && { say "the bridge never said every seat signed, see $STATE/bridge.out"; ok=1; break; }
  sleep 0.5
done
# The cleanup stops the bridge, as unplugging the badge would.

grep -hE "Test Case .*(passed|failed)|error:" "$STATE/testHost.log" "$STATE/testJoiner.log" | tee -a "$LOG"
BADGE_WORDS=$(grep -oE "\] words: .*" "$BADGE_LOG" | tail -1 | sed 's/^\] words: //')
HOST_WORDS=$(cat "$WALK/host-words.txt" 2>/dev/null)
JOINER_WORDS=$(cat "$WALK/joiner-words.txt" 2>/dev/null)
say "host, seat 1:   $HOST_WORDS"
say "badge, seat 2:  $BADGE_WORDS"
say "joiner, seat 3: $JOINER_WORDS"
say "relay entries: $(sqlite3 "$STATE/station.db" 'select count(*) from entries' 2>/dev/null || echo unknown)"
# The phones number their words ("1 buzz, 2 bench, ..."). The badge lists them.
plain() { echo "$1" | tr -d '0-9,' | tr -s ' ' | sed 's/^ //'; }
if [ -n "$BADGE_WORDS" ] && [ "$(plain "$HOST_WORDS")" = "$BADGE_WORDS" ] && [ "$(plain "$JOINER_WORDS")" = "$BADGE_WORDS" ]; then
  say "the six words match"
else
  say "the words differ"; ok=1
fi
say "logs and screenshots: $STATE"
[ $ok = 0 ] && say "ok" || say "FAILED"
exit $ok
