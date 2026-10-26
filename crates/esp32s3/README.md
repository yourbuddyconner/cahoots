# ESP32-S3 badge firmware

The Cahoots badge on an Arduino Nano ESP32: the ESP32-S3 with 16 MB of flash, the Waveshare 1.54" V2 e-paper, two tactile switches, and the USB-C port as the link to a laptop. It runs the same `cahoots-badge` code as the desktop simulator; this crate is `NanoBoard`, the `Board` under it (`src/board.rs`), and the peripheral setup in `main.rs`. The design is in [docs/superpowers/specs/2026-10-23-esp32s3-firmware-design.md](../../docs/superpowers/specs/2026-10-23-esp32s3-firmware-design.md).

This is the first firmware board because it is the one on the bench. V1, the Pico 2 board in [Build a badge](../../docs/diy-badge.md), comes next as a port of `board.rs` and the drivers under it.

## Wiring

The same as the bring-up sketch in [hardware/nano-esp32-epaper](../../hardware/nano-esp32-epaper/README.md), diagram included there.

| Function | Nano pin | GPIO |
|---|---|---|
| e-paper CLK, DIN | D13, D11 | 48, 38 |
| e-paper CS, DC, RST, BUSY | D10, D9, D8, D7 | 21, 18, 17, 10 |
| button A (left), B (right), to GND | D2, D3 | 5, 6 |
| link to the laptop | USB-C | USB-Serial-JTAG |

## What the board does

Two buttons give three gestures: A alone is Left, B alone is Right, and both pressed within 80 ms of each other is Both. Left and Right turn pages; Both selects, keeps or allows. On a question, Both says yes and either button alone says no. A press counts only after the button has read the same for 30 ms, and a single press waits out the 80 ms before it counts, so a Both is never also reported as a Left or a Right.

1. **Self-test** on the screen and in the log: the random source through the SP 800-90B health tests, a flash sector erased, programmed and read back, the buttons, and whether a circle is stored. Both skips the rest of the wait. To forget the stored circle, hold A from before power-on until the self-test ends; a Both never forgets it. If the last boot panicked, the panic message comes first, and Both continues.
2. **If a circle is stored**, it shows the six words from the circle record and goes to the menu (step 5).
3. **Pile.** The board waits for a pile below it on the USB link. A laptop running `cahoots-host pile --serial` stands in for the other cards. The board is the top card: when its seat appears on the screen, press Both and it counts the pile.
4. **Ceremony.** The screen shows commits and reveals arriving; the six words appear with the question "keep this circle?". Both keeps the circle, writing the pad to flash with the ledger and the circle record (params, seat, circle id and the six words); Left or Right discards it.
5. **Menu**, once a circle is stored. Left and Right move between the pages and Both chooses one: `words` shows the six words again; `key as code` spends 32 bytes of pad as a key and shows them as a code; `serve a laptop` answers custodian requests from the laptop on the USB link, and each request that spends pad asks first, Both to allow and Left or Right to refuse.

Every screen is a full refresh, so each change flashes once. Each screen's text also goes to the trace sector (see below) until the sector is full.

## Build

