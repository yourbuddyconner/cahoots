# RP2350 badge firmware

The Cahoots badge on a Raspberry Pi Pico 2: the RP2350 with 4 MiB of flash, the Waveshare 1.54" V2 e-paper, two tactile switches, the micro-USB port as the link to a laptop, and two UARTs as the links up and down a pile. It runs the same `cahoots-badge` code as the desktop simulator and the Nano; this crate is the board under it (`src/board.rs`) and the drivers: the e-paper over SPI, the buttons, the chip's true random number generator, the flash regions, and the three links.

The pin map is V1's, from [hardware/v1/design.py](../../hardware/v1/design.py), so this firmware runs unchanged on the V1 board when the boards arrive. Today it runs a ceremony on a breadboard with the Waveshare e-paper, and probes the SE051 at boot through a Mikroe Click board. The wiring diagrams are in [Build a badge](../../docs/diy-badge.md#raspberry-pi-pico-2).

## Wiring

| Function | GPIO | Pico pin |
|---|---|---|
| e-paper CLK, DIN | GP18, GP19 | 24, 25 |
| e-paper CS, DC, RST, BUSY | GP17, GP16, GP20, GP21 | 22, 21, 26, 27 |
| e-paper VCC, GND | 3V3, GND | 36, 38 |
| button A (left), B (right), to GND | GP14, GP15 | 19, 20 |
| link up: UART0 TX, RX | GP0, GP1 | 1, 2 |
| link down: UART1 TX, RX | GP4, GP5 | 6, 7 |
| link to the laptop | USB | |
| SE051 SDA, SCL (I2C0, 400 kHz) | GP8, GP9 | 11, 12 |
| SE051 enable, held high | GP2 | 4 |

The pile links are crossed between boards: this board's GP0 to the board above's GP5, this board's GP1 to the board above's GP4, ground to ground. Each RX pin carries a pull-down, so a connected peer's idle-high TX is the detect line.

## What the board does

