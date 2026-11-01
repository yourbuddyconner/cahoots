#!/bin/sh
# Pairing on the bench, hands off: flash a bench build of the Pico 2 firmware, keep a circle
# with the laptop as the two cards under the badge, then pair the laptop to the badge as a
# phone would. The laptop runs the phone's own chip client against the real chip through the
# firmware's APDU pass-through, and presses the badge's buttons over USB (the firmware's
# `bench` feature). Then it unpairs and checks that the old key no longer opens the chip.
#
#   scripts/pair-bench.sh [--port /dev/cu.usbmodempico21] [--state /tmp/pbench] [--no-flash]
#
# Logs land in $STATE/pair.log. The run ends with "pair bench: ok".
set -u
PORT=/dev/cu.usbmodempico21
STATE=/tmp/pbench
NAME=${NAME:-pair-$(date +%H%M)}
FLASH=1
while [ $# -gt 0 ]; do
  case "$1" in
    --port) PORT=$2; shift 2;;
    --state) STATE=$2; shift 2;;
    --no-flash) FLASH=0; shift;;
    *) echo "unknown argument $1"; exit 2;;
  esac
done
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
HOST=${CARGO_TARGET_DIR:-$ROOT/target}/debug/cahoots-host
mkdir -p "$STATE"
LOG="$STATE/pair.log"
: > "$LOG"
say() { echo "pair bench: $*" | tee -a "$LOG"; }
fail() { say "FAILED: $*"; exit 1; }
# One `cahoots-host chip` command: its APDUs and the badge's log go to the log, what it
# prints to stdout and the log both, and its exit status is the command's.
chip() {
  out=$("$HOST" chip "$@" --serial "$PORT" 2>>"$LOG")
  rc=$?
  [ -n "$out" ] && printf '%s\n' "$out" | tee -a "$LOG"
  return $rc
}

wait_port() {
  i=0
  while [ ! -e "$PORT" ]; do i=$((i+1)); [ $i -gt 60 ] && fail "no port $PORT after 60 s"; sleep 1; done
}

# The firmware is its own cargo project. Inside a git worktree, cargo walks up past the
# worktree to the main checkout's workspace, so the build runs from a copy in $STATE.
FW="$ROOT/crates/rp2350"
if [ "$(git rev-parse --git-dir)" != "$(git rev-parse --git-common-dir)" ]; then
  mkdir -p "$STATE/fw"
  rsync -a --delete --exclude target --exclude .git --exclude ios --exclude node_modules "$ROOT/" "$STATE/fw/" || fail "copying the tree for the firmware build"
  FW="$STATE/fw/crates/rp2350"
fi
ELF="$FW/target/thumbv8m.main-none-eabihf/release/cahoots-rp2350"

flash() {
  picotool reboot -f -u >>"$LOG" 2>&1 || true
  # picotool loads over USB in BOOTSEL mode, so the RP2350 drive need not be mounted.
  i=0
  until picotool info >/dev/null 2>&1; do
    i=$((i+1))
    [ $i -eq 15 ] && say "the board is not in BOOTSEL yet: hold BOOTSEL while plugging the badge in"
    [ $i -gt 180 ] && fail "the board did not come up in BOOTSEL after 3 minutes"
    sleep 1
  done
  picotool load -u -v -x -t elf "$ELF" >>"$LOG" 2>&1 || fail "picotool load"
  wait_port
  sleep 4
}

say "building the host and a bench build of the firmware"
cargo build -p cahoots-host >>"$LOG" 2>&1 || fail "host build"
(cd "$FW" && env -u CARGO_TARGET_DIR cargo build --release --features bench) >>"$LOG" 2>&1 || fail "firmware build"

if [ "$FLASH" = 1 ]; then
  say "flashing"
  flash
fi

