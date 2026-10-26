# From a breadboard to a badge you can order

*Written 2026-10-25. How a custom board gets made, what ours needs, and the order to do it in. The benchmark it builds toward is the manufactured badge in [Build a badge](diy-badge.md): a 54 × 86 mm card about 3 mm thick, the 1.54" e-paper, two buttons, magnets and pads for the pile, a lanyard slot.*

## Where we are

A dev board and a display module on jumper wires, with firmware that is about to run its first ceremony. That is a prototype of the electronics, not of the product. Nothing that runs today is on a board we designed, nothing holds together without a breadboard, and the parts are sized for convenience rather than for a card. The first board we designed, V1 in [hardware/v1](../hardware/v1/README.md), is drawn and passes KiCad's checks. It is not reviewed or ordered yet.

Yes, we design a PCB, and we design two: V1 soon, a simple board to learn the process on and to have something that works at a meetup, and V2, the card, once two questions below are answered.

## How a board gets made, in plain words

1. **Block diagram and budget.** What parts, how they connect, and how much current, board area and money each gets. Half the design decisions are made here.
2. **Parts.** Choose exact part numbers with footprints, and check they are in stock at the assembler's supplier. For small runs that means the parts library at JLCPCB or PCBWay, fed by LCSC.
3. **Schematic.** Draw the circuit in KiCad, which is free and open. Most of a design like ours is copied from reference designs: Espressif publishes the ESP32-S3 hardware guidelines, and the e-paper's boost circuit is on every open Waveshare schematic.
4. **Layout.** Draw the board outline, place the parts, route the copper, run the design rule checks. Two copper layers for a simple board; four once a bare chip with its memory is on it.
5. **Review.** Someone who has done it before reads the schematic and the layout. It is cheap compared with a wasted spin.
6. **Order.** Export Gerbers, drill files, the bill of materials and the placement file. Upload to JLCPCB or PCBWay with assembly. Five boards arrive in two to three weeks, for $100 to $400 depending on the parts.
7. **Bring-up.** Power first, with a current meter. Then flash the firmware, which already runs on this chip, and work through the self-test one peripheral at a time. Expect to find two or three mistakes, fix them, and order again. Two spins is normal, three is not a failure.
8. **Mechanical.** The tray, the top sheet, the magnets, the contacts. Prototyped by 3D printing in clear resin or by machining polycarbonate; injection moulded only at hundreds of units, because the tool costs thousands.
9. **Test and provisioning.** A fixture that powers each board through its pads, flashes it and runs the self-test, so a batch can be checked in minutes.
10. **Paperwork.** Electromagnetic compliance, battery shipping rules, RoHS. Priced below.

## Two questions to answer before the real card

**Power.** The 3 mm card assumed a CR2016 coin cell, because a ceremony is a few seconds of compute and e-paper draws only when it changes. But a CR2016 delivers a few milliamps continuously and short pulses of perhaps twenty; an ESP32-S3 at full clock draws tens of milliamps for the seconds the ceremony takes, and the panel draws about twenty for two seconds per refresh. The numbers may still work with a slower clock and a capacitor to cover the peaks, or they may not. Card-format products with displays use thin rechargeable pouch cells, half a millimetre to a millimetre and a half thick, charged through the same pads the dock uses. The measurement that decides this is one afternoon with a current meter on the Nano during a ceremony and a refresh. Until it is done, the battery, and with it the thickness, is open.

**Chip.** The product is designed around the Baochip-1x, which is why its tier is what it is: a hardware key store, on-chip memory, no radio. Those boards ship in December 2026, and the card cannot be laid out until the package height and the power draw are in hand. The ESP32-S3 is what we have now, and it has two problems for the card: the certified module is 3.2 mm tall on its own, and a bare chip needs its own flash, a crystal and a four-layer board. So the ESP32-S3 is the chip we learned on, the RP2350 is the chip for V1, and the Baochip is the chip for V2, unless December brings a surprise.

## V1

The first badge we can order: a complete badge, designed now with low risk, even if it is not a thin one. 54 × 86 mm, two copper layers. The badge is 13.4 mm thick, and 15 mm in its case.

The chip is the RP2350, as the roadmap decided for V1, and it arrives on V1 as a whole Raspberry Pi Pico 2 soldered flat onto our board through its castellated edges. That keeps every chip-level question off V1: the Pico 2 brings its own crystal, flash, regulator, micro-USB and ROM bootloader, Raspberry Pi has done the layout, and the part costs five dollars from anyone. Our board is then a connector board: the panel, the buttons, the battery, the pile contacts, and a footprint for the Pico 2. If the bare RP2350 is in the assembler's stock when V2 comes, that is when it goes on a board directly.