The same as the Nano, which [its README](../esp32s3/README.md#what-the-board-does) describes: self-test, the stored circle or the pile, the ceremony, then the menu. One difference is where the pile below comes from. The link down is UART1 when a board is on it, and the USB port otherwise, so the bottom board of a bench pile talks to the laptop over USB and every board above it talks over its cable. The link up is always UART0.

## Build

Needs the Cortex-M33 target and `picotool`:

```sh
rustup target add thumbv8m.main-none-eabihf
brew install picotool                  # or build it from raspberrypi/picotool
cd crates/rp2350
cargo build --release
```

The crate is outside the workspace because it has its own target and linker setup.

## Flash

The first time, hold BOOTSEL while plugging the Pico in; it appears as a drive named `RP2350`. Once this firmware runs, `picotool reboot -f -u` does that from the laptop, since the firmware carries picotool's reset interface. Then either:

```sh
cargo run --release                    # picotool loads the ELF over USB and starts it
```

or make a UF2 and drag it onto the drive:

```sh
picotool uf2 convert target/thumbv8m.main-none-eabihf/release/cahoots-rp2350 -t elf cahoots-rp2350.uf2
```

After it starts, the Mac sees a serial port `/dev/cu.usbmodem*`, which is the badge's USB link. Flashing again leaves the pad, the ledger and the circle record in place: only the program region is rewritten.

## Run a ceremony with the laptop as the pile

```sh
cargo run -p cahoots-host -- pile --cards 3 --serial /dev/cu.usbmodem* --name bench --state /tmp/pile
```

As on the Nano: the laptop runs two simulated cards and the board is card 3. Wait for "seat 2" on the screen, press both buttons, watch the commits and reveals count up, read the words, press both to keep the circle.

## Two boards, no laptop in the pile

Wire the top board's UART1 to the bottom board's UART0, crossed, with the grounds joined: top GP5 to bottom GP0, top GP4 to bottom GP1. The bottom board names the circle and the top board's Both starts the count; the laptop is not a card. Each board can still be on USB for power and its log, read with the quiet reader, which sends nothing and so does not make a board think a laptop is below it:

```sh
cargo run -p cahoots-host -- logs --serial /dev/cu.usbmodem* --label bottom
```

A Pico 2 W runs this firmware unchanged: it is the same RP2350 on the pins the badge uses, and its radio exists only once firmware turns it on, which this never does. A Pico 1 is an RP2040 and would need its own port.

With the laptop as part of a longer pile instead, run `pile` on the bottom board's port with `--cards` set to the laptop's cards plus the boards; `pile` pings, which is how a board knows the laptop is below it.

## Flash layout

The program may use the first 2 MiB (`memory.x`). Above it, by offset from the start of flash, in `src/store.rs`:

| Region | Offset | Size | Contents |
|---|---|---|---|
| pad | 0x200000 | 1 MiB | the first circle's pad, programmed once, spent bytes programmed to zero |
| ledger | 0x300000 | 64 KiB | the first circle's 16-byte spent-range records, appended |
| circle | 0x310000 | 4 KiB | the first circle's record, `CAH3`: params, seat, circle id, the six check words, the seat key (32 bytes, plaintext like the pad; zeros when the key is in the chip), tier, the sealed manifest, the chip slot and the circle root (when there is no chip). A `CAH2` record, without the last two, still loads |
| panic | 0x311000 | 4 KiB | the last panic's message, shown at the next boot |
| trace | 0x312000 | 4 KiB | the last 4 KiB of log lines and screens before a panic, written with the panic record |
| circles | 0x313000 | 15 x 4 KiB | the records of circles 2 to 16, one sector each, as the first's |
| places | 0x322000 | 15 x 56 KiB | for circles 2 to 16, one block each: a 48 KiB pad, then an 8 KiB ledger (512 spends) |
| chip record | 0x3F4000 | 4 KiB | `CAHC`, a length and the chip record: which chip slots hold a circle's seat key, root and epoch counter |

The badge holds up to 16 circles. Each has its own record, pad and ledger, and keeps its place until it is forgotten; a new circle takes the lowest free place. The first place is where firmware from before many circles kept its one circle, so flashing over keeps it. A pad in any place but the first can be at most 48 KiB; the Pico proposes 16 KiB.

The chip runs from this flash, so every erase and program hands the bus to the boot ROM's flash routines from a function placed in RAM (`in_ram`), with interrupts off, and reads go through the memory-mapped window. The record format is byte for byte the Nano's, so the same `cahoots-host` tools read both.

Reading the trace after a panic: `picotool save -r 0x10312000 0x10313000 trace.bin` with the board in BOOTSEL, then strip the `0xFF`. Unlike the Nano, the trace is kept in RAM while the board runs and written to flash only by the panic handler, because a flash program holds interrupts off for milliseconds and the UART FIFOs hold about a third of one.

The pad is plaintext on flash. The chip holds the seat keys and roots of circles made with it, but not the pad.

## The chip

The NXP SE051 (for now on a MikroE Click board) sits on I2C0, GP8 and GP9, at 400 kHz, with its enable pin GP2 held high. At boot the firmware wakes it, selects its applet and, once in the chip's life, creates the P-256 curve object. The self-test then shows `se051: applet x.y.z, N circles`, where N is the number of seat keys in the chip. If nothing answers, the line is `se051: not found` and the badge runs as it did before the chip: the seat key and the root stay in the circle record.

A circle made on a badge with a chip keeps its seat key (P-256, made in the chip and never out of it), its root (an HMAC key that can be used but not read) and its chat epoch counter in the chip, at object ids `0x4C41_01ss`, `0x4C41_02ss` and `0x4C41_03ss` for chip slot `ss`. The chip record above says which slots are in use; at boot the badge deletes any Cahoots object in a slot the record does not know.

The driver is `crates/chip/src/se05x.rs`, which puts Nitrokey's [`se05x`](https://crates.io/crates/se05x) crate (0.4.2) behind the `ChipOps` trait the rest of the code calls. Every command runs in the default session.

Checked on a real chip (2026-10-29, applet 7.2.0): a counter goes up by one on a bare `WriteCounter`, and the applet refuses to create a counter with a size and a value in one command (0x6A80), so the driver creates it at zero and sets a value in a second write. Not yet checked: the order of the bytes in Ed25519 keys and signatures (nothing makes one today), the object ids of NXP's attestation key and certificate (`0xF0000012`, `0xF0000013`), and the exact bytes the chip signs in an attestation, which `crates/objects/src/attestation.rs` must match.

### The bench, hands off

`scripts/bench.sh` runs the whole thing with nobody touching the board: it builds the firmware with its `bench` feature, flashes it, runs a ceremony with the laptop as two cards under the badge, then attests, starts the agent, signs a file and a git commit, and checks both. The `bench` feature lets the laptop press the badge's buttons with a `Press` frame (kind 29) on the USB link; the laptop's serial link (`cahoots-host`, with `CAHOOTS_BENCH=new-circle` or `serve`) reads the badge's screen lines and presses for the goal, and the firmware logs `drawn` after each refresh so a press is never one the badge throws away as made during the refresh. The boot line of such a build says `BENCH BUILD`; release firmware never has the feature, because a badge that takes presses from a cable has no consent. Bench builds also probe the chip's counter commands at boot and log the answers.

Two things the bench taught about USB on macOS: a port handle opened after `picotool reboot` of a running badge can deliver nothing until the device enumerates again (a BOOTSEL flash cycle, or a replug), so the script never reboots a running badge and treats a silent port as a reason to flash; and the laptop's writes to the port must be non-blocking, or a badge that answers NAK while its inbox is full (flow control, so a 20 KiB reveal is never overwritten) holds the laptop in the kernel and both sides wait on each other.

## The license

The `se05x` crate is LGPL-3.0-only. This firmware is MIT OR Apache-2.0 and is one static binary that includes it. The LGPL allows that as long as a user can rebuild the binary with a changed copy of the library. Here they can: the repository is public, and `cargo build --release` in this directory makes the binary from public sources. The crate is used only here, in the firmware, and only behind `ChipOps`, so it can be replaced without changing anything above it. The workspace's default build does not include it.

## Memory

520 KiB of SRAM holds a 384 KiB heap and the stack. A three-member ceremony with 16 KiB contributions peaked at 152 KB of heap and 41 KB of stack without the chip; with the chip, two reveals arriving at once pushed the first build's 320 KiB heap past its end (the 200 KiB before it ran out the same way), so the heap is now 384 KiB (432 KiB left the stack too little room: a hard fault with the stack-overflow bit set, 64 KB deep, right after the chip made the seat key), the ceremony takes each frame out of its receive buffer before handling it, and a card with nobody to forward to builds no frame. The board logs its heap and its deepest stack use as it goes, and a stack limit register set at boot turns an overflow into a recorded hard fault instead of a corrupted heap. There is no PSRAM, so the pad and the reveals live in the heap, which is why the pad proposal is 16 KiB.

## Done and not yet

On 2026-10-27 a Pico 2 on a breadboard ran a three-member ceremony with the laptop as the two cards below it, showed the same six words as the laptop, kept the circle, and read it back from flash after a reflash. PIN entry and the QR screen are the same open items as on the Nano. The chip's driver is built but not yet run on the board: flashing, a ceremony with the chip, an attestation checked against NXP's root, and ssh and git signing remain for the bench.

Twice in seven runs the count the top card sends down after Both never reached the laptop, and the ceremony waited at one commit. The log now says when the USB host is taken for absent (no byte for two seconds, after which frames down are dropped on purpose) and how many bytes each frame down was, so the next time it happens the log says why.

The UARTs receive by interrupt into a queue, and interrupts are off during every flash erase and program. Those happen at boot (the self-test sector), when a circle is kept (the pad, the ledger, the record) and on every spend (a ledger record, the spent bytes), not during the pile or the ceremony traffic, so a neighbour's frame is safe in practice. A frame that does land in those milliseconds loses bytes and is dropped by its CRC. Receiving by DMA, which runs while the flash is busy, is the proper fix and is not done.
