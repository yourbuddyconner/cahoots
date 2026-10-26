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

Writes on the socket link never block. Bytes that do not fit the socket buffer are queued and flushed on every later call. The ring driver keeps reading while it flushes its last forwarded frames before it finishes, so a device that completes first never strands its neighbour. A UART driver with a DMA ring buffer needs both behaviours too.

## A pile on the laptop, with a real board on top

```sh
cargo run -p cahoots-host -- pile --cards 3 --state /tmp/pile                       # three simulated cards
cargo run -p cahoots-host -- pile --cards 3 --serial /dev/cu.usbmodem* --state /tmp/pile   # two simulated, a board on top
```

`pile` stacks cards on Unix sockets, bottom first, and runs `hal::pile` discovery on each: the bottom card names the circle, the top card presses B. With `--serial`, the top simulated card's up link is that serial port and a real badge is the top card; its button starts the count. `SerialLink` mirrors `SocketLink` and adds what a USB port needs: a `Ping` frame twice a second so the board can tell the laptop is there, and the board's `Log` frames printed here as `[board] ...`.

## The badge crate on the laptop

`badge` runs one card through `cahoots-badge`, the same code a real board runs, on a `HostBoard`: the boot screen, then a pile pass while the card has no circle, then the idle menu. `--listen` is the socket the card below connects to (the down link), and `--next` is the card above's socket (the up link). Use `-` for a side with nothing plugged in.

```sh
H=target/debug/cahoots-host
$H badge --state /tmp/b0 --listen - --next /tmp/b1.sock --name sf --pad-len 16384 &   # bottom
$H badge --state /tmp/b1 --listen /tmp/b1.sock --next - --name sf --pad-len 16384 &   # top
echo B > /tmp/b1/buttons.txt     # top card, once it shows its seat: count the pile
echo B > /tmp/b0/buttons.txt     # at "B keep this circle . A discard" on each card: keep
echo B > /tmp/b1/buttons.txt
```

The state directory holds the files `Device` keeps (`pad.bin`, `ledger.bin`, `circle.txt`), so `status`, `send` and `recv` work on it. It also holds:

| File | What it does |
|---|---|
| `display.png` | The screen |
| `status.txt` | The text of the latest screen, so a script can follow the card |
| `buttons.txt` | Each line `A` or `B` is one press. The card reads the file and removes it. |
| `a_down` | While this file exists, button A is held. Create it before boot to forget the circle. |

`tests/badge.rs` runs two `HostBoard`s in threads over a socket, presses B by writing `buttons.txt`, and checks that both cards keep the same circle and that `Device::open` reads the same check words from each directory.