Why the RP2350 and not the ESP32-S3 the firmware runs on today: it has no radio, so "no radio during the ceremony" is a fact about the silicon; it has a hardware random source, signed boot, one-time-programmable key storage and glitch detectors; it draws about half the current under load and sleeps in microamps; and Rust targets it upstream, with drag-and-drop flashing over USB. The port is the four board modules, since the display driver and the pile and ceremony code are shared. The ESP32-S3 boards stay as the second family in the pile.

- A Pico 2, the plain one without headers and without the wireless chip, soldered down.
- The raw 1.54" panel, the GDEY0154D67 from Good Display, on its 24-pin flex connector, with the boost circuit from the open reference schematics. The same panel as the Waveshare module, five to seven dollars.
- A Qwiic (STEMMA QT) socket on I2C0 at the bottom centre, so any Qwiic board plugs in without soldering, and a 2x5 header on the back with 3V3, GND and eight spare GPIO for firmware forks.
- Two tactile switches, a CR2032 holder for the prototype. The Pico's micro-USB is the flashing port. The dock is not in V1.
- A five-pin magnetic pogo pair (Adafruit 5413) for the pile: the pin half stands on the board at the top end with its pins up through the lid, and the pad half sits in the tray floor, wired to the board with a short pigtail. Two JST-SH UART sockets at the bottom edge take a cable instead. Four magnet pockets in the tray. The pile mechanics get tested on real badges before the thin card is designed.
- A clear PETG tray with a snap-fit lid, printed. Resin or cut acrylic later from the same STEP.
- Decided 2026-10-26: V1 gets a secure element, the NXP SE051, on the next revision. It sits on the I2C bus next to the Qwiic socket, and until the board spins the same chip plugs into the socket on a Mikroe SE051 Plug&Trust Click board, which has its own antenna switch for the bench. The pad on the external flash is encrypted under a key the chip holds and never gives up; the chip decrypts the bytes of each spend; the ledger's counter lives in the chip. The bus between the Pico and the chip runs the chip's secure channel. The chip's NFC antenna is routed on V1 and the contactless interface ships switched off; the owner turns it on from the badge's menu, and the keys that need consent are reachable only from the Pico. See M4f in the [roadmap](plan.md).

Cost for ten: about $35 to $50 a badge, plus the mechanical prototypes. Time: a week to design, three weeks per spin, two spins. The schematic, the bill of materials and the layout are drawn in KiCad and pass its checks (hardware/v1). Next, you review, order and build the first one on the bench. What it buys: a meetup with ten real badges, the pile ritual tested with magnets and pads, and the whole process learned on something forgiving.

## V2: the card

This is the product. It is designed once the power and chip questions are answered, around the Baochip-1x, on a four-layer board the size of a credit card, with the e-paper's glass as the top surface, dome buttons under a sheet, a thin cell, the pads and magnets from V1. This is a job for a contract hardware engineer and a mechanical designer: a few weeks of work and a five-figure fee, with three spins planned. Small batches of the tray are machined or cast; a mould is justified at five hundred units.

What the unit costs at five hundred: a bill of materials around $25 to $35, so a $60 to $90 badge. The e-paper and the chip are most of it.

## Money and paperwork

- **Prototypes.** V1: under $1,000 all in. V2: $15,000 to $40,000 for the engineering, plus $3,000 to $15,000 for a tool if moulded.
- **Compliance.** A board with a radio chip on it needs radio certification only if the radio transmits; ours never does, but a bare ESP32-S3 still needs emissions testing as an ordinary electronic device, and a module carries much of that already. The Baochip has no radio, which is simpler and cheaper. Budget $5,000 to $15,000 for testing before anything is sold in volume. Hobby quantities handed out at a meetup are a different matter from a product.
- **Batteries.** Coin cells and pouch cells both have shipping rules, and a pouch cell needs a protection circuit on the board.
- **Fulfilment.** Crowd Supply does campaigns, manufacturing introductions and warehousing for this kind of open hardware, and the Baochip itself ships through them.

## In what order

1. Finish the firmware on the Nano and run the first ceremony with the laptop as the pile. In progress.
2. Order a Pico 2 and port the four board modules to it: the USB link, the flash store, the random source, the pins. A week.
3. Measure power during a ceremony and a refresh, on both chips. One afternoon. That decides the battery and the thickness.
4. Review and order V1 around the Pico 2 footprint. The design is drawn and checked. Ten units for the first meetup, the pile mechanics tested for real.
5. December: Dabao boards arrive. Port the firmware; measure the Baochip's power and height.
6. Brief a hardware engineer for V2 with the measurements, the V1 lessons and the benchmark from the DIY guide. Three spins.
7. Crowd Supply for the first run.
