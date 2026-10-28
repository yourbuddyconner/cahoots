# RP2350 badge firmware

The Cahoots badge on a Raspberry Pi Pico 2: the RP2350 with 4 MiB of flash, the Waveshare 1.54" V2 e-paper, two tactile switches, the micro-USB port as the link to a laptop, and two UARTs as the links up and down a pile. It runs the same `cahoots-badge` code as the desktop simulator and the Nano; this crate is the board under it (`src/board.rs`) and the drivers: the e-paper over SPI, the buttons, the chip's true random number generator, the flash regions, and the three links.

The pin map is V1's, from [hardware/v1/design.py](../../hardware/v1/design.py), so this firmware runs unchanged on the V1 board when it arrives. The wiring diagrams are in [Build a badge](../../docs/diy-badge.md#raspberry-pi-pico-2).

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

With two or more boards wired as a pile, run the same command on the bottom board's port with `--cards` set to the laptop's cards plus the boards. The boards above the bottom one need no USB except for power.

## Flash layout

The program may use the first 2 MiB (`memory.x`). Above it, by offset from the start of flash, in `src/store.rs`:

| Region | Offset | Size | Contents |
|---|---|---|---|
| pad | 0x200000 | 1 MiB | the pad, programmed once, spent bytes programmed to zero |
| ledger | 0x300000 | 64 KiB | 16-byte spent-range records, appended |
| circle | 0x310000 | 4 KiB | `CAH2`: params, seat, circle id, the six check words, the seat key (32 bytes, plaintext like the pad), tier, the sealed manifest |
| panic | 0x311000 | 4 KiB | the last panic's message, shown at the next boot |
| trace | 0x312000 | 4 KiB | the last 4 KiB of log lines and screens before a panic, written with the panic record |

The chip runs from this flash, so every erase and program hands the bus to the boot ROM's flash routines from a function placed in RAM (`in_ram`), with interrupts off, and reads go through the memory-mapped window. The record format is byte for byte the Nano's, so the same `cahoots-host` tools read both.

Reading the trace after a panic: `picotool save -r 0x10312000 0x10313000 trace.bin` with the board in BOOTSEL, then strip the `0xFF`. Unlike the Nano, the trace is kept in RAM while the board runs and written to flash only by the panic handler, because a flash program holds interrupts off for milliseconds and the UART FIFOs hold about a third of one.

The pad is plaintext on flash. The key store chip (roadmap M4f) is what changes that.

## Memory

520 KiB of SRAM holds a 320 KiB heap and the stack. A three-member ceremony with 16 KiB contributions peaked at 152 KB of heap and 41 KB of stack; the first build's 200 KiB heap ran out while the sealed reveals were in flight. The board logs its heap and its deepest stack use as it goes, and a stack limit register set at boot turns an overflow into a recorded hard fault instead of a corrupted heap. There is no PSRAM, so the pad and the reveals live in the heap, which is why the pad proposal is 16 KiB.

## Done and not yet

On 2026-10-27 a Pico 2 on a breadboard ran a three-member ceremony with the laptop as the two cards below it, showed the same six words as the laptop, kept the circle, and read it back from flash after a reflash. PIN entry, the key store chip, and the QR screen are the same open items as on the Nano.

Twice in seven runs the count the top card sends down after Both never reached the laptop, and the ceremony waited at one commit. The log now says when the USB host is taken for absent (no byte for two seconds, after which frames down are dropped on purpose) and how many bytes each frame down was, so the next time it happens the log says why.

The UARTs receive by interrupt into a queue, and interrupts are off during every flash erase and program. Those happen at boot (the self-test sector), when a circle is kept (the pad, the ledger, the record) and on every spend (a ledger record, the spent bytes), not during the pile or the ceremony traffic, so a neighbour's frame is safe in practice. A frame that does land in those milliseconds loses bytes and is dropped by its CRC. Receiving by DMA, which runs while the flash is busy, is the proper fix and is not done.