say "ceremony as circle $NAME: the laptop is two cards under the badge"
rm -rf "$STATE/pile"
try=0
while :; do
  try=$((try+1))
  CAHOOTS_BENCH=new-circle CAHOOTS_FRAMES=1 "$HOST" pile --cards 3 --serial "$PORT" --name "$NAME" --state "$STATE/pile" >>"$LOG" 2>&1 &
  PILE=$!
  i=0; live=0
  while [ $i -lt 20 ]; do
    grep -q "^\[board\] screen:" "$LOG" && { live=1; break; }
    kill -0 $PILE 2>/dev/null || break
    sleep 1; i=$((i+1))
  done
  if [ $live = 1 ]; then break; fi
  kill $PILE 2>/dev/null; wait $PILE 2>/dev/null
  [ $try -ge 3 ] && fail "the badge said nothing on $PORT after three tries"
  say "the port is deaf, flashing to re-enumerate the badge (try $try)"
  flash
done
wait $PILE || fail "the ceremony did not complete, see $LOG"
# The cards on the laptop are done before the badge asks to keep the circle, so the pile
# may be gone before the badge's keep prompt: press Both for it here.
if ! grep -q "chip: slot [0-9]* kept" "$LOG"; then
  if grep -q "^\[board\] words: " "$LOG"; then AFTER=""; else AFTER="words: "; fi
  chip press both ${AFTER:+--after "$AFTER"} --until " kept" >/dev/null || fail "the badge did not keep the circle"
fi
SLOT=$(sed -n 's/.*chip: slot \([0-9]*\) kept.*/\1/p' "$LOG" | tail -1)
[ -n "$SLOT" ] || fail "the circle's keys are not in the chip"
say "circle kept, its keys in the chip at slot $SLOT"

say "an unpaired tap, in no session"
chip public > "$STATE/public0.txt" || fail "reading the public objects"
grep -q "^slot $SLOT: " "$STATE/public0.txt" || fail "the new circle has no public object"
chip refusals --circle "$SLOT" || fail "an unpaired tap was allowed something, see $LOG"

say "opening pair a phone on the badge"
chip goto --entry "pair a phone" || fail "could not open pair a phone"
chip pairing > "$STATE/pairing.txt" || fail "reading the pairing object"
grep -q "^pairing: open" "$STATE/pairing.txt" || fail "the pairing object says no pairing is open"

KEY=$(openssl rand -hex 16)
echo "$KEY" > "$STATE/slot.key"
say "writing the inbox as the bench laptop"
chip inbox --name "bench laptop" --key "$KEY" --state "$STATE" > "$STATE/inbox.txt" || fail "writing the inbox"
WORDS=$(sed -n 's/^words: //p' "$STATE/inbox.txt")
PHONE=$(sed -n 's/^phone key: //p' "$STATE/inbox.txt")
say "the laptop shows: $WORDS"

chip press both --until "pairing: " > "$STATE/ask.txt" || fail "the badge did not read the inbox"
grep -q "words $WORDS\$" "$STATE/ask.txt" || fail "the badge's words are not the laptop's: $(cat "$STATE/ask.txt")"
say "the badge shows the same words"
chip press both --until "paired with" >/dev/null || fail "the badge did not pair"
say "paired"

chip open --key "$KEY" || fail "the slot did not open with the paired key"
chip hmac --key "$KEY" --circle "$SLOT" --label "chat/gen" || fail "no HMAC with the root of slot $SLOT"
chip public > "$STATE/public1.txt" || fail "reading the public objects after pairing"
grep -q "device grant yes, phone key $PHONE" "$STATE/public1.txt" || fail "the public object does not carry the grant and the phone's key"
say "the public object carries the device grant and the phone's key"

say "unpairing on the badge"
chip goto --entry "phone" || fail "could not open the phone page"
chip press both --until "phone unpaired" >/dev/null || fail "the badge did not unpair"
if chip open --key "$KEY"; then fail "the old key still opens the slot"; fi
say "the old key no longer opens the slot"
chip public > "$STATE/public2.txt" || fail "reading the public objects after unpairing"
grep -q "phone key none, pending bumps [1-9]" "$STATE/public2.txt" || fail "the public object does not hold a pending bump"
say "the public object names no phone and holds a pending bump"
say "ok"
