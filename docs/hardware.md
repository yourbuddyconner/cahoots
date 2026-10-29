# Hardware

You can try Cahoots with phones and laptops. A badge is a separate device for running the ceremony and holding circle secrets.

The hardware is in development. A PCB render shows a design, not evidence that every part has been tested on a manufactured board.

## Choose a starting point

| Goal | Start here |
|---|---|
| Try the protocol without hardware | [Create a circle](ceremony-guide.md) in the browser. |
| Develop the badge software on a laptop | Use the [host simulator](../crates/host/README.md). |
| Wire a display and buttons to a microcontroller | Follow the [DIY badge guide](diy-badge.md). |
| Work on the current badge PCB | Read the [V1 board reference](../hardware/v1/README.md). |

## V1: Pico 2 and e-paper

The current board design uses a Raspberry Pi Pico 2, an e-paper display, two buttons, wired links, and an NXP SE051 secure element.

A breadboard Pico 2 has completed a ceremony with a laptop simulating the other members, saved the circle to flash, and loaded it again. This does not establish that a pile of manufactured V1 badges works end to end.

The secure-element driver builds. Seat-key signing and attestation have been tested with a software chip, but real-chip operation and attestation still need bench verification. Do not assume the current firmware protects a stolen badge’s pad with the secure element.

The [Pico 2 firmware README](../crates/rp2350/README.md) covers flashing and its current limitations. The [board reference](../hardware/v1/README.md) contains the pin map, schematic, component choices, and case files.

## Nano ESP32 prototype

The Nano ESP32 build uses an external e-paper module and buttons. It is a development platform for the same badge software.

Its pad is stored in plaintext flash. A person with physical access can copy it. Secure boot and flash encryption are not established protections of this build.

Use the [DIY guide](diy-badge.md) for wiring and the [ESP32 firmware README](../crates/esp32s3/README.md) for build and flash instructions.

## V2: a later design

The Baochip-based badge is a planned successor. Its firmware target is a placeholder, not an alternative you can use today.

See [Project status](status.md) before choosing a hardware path. The longer [manufacturing notes](manufacturing.md) and [roadmap](plan.md) are development references, not purchasing or assembly instructions.

## What a badge changes

A dedicated device can provide a physical confirmation screen, buttons, and more control over secret storage than a browser. Those benefits depend on the actual firmware, storage, and boot configuration.

The [device tiers](tiers.md) describe these distinctions. A circle still shares its secret with every member: one compromised member can expose it, even if everyone else uses stronger hardware.
