#!/bin/sh
# Pairing on the bench, hands off: flash a bench build of the Pico 2 firmware, keep a circle
# with the laptop as the two cards under the badge, then pair the laptop to the badge as the
# app pairs a phone. The laptop runs the phone crate's own pairing, with no network, against
# the real chip through the firmware's APDU pass-through, and presses the badge's buttons over
# USB (the firmware's `bench` feature): from home, Both opens the list, and a note is left to
# go home by itself. The second tap is timed. Then the run unpairs and checks that the old key
# no longer opens the chip.
#
# The badge takes one phone. A badge with a phone paired already answers "pair a phone" with
# "unpair the current phone first", and the run stops there.
#
#   scripts/pair-bench.sh [--port /dev/cu.usbmodempico21] [--state /tmp/pbench] [--no-flash] [--forget | --keep]
#
# --forget, the default, forgets the circle the run kept, through the badge's own forget
# page, once the run is done, so the bench chip does not fill up: it holds five circles.
# --keep leaves it on the badge.
#
# Logs land in $STATE/pair.log. The run ends with "pair bench: ok".
#
# Only the badge and its paired phone read a circle's public object. So the run reads them in
# the slot's session while the laptop is paired, checks that a tap in no session is refused,
# and after the unpair reads what the badge says it wrote from the badge's log.
set -u
PORT=/dev/cu.usbmodempico21
STATE=/tmp/pbench
NAME=${NAME:-pair-$(date +%H%M)}
FLASH=1
FORGET=1
while [ $# -gt 0 ]; do
  case "$1" in
    --port) PORT=$2; shift 2;;
    --state) STATE=$2; shift 2;;
    --no-flash) FLASH=0; shift;;
    --forget) FORGET=1; shift;;
    --keep) FORGET=0; shift;;
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
PILE=""
fail() { say "FAILED: $*"; exit 1; }
# A pile left running would hold the port: whatever ends the run, it goes too.
trap '[ -n "$PILE" ] && kill $PILE 2>/dev/null' EXIT
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
# The ceremony takes well under a minute on the bench: three minutes means it hung.
i=0
while kill -0 $PILE 2>/dev/null; do
  i=$((i+1)); [ $i -gt 180 ] && fail "the ceremony did not end in three minutes, see $LOG"
  sleep 1
done
wait $PILE || fail "the ceremony did not complete, see $LOG"
PILE=""
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
chip pairing > "$STATE/pairing0.txt" || fail "reading the pairing object"
grep -Eq "^circles: (.* )?$SLOT( |$)" "$STATE/pairing0.txt" || fail "the pairing object does not name slot $SLOT"
chip public > "$STATE/public0.txt" || fail "a tap in no session read a public object, see $LOG"
grep -q "^slot $SLOT: refused in no session" "$STATE/public0.txt" || fail "the new circle's public object was not refused"
chip refusals --circle "$SLOT" || fail "an unpaired tap was allowed something, see $LOG"

say "opening pair a phone on the badge"
chip goto --entry "pair a phone" || fail "could not open pair a phone: $(tail -1 "$LOG")"
chip pairing > "$STATE/pairing.txt" || fail "reading the pairing object"
grep -q "^pairing: open" "$STATE/pairing.txt" || fail "the pairing object says no pairing is open"

# The first tap writes the inbox. Both reads it and shows the words. Only if they are the
# laptop's, Both again, once the question is drawn, pairs. Any other words are answered no.
# Then the second tap keeps every circle of the badge on the laptop's phone.
say "pairing as the app does, the laptop as the phone"
rm -rf "$STATE/phone"
chip pair --name "bench laptop" --state "$STATE/phone" > "$STATE/pair.txt" \
  || fail "the badge did not pair, see $LOG"
WORDS=$(sed -n 's/^words: //p' "$STATE/pair.txt")
PHONE=$(sed -n 's/^phone key: //p' "$STATE/pair.txt")
KEY=$(sed -n 's/^slot key: //p' "$STATE/pair.txt")
echo "$KEY" > "$STATE/slot.key"
[ "$(echo "$WORDS" | wc -w | tr -d ' ')" = 6 ] || fail "the laptop showed $(echo "$WORDS" | wc -w | tr -d ' ') words, not six"
[ -n "$KEY" ] || fail "the second tap kept no slot key"
say "the laptop showed $WORDS, the badge showed the same words and paired"
say "$(grep '^second tap: ' "$STATE/pair.txt")"

chip open --key "$KEY" || fail "the slot did not open with the paired key"
chip hmac --key "$KEY" --circle "$SLOT" --label "chat/gen" || fail "no HMAC with the root of slot $SLOT"
chip public --key "$KEY" > "$STATE/public1.txt" || fail "reading the public objects in the slot's session"
grep -q "^slot " "$STATE/public1.txt" || fail "no public objects after pairing"
if grep "^slot " "$STATE/public1.txt" | grep -v "device grant yes, phone key $PHONE," >/dev/null; then
  fail "a public object does not carry the grant and the phone's key"
fi
say "every public object carries the device grant and the phone's key"

say "unpairing on the badge"
chip goto --entry "phone" || fail "could not open the phone page"
chip press both --until "phone unpaired" >/dev/null || fail "the badge did not unpair"
# The handshake must be refused for the key, not fail for any other reason.
if "$HOST" chip open --key "$KEY" --serial "$PORT" >>"$LOG" 2>"$STATE/old-key.err"; then
  cat "$STATE/old-key.err" >>"$LOG"
  fail "the old key still opens the slot"
fi
cat "$STATE/old-key.err" >>"$LOG"
grep -q "the card does not open with this key" "$STATE/old-key.err" \
  || fail "the old key failed for another reason: $(tail -1 "$STATE/old-key.err")"
say "the old key no longer opens the slot"
# Nothing outside the badge reads a public object now: the badge's log says what it wrote.
grep -q "pairing: slot $SLOT public object written, [1-9] pending bumps, no phone" "$LOG" \
  || fail "the badge did not write slot $SLOT's public object with a pending bump and no phone"
if grep "pairing: slot [0-9]* public object written" "$LOG" | grep -v "pending bumps, no phone" >/dev/null; then
  fail "a public object still names the phone"
fi
say "every public object names no phone and holds a pending bump"
if [ "$FORGET" = 1 ]; then
  chip forget --circle "$NAME" >/dev/null || fail "could not forget $NAME"
  say "forgot $NAME"
fi
say "ok"
