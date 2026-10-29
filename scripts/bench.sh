#!/bin/sh
# The badge bench, hands off: flash a bench build of the Pico 2 firmware, run a ceremony with
# the laptop as the two cards under the badge, then attest the seat key and sign with it
# through the SSH agent. The laptop presses the badge's buttons over USB (the firmware's
# `bench` feature), so nobody has to hold the board.
#
#   scripts/bench.sh [--port /dev/cu.usbmodempico21] [--state /tmp/bench] [--no-flash] [--github]
#
# --github adds the seat key to the GitHub account `gh` is signed in to and runs
# `ssh -T git@github.com`. Logs land in $STATE/bench.log.
set -u
PORT=/dev/cu.usbmodempico21
STATE=/tmp/bench
NAME=${NAME:-bench-$(date +%H%M)}
FLASH=1
GITHUB=0
while [ $# -gt 0 ]; do
  case "$1" in
    --port) PORT=$2; shift 2;;
    --state) STATE=$2; shift 2;;
    --no-flash) FLASH=0; shift;;
    --github) GITHUB=1; shift;;
    *) echo "unknown argument $1"; exit 2;;
  esac
done
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
HOST=target/debug/cahoots-host
ELF=crates/rp2350/target/thumbv8m.main-none-eabihf/release/cahoots-rp2350
mkdir -p "$STATE"
LOG="$STATE/bench.log"
: > "$LOG"
say() { echo "bench: $*" | tee -a "$LOG"; }
fail() { say "FAILED: $*"; exit 1; }

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

say "building the host and a bench build of the firmware"
cargo build -p cahoots-host >>"$LOG" 2>&1 || fail "host build"
(cd crates/rp2350 && cargo build --release --features bench) >>"$LOG" 2>&1 || fail "firmware build"

if [ "$FLASH" = 1 ]; then
  say "flashing"
  flash
fi


say "ceremony as circle $NAME: the laptop is two cards under the badge; state in $STATE/pile"
rm -rf "$STATE/pile"
# The badge is never rebooted by picotool once it is up: on macOS the port handle opened
# after such a reboot has been seen to deliver nothing until the device enumerates again.
# A BOOTSEL flash cycle is such a re-enumeration, so a deaf port is cured by flashing.
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
  say "the port is deaf; flashing to re-enumerate the badge (try $try)"
  flash
done
wait $PILE || fail "the ceremony did not complete; see $LOG"
WORDS=$(grep -oE "words from the record: .*|check words: .*" "$LOG" | tail -1)
say "ceremony done. $WORDS"
grep -q "seat key made in the chip" "$LOG" && say "the seat key was made in the chip" || say "NOTE: no chip key line in the log"

say "attestation"
CAHOOTS_BENCH=serve "$HOST" attest --serial "$PORT" --circle "$NAME" --json >"$STATE/attest.json" 2>>"$LOG" && say "attest: $(head -c 300 "$STATE/attest.json")" || say "attest did not verify (expected until NXP's root is in the tree); output in $LOG"

say "ssh agent"
SOCK="$STATE/agent.sock"
rm -f "$SOCK"
CAHOOTS_BENCH=serve "$HOST" agent --serial "$PORT" --circle "$NAME" --socket "$SOCK" >"$STATE/agent.env" 2>>"$LOG" &
AGENT=$!
i=0; while [ ! -S "$SOCK" ] && [ $i -lt 120 ]; do sleep 1; i=$((i+1)); done
sleep 2
export SSH_AUTH_SOCK="$SOCK"
ssh-add -L >"$STATE/keys.txt" 2>>"$LOG" || fail "ssh-add -L"
say "keys: $(cat "$STATE/keys.txt")"
grep " cahoots:$NAME" "$STATE/keys.txt" > "$STATE/bench.pub" || fail "no bench key in the agent"
say "signing a file with the chip's key"
printf 'Release notes for the bench\n\nSigned on the badge.\n' > "$STATE/msg.txt"
rm -f "$STATE/msg.txt.sig"
if "$HOST" sign --file "$STATE/msg.txt" --circle "$NAME" >>"$LOG" 2>&1; then
  printf 'bench namespaces="file" %s\n' "$(cut -d' ' -f1,2 "$STATE/bench.pub")" > "$STATE/allowed"
  if ssh-keygen -Y verify -f "$STATE/allowed" -I bench -n file -s "$STATE/msg.txt.sig" < "$STATE/msg.txt" >>"$LOG" 2>&1; then
    say "a file signature from the chip verifies with ssh-keygen -Y verify"
  else
    say "FAILED: the file signature did not verify"
  fi
else
  say "FAILED: cahoots-host sign through the agent; see $LOG"
fi
say "signing a git commit with the chip's key"
REPO="$STATE/repo"; rm -rf "$REPO"; mkdir -p "$REPO"
( cd "$REPO" && git init -q && git config user.name "Bench" && git config user.email bench@example.com \
  && git config gpg.format ssh && git config user.signingkey "key::$(cut -d' ' -f1,2 "$STATE/bench.pub")" \
  && git config gpg.ssh.program "$ROOT/target/debug/cahoots-host-git-sign" && git config commit.gpgsign true \
  && printf 'bench@example.com namespaces="git" %s\n' "$(cut -d' ' -f1,2 "$STATE/bench.pub")" > "$STATE/allowed_git" \
  && git config gpg.ssh.allowedSignersFile "$STATE/allowed_git" \
  && echo hello > hello.txt && git add hello.txt && git commit -q -m "A commit signed on the badge" ) >>"$LOG" 2>&1 \
  && ( cd "$REPO" && git log --show-signature -1 2>&1 | grep -q 'Good "git" signature' ) \
  && say "a git commit signed by the chip verifies" || say "FAILED: the git commit was not signed or did not verify; see $LOG"
if [ "$GITHUB" = 1 ]; then
  say "github: adding the key and logging in"
  gh ssh-key add "$STATE/bench.pub" --title "cahoots bench $(date +%F)" --type authentication >>"$LOG" 2>&1 || say "gh ssh-key add (authentication) failed or exists"
  # Needs the admin:ssh_signing_key scope: gh auth refresh -h github.com -s admin:ssh_signing_key
  gh ssh-key add "$STATE/bench.pub" --title "cahoots bench $(date +%F)" --type signing >>"$LOG" 2>&1 || say "gh ssh-key add (signing) failed or exists; for verified commits on GitHub, run: gh auth refresh -h github.com -s admin:ssh_signing_key, then add it"
  ssh -o StrictHostKeyChecking=accept-new -T git@github.com >>"$LOG" 2>&1
  tail -1 "$LOG" | grep -q "successfully authenticated" && say "ssh -T git@github.com: authenticated with the chip's key" || say "FAILED: ssh -T git@github.com; see $LOG"
fi
kill $AGENT 2>/dev/null
say "done. log: $LOG"
