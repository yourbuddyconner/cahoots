# Host simulator

Runs the real `cahoots-hal` device logic on a laptop with stand-ins for the hardware:

| Hardware | Simulator |
|---|---|
| TRNG | OS CSPRNG (`getrandom`), behind the SP 800-90B health tests. Or `--seed N` for a reproducible demo. |
| Touch link (UART) | A Unix-domain socket to each neighbour |
| Pad region (NOR flash or RRAM) | `pad.bin`, 4 KiB erase unit, erase = overwrite zeros + sync |
| Ledger region | `ledger.bin`, 64 KiB of `0xFF`, 16-byte append-only records |
| 200x200 e-paper | `display.png`, re-rendered on every status change |

A simulated badge is a `CSPRNG` source, not a hardware TRNG. Its circles are labelled `CSPRNG`.

## Four devices on one laptop

Each device listens on its own socket and connects to the next one, so every device has a socket to both neighbours. Frames travel both ways over every socket, as they do through the pads in a pile, so the ring may be closed or left open. Start all four within 30 seconds of each other.

```sh
cargo build -p cahoots-host
H=target/debug/cahoots-host
for i in 0 1 2 3; do
  $H ceremony --index $i --n 4 --pad-len 65536 --name sf-meetup \
    --listen /tmp/cahoots$i.sock --next /tmp/cahoots$(( (i + 1) % 4 )).sock \
    --state /tmp/cahoots-dev$i &
done
wait
```

Every device prints the same six check words and writes its state directory. Open `/tmp/cahoots-dev0/display.png` to see what the badge would show.

Send a message from device 0 and read it on device 3:

```sh
MSG=$($H send --state /tmp/cahoots-dev0 --text "hello from the ring")
$H recv --state /tmp/cahoots-dev3 --message "$MSG"
$H recv --state /tmp/cahoots-dev3 --message "$MSG"   # second time: refused as reuse
$H status --state /tmp/cahoots-dev0                   # spent ranges and remaining bytes
xxd /tmp/cahoots-dev0/pad.bin | head -4               # the spent range reads as zeros
```

The base64url message can be pasted into the browser app (Read a message) or posted to the Numbers Station. Any other device that holds the same pad decrypts it.

## What the integration test covers

`tests/simulator.rs` runs four devices in threads over socket pairs with deterministic entropy, checks that all four check words match the words computed directly from the XOR of the contributions, sends a message from device 0 to device 3, confirms both pad files show the spent range erased on disk and the rest intact, confirms a replay is refused, and reopens a device from disk to confirm the ledger replays.

## Ring driver notes

Writes on the socket link never block. Bytes that do not fit the socket buffer are queued and flushed on every later call. The ring driver keeps reading while it flushes its last forwarded frames before it finishes, so a device that completes first never strands its neighbour. Both behaviours are what a UART driver with a DMA ring buffer needs too.

## A pile on the laptop, with a real board on top

```sh
cargo run -p cahoots-host -- pile --cards 3 --state /tmp/pile                       # three simulated cards
cargo run -p cahoots-host -- pile --cards 3 --serial /dev/cu.usbmodem* --state /tmp/pile   # two simulated, a board on top
```

`pile` stacks cards on Unix sockets, bottom first, and runs `hal::pile` discovery on each: the bottom card names the circle, the top card presses B. With `--serial`, the top simulated card's up link is that serial port and a real badge is the top card; its button starts the count. `SerialLink` mirrors `SocketLink` and adds what a USB port needs: a `Ping` frame twice a second so the board can tell the laptop is there, and the board's `Log` frames printed here as `[board] ...`.
