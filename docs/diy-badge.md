# Build a badge

Cahoots runs on any board that can do four things: make random numbers, store the pad and erase parts of it for real, show six words, and talk to the next badge over a wire. Those four are the HAL (`crates/hal`). A board that implements them and completes a ceremony is a Cahoots badge, and its README says which [tier](tiers.md) it reaches and why.

Three boards exist or are drawn.

| Board | What it is | Tier | Status |
|---|---|---|---|
| **The DIY Arduino build** | An Arduino Nano ESP32, a Waveshare 1.54" e-paper module and two buttons on jumper wires. The firmware runs here. | T1 | On the bench. Runs the pile and the ceremony with a laptop standing in for the other cards. |
| **V1** | A 54 × 86 mm board that a Raspberry Pi Pico 2 solders onto, with the raw panel, two switches, a coin cell, magnets and pogo pins for the pile, in a printed tray. | T1 | Drawn and checked in KiCad. Not reviewed or ordered. |
| **V2** | The card: the Baochip-1x on a credit-card board about 3 mm thick, the e-paper glass on top, magnets and pads for the pile. | T3; T4 for anyone who inspects the chip | Designed once the power and chip questions are answered. Baochip boards ship 15 December 2026. |

All three run the same Rust core and the same ceremony. The four trait implementations are the only part that belongs to a board.

## What every board shares

- **An e-paper screen.** Every board uses a 1.54" 200 × 200 e-paper panel. It holds the six words with no power, it is readable in sunlight, and it never needs to animate. A single code fits on it, and the reveal, which is too big for a single code, travels over the wire or the USB cable instead. A 200 × 200 panel shows a version 4 code at 6 pixels per module, which a phone reads at arm's length.
- **Two buttons.** A is next, B is confirm. That is enough for every screen: cycle through a PIN digit with A and lock it with B, confirm the words with B or reject them with A, pick a seat with A and take it with B. There is no joystick.
- **A wire to the next badge.** This is the touch link, one per neighbour. It is a UART between two boards, or the pogo pins and pads of the pile. Reveals are sealed to every member before they go on the wire, so the wire carries no secret and needs no handshake of its own. No radio is used during a ceremony on any board.
- **USB to a phone or laptop.** The browser app connects over WebUSB or WebSerial and becomes the badge's camera: it scans the other members' codes with the phone camera and passes them down the cable, and it can mirror the badge's screen. The secret never goes up the cable. Only commitments, reveals, sealed messages, and the ledger position do. This is roadmap item M4d. Today the USB link carries the pile protocol to a laptop running `cahoots-host`.

## The DIY Arduino build

This is the board on the bench, and the firmware runs here first. The parts are an Arduino Nano ESP32 (an ESP32-S3 with 16 MB of flash and 8 MB of PSRAM, USB-C, 3.3 V logic), a Waveshare 1.54" e-Paper Module V2 (200 × 200, SSD1681 controller, SPI, the 8-pin cable), and two 6 mm tactile switches. Everything joins with jumper wires. The wiring, with a diagram and the pin positions on the Nano's headers, is in [hardware/nano-esp32-epaper](../hardware/nano-esp32-epaper/README.md). That directory also holds the Arduino sketch that checked the panel before the firmware existed.

| Function | Nano pin | GPIO |
|---|---|---|
| e-paper CLK, DIN | D13, D11 | 48, 38 |
| e-paper CS, DC, RST, BUSY | D10, D9, D8, D7 | 21, 18, 17, 10 |
| button A (next), B (confirm), to GND | D2, D3 | 5, 6 |
| link to the laptop | USB-C | USB-Serial-JTAG |

The firmware is [`crates/esp32s3`](../crates/esp32s3/README.md), `no_std` Rust on `esp-hal`, and its design is in [docs/superpowers/specs/2026-10-23-esp32s3-firmware-design.md](superpowers/specs/2026-10-23-esp32s3-firmware-design.md). The display driver is `epd-waveshare`'s `epd1in54_v2` over SPI at 1 MHz, because 10 MHz does not get through the module's level shifter on jumper wires. The pad lives in a 1 MiB region of the flash, the ledger in 64 KiB after it, and the circle record in 4 KiB after that.

What it proves: the four HAL traits work on real hardware, the panel shows the badge's screens, and a board completes a pile and a ceremony with a laptop standing in for the other cards. The board is the top card. When its seat appears on the status line, press B and it counts the pile. The six words appear, B keeps the circle and A discards it. The laptop prints each simulated card's words and they match the glass.

What it does not prove: the pad is plaintext on flash, so a reader with physical access can copy it. Flash encryption and secure boot are one-way eFuse operations and wait for sign-off. The radio is present on the chip and no radio code is linked. That makes this board T1. A UART link to a second Nano, PIN entry and the QR screen are listed in the crate README under "Not yet".

## V1

V1 is the first badge anyone can order: a complete badge, designed with low risk, and not a thin one. It is a 54 × 86 mm two-layer board that a Raspberry Pi Pico 2 solders onto flat through its castellations. The Pico 2 brings its own crystal, flash, regulator, micro-USB and ROM bootloader, so no chip-level question lands on V1. The RP2350 has no radio, so "no radio during the ceremony" is a fact about the silicon.