Needs the `esp` toolchain from [espup](https://github.com/esp-rs/espup) and `espflash`:

```sh
cargo install espup espflash
espup install --targets esp32s3 --export-file ~/.cargo/esp-env.sh
source ~/.cargo/esp-env.sh        # every new shell: puts the Xtensa linker on PATH
cd crates/esp32s3
cargo build --release             # rust-toolchain.toml selects the esp channel
```

The crate is outside the workspace because the Xtensa target needs that toolchain.

## Flash

The first time, the chip must be in its ROM download mode, because the board ships with Arduino's bootloader, which only speaks DFU:

1. Unplug USB. Connect **B1** (left header, pin 13 from the USB end) to **GND** with a jumper.
2. Plug USB in. Remove the jumper. The Mac now sees a "USB JTAG/serial debug unit".
3. `cargo run --release` flashes through espflash and opens its monitor. Quit the monitor with Ctrl-C; the badge needs the port for `cahoots-host`.

After that, espflash resets the board into download mode by itself over the same port.

Flashing replaces the Arduino bootloader. To run the Arduino bring-up sketch again, put the board in ROM download mode as above and run `arduino-cli burn-bootloader -b arduino:esp32:nano_nora -p <port>`, or the IDE's Tools > Burn Bootloader.

## Run a ceremony with the laptop as the pile

```sh
cargo run -p cahoots-host -- pile --cards 3 --serial /dev/cu.usbmodem* --name bench --state /tmp/pile
```

The laptop runs two simulated cards, bottom first, with the serial port as the top simulated card's up link; the board is card 3. On the board, wait for "seat 2" on the screen, press both buttons, watch the commits and reveals count up, read the words, and press both to keep them. The host prints each simulated card's words and circle id; they match the glass. The board's log lines arrive on the same port and the host prints them as `[board] ...`.

## The link over USB

A USB port has no detect line, so `peer_present` on the board is "a frame arrived in the last two seconds", and the host's `SerialLink` sends a `Ping` frame (kind 30) twice a second. The board's log lines travel as `Log` frames (kind 31) on the same port. Both the pile layer and the ceremony driver ignore those two kinds.

## Memory

Internal RAM holds the stack and a 96 KiB heap; the pad, the reveals and the link queues
live in the Nano's 8 MB PSRAM. The post-quantum sealing in the ceremony needs well over
100 KiB of stack, and the heap static sits directly below the stack in internal RAM, so a
large internal heap starves the stack: with 288 KiB of heap the stack was 31 KiB and the
ceremony hit the stack guard. The board logs its deepest stack use as it goes.

## Reading a board that says nothing

Three flash sectors after the circle record let a stuck board be read from the laptop
without the screen or the USB link:

| Address | What |
|---|---|
| `0x411000` | The last panic's message, shown on the screen at the next boot and erased |
| `0x412000` | Every log line of the current boot, written as it happens |

```sh
espflash read-flash -p /dev/cu.usbmodem101 0x412000 0x1000 /tmp/trace.bin   # then strip 0xFF
espflash reset -p /dev/cu.usbmodem101                                       # back to the app
```

Note that `espflash monitor` and `board-info` leave the chip in download mode; `reset`
starts the app again. A reset also drops the USB port for a moment, so a host that was
listening loses it.

## Flash layout

`partitions.csv` reserves three regions, read and written by address through `esp-storage`:

| Region | Offset | Size | What |
|---|---|---|---|
| pad | 0x300000 | 1 MiB | the pad, written once onto erased sectors; spent ranges are programmed to zero in place |
| ledger | 0x400000 | 64 KiB | `hal::ledger` records, 16 bytes each, appended onto erased flash |
| circle | 0x410000 | 4 KiB | params, seat, circle id and the six check words |

A circle record written by firmware from before the words were stored loads as no circle, so the board goes to the pile; keeping a new circle overwrites the old record, pad and ledger.

The pad is plaintext on flash. Flash encryption and secure boot are one-way eFuse operations and wait for Conner's sign-off, so the board is tier 1: a hardware random source with health tests, no radio during a ceremony because no radio code is linked, and a pad that a reader with physical access can copy.

## Randomness

The ESP32-S3's RNG is a true random source only while an entropy source feeds it. `main` keeps `TrngSource` alive with the ADC noise source on for the whole run and reads through `Trng`, wrapped in `hal::health::Tested` with a claimed min-entropy of 6 bits per byte. That claim has not been estimated yet; a `trng-dump` mode over the link, run through SP 800-90B's estimator, is the follow-up.

## Not yet

A UART link to a second board, PIN entry, the QR screen (`key as code` draws the bytes as text for now), PSRAM.
