# ESP32-S3 badge firmware: design

*Written 2026-10-23. Conner asked for this to ship without a review round, in the spirit of what is already decided. The decisions it leans on: the four HAL traits are the contract; the pile is the topology; e-paper and two buttons are the interface; no radio during a ceremony; nothing irreversible is done to a chip without Conner's sign-off; a board earns its tier by what it can actually do.*

## What it is

`crates/esp32s3`: `no_std` Rust firmware on `esp-hal` for the Arduino Nano ESP32 (ESP32-S3, 16 MB flash, 8 MB PSRAM) with the Waveshare 1.54" V2 e-paper and two tactile switches, wired as the bring-up sketch in `hardware/nano-esp32-epaper` has them. It implements the four traits and runs the same `hal::pile` and `hal::ring` code as the simulator. The ESP32-S3 becomes the first firmware board because it is the one on the bench; the Pico crate later is a port of the four trait implementations.

## Hardware map (Nano ESP32, Arduino pin / GPIO)

| Function | Pin | GPIO |
|---|---|---|
| e-paper SCK / MOSI | D13 / D11 | 48 / 38 |
| e-paper CS / DC / RST / BUSY | D10 / D9 / D8 / D7 | 21 / 18 / 17 / 10 |
| button A (next) / B (confirm) | D2 / D3 | 5 / 6, pull-up, active low |
| link to the laptop | USB-C | USB-Serial-JTAG peripheral |
| link to the next card (second board) | D4 TX, D5 RX, D6 detect | 7, 8, 9 |

## The four traits

- **Entropy.** `esp_hal::rng::Trng` with the ADC noise source enabled, wrapped in `hal::health::Tested` with a claimed min-entropy of 6 bits per byte until the source has been estimated (a `trng-dump` mode over the link, run through SP 800-90B's estimator, is the follow-up). No `esp-wifi` is linked, so the radio is never initialised: the tier note is "radio present, off by firmware".
- **PadStore.** A fixed region of the external NOR flash through `esp-storage`. Writing the pad erases its sectors to 0xFF and programs the bytes. `erase(off, len)` programs zeros over the range, with read-modify-write of the 4-byte words at the edges, so the trait's "reads back as zero" holds at byte granularity; `erase_unit` reports the 4 KiB sector. The ledger is a second region driven by `hal::ledger::PersistentLedger`; the circle record (params, seat, circle id) a third. The pad is plaintext on flash: flash encryption and secure boot are eFuse-irreversible and wait for Conner's sign-off, so this board is tier 2 at best.
- **Display.** `epd-waveshare`'s `epd1in54_v2` over SPI at 1 MHz (10 MHz does not get through the module's level shifter on jumper wires), drawn with `embedded-graphics`: the words screen in the bring-up's layout with two columns and the right column right-aligned, the status line by partial refresh, the QR with `qrcodegen` at version 4 or the smallest that fits.
- **Link.** Two implementations. `UsbLink` over the USB-Serial-JTAG peripheral: non-blocking byte I/O with a pending buffer like the host's `SocketLink`; its `peer_present` is "a frame arrived in the last two seconds", because a USB port has no detect line, and the host side sends a `Ping` frame (kind 30) every half second to keep it true. `UartLink` over UART1 with a detect input pulled low by the neighbour's ground, for the second board. Log lines go out as `Log` frames (kind 31) on the USB link so one cable carries both; both layers ignore kinds 30 and 31.

## Flow

Boot: init, show the self-test screen (TRNG health over 4 KiB, pad region write and erase and read back, buttons, links). If a circle record exists, recompute the words from the pad and show them.

Pile: `Pile::new(down = the link that has a peer, up = the other, ...)`. The board with only the USB link is the top card of a pile whose other cards the laptop simulates; pressing B on it sends `Count`, exactly the ritual in the DIY guide. With a second board on the UART, the USB link is the laptop's seat below the bottom board, or absent.

Ceremony: `into_ceremony` then `run` with the e-paper status line. Words shown. B confirms and persists the pad, ledger and circle record; A rejects and erases the pad bytes from RAM.

## Host side

`cahoots-host pile --cards N [--serial PORT] --name --pad-len --state DIR` runs N minus one simulated cards in threads over Unix sockets, bottom first; the top simulated card's up link is the serial port when given, else that card presses B itself. Each simulated card persists like `cahoots-host ceremony` does. `SerialLink` mirrors `SocketLink`, sends the `Ping` beacon, and prints `Log` frames to stderr as `[board] ...`. `Link` is implemented for `Box<dyn Link>` so a pile can mix link types.

## Build, flash, recover

`rust-toolchain.toml` pins the `esp` channel from espup; `.cargo/config.toml` sets the Xtensa target, `build-std`, and `espflash flash --monitor` as the runner; a `partitions.csv` places the app, the pad, the ledger and the circle record. The first flash needs the chip in ROM download mode: B1 to GND while pressing RST. Later flashes reset through the USB-Serial-JTAG lines on their own. Flashing replaces the Arduino bootloader; `arduino-cli burn-bootloader` or the IDE's Burn Bootloader puts it back for the Arduino sketch.

## Tests

HAL: `Ping` and `Log` frames are ignored by the pile; boxed links work. Host: an all-simulated pile of three completes a ceremony and the three devices agree on words and circle id; `SerialLink` frame stripping is a pure function with its own test. Firmware: builds for the target; each trait is exercised on the board through the log; the acceptance test is the board and two simulated cards completing a ceremony with matching words on the glass and in the host's output.

## Out of scope here

Flash encryption, secure boot, PIN entry, the dock and pads, LoRa, PSRAM (enabled only if a pad size needs it).
