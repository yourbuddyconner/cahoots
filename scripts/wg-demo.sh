#!/usr/bin/env bash
# Two hosts keyed only from a cahoot, in Linux network namespaces: announce on the feed, issue
# and accept a pre-shared key over the feed, ping through the tunnel, rotate, refuse a replay.
#
#   sudo -v && scripts/wg-demo.sh
#
# Needs Linux with wireguard-tools and iproute2 (the kernel module is in mainline). The relay
# runs inside one namespace; nothing leaves the machine. On macOS use two wireguard-go
# interfaces by hand; the cahoots-wg commands are the same.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d)
BIN=$ROOT/target/debug
NS_A=cahoot-a; NS_B=cahoot-b
cleanup() {
  set +e
  [ -n "${STATION:-}" ] && sudo kill "$STATION" 2>/dev/null
  sudo ip netns del $NS_A 2>/dev/null; sudo ip netns del $NS_B 2>/dev/null
  sudo rm -rf "$WORK"
}
trap cleanup EXIT

echo "== build"
(cd "$ROOT" && cargo build -q -p cahoots-station -p cahoots-wireguard -p cahoots-chat --features cahoots-chat/cli)

echo "== ceremony (dev shortcut, two members)"
"$BIN/cahoots-chat" local-ceremony --members 2 --out "$WORK/circles" --name wg-demo >/dev/null
CIRCLE=$(ls "$WORK/circles/member-0" | grep -v '^key$' | head -1)
A="$BIN/cahoots-wg --dir $WORK/circles/member-0"
B="$BIN/cahoots-wg --dir $WORK/circles/member-1"
ADDR_A=$($A address --circle "$CIRCLE" | cut -d' ' -f1)
ADDR_B=$($B address --circle "$CIRCLE" | cut -d' ' -f1)
echo "circle $CIRCLE  a=$ADDR_A  b=$ADDR_B"

echo "== two namespaces joined by a cable"
sudo ip netns add $NS_A; sudo ip netns add $NS_B
sudo ip link add va type veth peer name vb
sudo ip link set va netns $NS_A; sudo ip link set vb netns $NS_B
sudo ip -n $NS_A addr add 192.168.77.1/24 dev va; sudo ip -n $NS_A link set va up; sudo ip -n $NS_A link set lo up
sudo ip -n $NS_B addr add 192.168.77.2/24 dev vb; sudo ip -n $NS_B link set vb up; sudo ip -n $NS_B link set lo up

echo "== relay (untrusted) inside namespace a"
sudo ip netns exec $NS_A "$BIN/cahoots-station" --listen 192.168.77.1:8080 --db "$WORK/station.db" >/dev/null 2>&1 &
STATION=$!
RELAY=http://192.168.77.1:8080
for _ in $(seq 1 50); do sudo ip netns exec $NS_A curl -sf "$RELAY/healthz" >/dev/null 2>&1 && break; sleep 0.1; done

echo "== wireguard interfaces, private keys made locally"
for h in a b; do
  ns=$([ $h = a ] && echo $NS_A || echo $NS_B)
  wg genkey > "$WORK/$h.key"; wg pubkey < "$WORK/$h.key" > "$WORK/$h.pub"
  sudo ip -n $ns link add wg0 type wireguard
  sudo ip netns exec $ns wg set wg0 private-key "$WORK/$h.key" listen-port 51820
done
sudo ip -n $NS_A addr add "$ADDR_A" dev wg0; sudo ip -n $NS_A link set wg0 up
sudo ip -n $NS_B addr add "$ADDR_B" dev wg0; sudo ip -n $NS_B link set wg0 up

echo "== announce: public keys and endpoints go to the feed, sealed by the pad"
sudo -E ip netns exec $NS_A $A announce --circle "$CIRCLE" --relay $RELAY --public-key "$(cat "$WORK/a.pub")" --endpoint 192.168.77.1:51820
sudo -E ip netns exec $NS_B $B announce --circle "$CIRCLE" --relay $RELAY --public-key "$(cat "$WORK/b.pub")" --endpoint 192.168.77.2:51820
sudo -E ip netns exec $NS_A $A peers --circle "$CIRCLE" --relay $RELAY
sudo -E ip netns exec $NS_B $B peers --circle "$CIRCLE" --relay $RELAY

echo "== key: a issues to seat 2 over the feed, b accepts from the feed, both install with wg set"
sudo -E ip netns exec $NS_A $A issue --circle "$CIRCLE" --to 2 --relay $RELAY --apply wg0
sudo -E ip netns exec $NS_B $B accept --circle "$CIRCLE" --relay $RELAY --apply wg0

echo "== ping through the tunnel"
sudo ip netns exec $NS_A ping -c 3 -W 2 "${ADDR_B%/*}"
PSK_LINE=$(sudo ip netns exec $NS_A wg show wg0 preshared-keys)
echo "$PSK_LINE" | grep -qv '(none)' && echo "preshared key present on a"
sudo ip netns exec $NS_B wg show wg0 preshared-keys | grep -qv '(none)' && echo "preshared key present on b"

echo "== rotate: a new key for the same pair, over the feed"
sudo -E ip netns exec $NS_A $A issue --circle "$CIRCLE" --to 2 --relay $RELAY --apply wg0
sudo -E ip netns exec $NS_B $B accept --circle "$CIRCLE" --relay $RELAY --apply wg0
sudo ip netns exec $NS_A ping -c 3 -W 2 "${ADDR_B%/*}"
echo "rotated and still reachable"

echo "== replay: the first handle again is refused on b"
FIRST=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['accepted'][0])" "$WORK/circles/member-1/$CIRCLE/wg.json")
if sudo -E ip netns exec $NS_B $B accept --circle "$CIRCLE" --handle "$FIRST" --raw 2>"$WORK/replay.err"; then
  echo "replay was NOT refused"; exit 1
fi
grep -q "already spent" "$WORK/replay.err" && echo "replay refused: $(cat "$WORK/replay.err")"

echo "== budget"
$A circles
echo "OK: two hosts, keys from the cahoot only, tunnel up, rotated, replay refused"