On the board: the raw 1.54" panel (Good Display GDEY0154D67) on its 24-pin flex with the boost circuit from the panel's reference schematic, two tactile switches, a CR2032 holder behind a slide switch, a five-pin magnetic pogo pair (Adafruit 5413) for the pile, and two JST-SH UART sockets at the bottom edge for a cable to a badge on the bench or a laptop. The pin half of the pogo pair stands on the board with its pins up through the lid, and the pad half sits in the tray floor. Stack two badges and the lower one's pins meet the upper one's pads, with the magnets holding them. The case is a clear PETG tray and a snap-fit lid, printed from the same part table. The badge is 13.4 mm thick, and 15 mm in its case.

V1 has no secure element and no flash encryption. Those are V2's questions. V1 is T1.

The design, the pin map, the case and the review list before ordering are in [hardware/v1](../hardware/v1/README.md). The plan it belongs to, with the costs and the order of work, is [docs/manufacturing.md](manufacturing.md). The firmware port is the four trait implementations from `crates/esp32s3`; the display driver and the pile and ceremony code are shared.

## V2: the card

V2 is the product. The chip is the Baochip-1x: a 350 MHz VexRiscv RV32-IMAC core with an MMU, four 700 MHz PicoRV I/O cores, 4 MiB of on-chip RRAM, 2 MiB of SRAM, USB as a high-speed device, and no radio. It has its own true random number generator and a key store with one-way counters, which is why it is T3 on its own. Most of the chip's RTL is published, and its package lets an owner compare the die with the design under an infrared microscope, which is T4. The Dabao is the chip on a small evaluation board, $12 from Crowd Supply, shipping 15 December 2026. The firmware for it, `crates/dabao`, is a planned Xous app that will develop in hosted mode on a laptop with the chip's emulator (roadmap M4c). It is not started.

The card is designed once the power and chip questions in [docs/manufacturing.md](manufacturing.md) are answered. The rest of this section is the benchmark it is designed to.

The badge is a card: 54 × 86 mm, 3 mm thick, clear polycarbonate, with a slot at one end for a lanyard. It is the model on the landing page. Nothing is stacked inside except on the board; the display, the chip and the battery sit side by side.

| Part | How | mm |
|---|---|---|
| Bottom tray | moulded polycarbonate floor, 0.6 mm walls | 0.8 |
| Board | thin FR4, cutouts for the cell and the magnets | 0.4 |
| Display | 1.54" glass e-paper on the board, its own top surface | 1.1 |
| Chip and passives | any normal package, beside the display | 0.9 |
| Battery | CR2016 coin cell in a board cutout, on the floor | 1.6 |
| Buttons | two metal domes under the top sheet | 0.3 |
| Contacts | spring fingers under the card, gold pads on the face | 1.0 |
| Top sheet | clear film, printed on its underside | 0.3 |

The tallest column is the battery's, and with adhesive and tolerance the card comes out at 2.9 to 3.2 mm: four credit cards. A pile of eight is 25 mm. Whether a coin cell can power a ceremony and a refresh is the open measurement in the manufacturing plan; a thin rechargeable pouch cell is the alternative, and the thickness follows that answer.

**Cards stack instead of lining up.** Four magnets in the floor line a card up on the one below and hold it a fraction of a millimetre above it, with opposite poles so it only snaps on one way round. Four gold pads on the face at the contacts end meet four spring fingers under the card above. A pile is a line of links, so each card has one link up and one down. The firmware for it is in `crates/hal`: the ceremony driver floods each new frame on every link except the one it came from, which its dedupe makes correct on a line, a ring, or a pair. The pile layer (`hal::pile`) gives each card its seat from the card below it, with the bottom card naming the circle, and sends the count down from the top card when its button is pressed. Nobody says a seat number out loud. The ritual: press B on the top card, wait for the six words to appear on each screen, then press B to keep the circle or A to discard it. V1 tests this ritual with real magnets and pogo pins before the card is drawn.

**What 3 mm costs.** No USB-C: the receptacle alone needs about 3.2 mm of device, so the phone link runs through the same pads over a small dock with a USB-C socket on it. A coin cell instead of a charger: e-paper draws only when it changes, a ceremony is a few seconds of compute, and sleep is microamps, so a CR2016 would run the badge for a year or more and is replaceable anywhere. Glass in the card: the e-paper is glass, so it hangs on a lanyard and should not ride in a back pocket.

Two millimetres would need a flexible e-paper, a thin-film cell, a wafer-level chip package and a laminated card build. That is a card manufacturer's process, and nobody could build the same thing on a bench, so it is not the first target.

## Adding a board

Implement the four HAL traits (`Entropy`, `PadStore`, `Display`, `Link`) in a crate for the board. The `host` and `hal` crates have in-process tests that run a ceremony over a pile of simulated cards, and `cahoots-host pile --serial` runs the same simulated cards against a real board over a serial port. A device suite that runs the tests against a real board on its own is planned. A board passes by completing a ceremony and producing the same six words as the simulated cards. Write the crate's README with the tier the board earns and why, judged by what its storage and boot chain can do, not by what the crate claims. The ESP32-S3 crate is the first to run on real hardware; its four implementations (`display.rs`, `link.rs`, `store.rs`, `entropy.rs`) are the template for V1's Pico 2 and for the Dabao.

## Open questions

1. Whether a coin cell survives a ceremony and a refresh. One afternoon with a current meter on the Nano decides the battery and the thickness of V2.
2. Which Dabao I/Os carry SPI and UART, and whether a header can be fitted without soldering.
3. Whether the PIN is entered on the badge's two buttons or on the phone over USB. On the badge is the point of a PIN.
