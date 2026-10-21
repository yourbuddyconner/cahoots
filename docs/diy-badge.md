# The DIY badge

A Cahoots badge you can build from three parts you can buy anywhere, with no soldering, for about $20. The shape comes from the Wedgie pocket computer (wedgie.dev): a Pico, a screen hat with buttons, and a secure element wedged between them. Cahoots adopts the shape. The parts below are the recommended build; the matrix after it lists the alternatives and what each one costs you in dollars and in security.

**Status: not yet built.** This page is the plan we are building to. The firmware image does not exist yet (roadmap M4b). We will build one ourselves first and replace this line with what we learned.

![The stack](diagrams/diy-stack.svg)

## The recommended build

| Part | Why this one | Price |
|---|---|---|
| **Raspberry Pi Pico 2 with headers** (Pico 2 H) | RP2350: signed boot, 8 KB of one-time-programmable memory for keys, a hardware random number generator, glitch detectors. Two dollars more than a Pico 1 for a whole tier of security. | about $7 |
| **Waveshare Pico-LCD-1.3** | 240 × 240 colour screen, a joystick, and four buttons on a hat that plugs straight onto the Pico. Big enough for the six words, a single code, and a PIN prompt. | $7.31 to $8.99 |
| **Adafruit OPTIGA Trust M breakout** (product 4351) | The only secure element on a no-soldering Qwiic breakout. A second random source, an AES key that never leaves the chip, four counters a copied flash cannot roll back. | $4.95 |
| **Qwiic / STEMMA QT cable with bare wire ends** | Four wires that push into the hat's header beside the Pico's pins. | about $1 |
| **Total** | | **about $21** at single-unit retail, under $20 with the screen bought direct from Waveshare or any part bought in a pack |

Where to buy: the Pico 2 H from any Raspberry Pi reseller; the screen from [Waveshare](https://www.waveshare.com/product/pico-lcd-1.3.htm), [The Pi Hut](https://thepihut.com/collections/raspberry-pi-screens/products/1-3-ips-lcd-display-module-for-raspberry-pi-pico-240x240) or [RobotShop](https://robotshop.com/products/waveshare-13in-lcd-display-module-rpi-pico-65k-colors-240x240-spi); the Trust M breakout from [Adafruit](https://www.adafruit.com/product/4351), Mouser, DigiKey or The Pi Hut; the cable from Adafruit (STEMMA QT to bare wires) or SparkFun (Qwiic breadboard cable).

## The matrix

Pick one from each row. The tier column uses the definitions in [device tiers](tiers.md). A circle is as safe as its least safe member, so a group of these badges is the lowest tier among them.

### Board

| Option | Price | What you get | What you give up |
|---|---|---|---|
| **Pico 2 H** (recommended) | $7 | Signed boot, on-chip random source, glitch detectors, headers fitted | |
| Pico 2 (no headers) | $5 | The same chip | You solder two header strips. Ten minutes, and it breaks the no-soldering promise |
| Pico H / Pico (RP2040) | $4 to $5 | Runs the same firmware | No signed boot, so replaced firmware runs. One tier lower with the same secure element |
| RP2040 clone boards | $2 to $3 in packs | Cheapest possible | Quality varies; the case may not fit; no signed boot |
| Pico 2 W / Pico W | $7 to $8 | Wi-Fi and Bluetooth | A radio on the badge. The protocol requires the radio off during a ceremony, and a badge with no radio has nothing to prove. Not recommended |

### Screen

| Option | Price | What you get | What you give up |
|---|---|---|---|
| **Waveshare Pico-LCD-1.3** (recommended) | $7 to $9 | 240 × 240 colour, joystick, four buttons. Shows words, a scannable code, and a PIN pad | |
| Waveshare Pico-OLED-1.3 | $8 to $9 | 128 × 64 monochrome, two buttons | Too small to show a code a phone can scan. Codes go through the phone over USB instead. PIN entry with two buttons is slow |
| Bare 1.3" ST7789 module | $3 to $4 | The same panel as the hat | Seven wires to solder, no buttons, no case fit |
| No screen | $0 | The phone is the screen over USB | The badge cannot show you the six words itself, so you are trusting the phone's display of them. Fine for a headless node (a router), wrong for a person's badge |

### Secure element

| Option | Price | What you get | What you give up |
|---|---|---|---|
| **Infineon OPTIGA Trust M V3**, Adafruit breakout (recommended) | $5 | TRNG, one AES-256 key that never leaves the chip, four counters (600,000 counts each), 10 kB storage, CC EAL6+, PSA Level 3, encrypted I2C session. A key can be linked to a counter so it stops working at a threshold, which is how the PIN lockout is built | The chip has no PIN of its own. The firmware decides when to burn an attempt |
| NXP EdgeLock SE050 / SE051 | $7 chip, no hobbyist breakout | 50 kB storage, AES with policies, counters, and a PIN enforced by the chip: an authentication object with an attempt limit of 1 to 255, counted down before each try, reset only on success. Reaches T3 | You solder it, or we design a breakout. NXP's own board is a $60 Arduino shield |
| Microchip ATECC608B | $1 chip, several Qwiic breakouts | RNG, AES-128, two counters, 16 small slots | No PIN, fewer counters, less storage. Enough for the counter and the wrapping key, nothing more |
| None | $0 | | T1 at best: a stolen board is a copy of the secret and a restored flash copy is not detected |

### What each combination reaches

| Build | Tier | Why |
|---|---|---|
| Pico 1 + screen, no secure element | T1 | Real erase, but a stolen board is a copy of the pad and the flash can be restored. |
| Pico 1 + Trust M | T2 | The pad at rest is ciphertext without the chip. The ledger cannot be rolled back. A PIN with a counter-backed lockout. Firmware can still be replaced. |
| **Pico 2 + Trust M** (recommended) | T2 with signed firmware | Only firmware the group pinned runs. Flash is still readable, but the pad on it is ciphertext. |
| Pico 2 + SE050 | T3 | The wrapping key never leaves the chip, the chip enforces the PIN attempts, and after the limit the key is gone and the pad with it. |

None of these is T4. That needs a published chip design and someone willing to inspect it, which is the badge we manufacture.

## Wiring

![Wiring](diagrams/diy-wiring.svg)

The cable's plug carries four wires in a fixed order: ground, 3.3 V, SDA, SCL. Trust that order, not the colours, because cables from different makers use different colours. With the Pico's USB connector at the top and counting pins from that end:

| Wire | Goes to | Pico pin | Signal |
|---|---|---|---|
| Ground | right side, 3rd pin | 38 | GND |
| 3.3 V | right side, 5th pin | 36 | 3V3 out. **Never the 4th pin**: that is 3V3_EN, and grounding it holds the board off |
| SDA | left side, 6th pin | 6 | GP4, I2C0 SDA |
| SCL | left side, 7th pin | 7 | GP5, I2C0 SCL |

The screen hat uses SPI1 on GP8 to GP13 and the buttons and joystick on GP2, GP3 and GP15 to GP21, so nothing collides with GP4 and GP5. The Trust M answers at I2C address 0x30.

## Assembly

1. Push the four bare wire ends into the hat's female header, in the slots beside the four Pico pins above. The wire shares the slot with the pin. Push until the insulation meets the header.
2. Plug the Pico into the hat, USB end matching the hat's marked end.
3. Plug the cable's other end into the Trust M breakout's Qwiic port and lay the breakout flat between the two boards, port toward the USB end.
4. Snap the case on, if you have one. Wedgie's case is made for the pink NULLLAB Pico; a case for the official Pico 2 H will be published with the firmware.
5. Plug the USB cable into a computer.

## Firmware and first boot

Not published yet. When it is, the steps will be: hold BOOTSEL while plugging in, drag the `.uf2` file onto the drive that appears, and the badge reboots into Cahoots. The first screen will show the firmware hash so the group can compare it, then ask for a PIN.

What the firmware will check on first boot, and show on the screen: the secure element answers on I2C and its random source passes the health tests; the screen and all buttons work; the flash region for the pad erases and reads back clean.

## What the secure element does

Every member holds the whole pad, and the pad is too big for any secure element (64 KiB to 256 KiB against 10 kB to 50 kB of secure storage). So the pad lives in the microcontroller's flash, and the secure element does four smaller jobs that a bare microcontroller cannot do honestly.

1. **A second source of randomness.** The chip's random number generator is read over I2C and mixed into the device's contribution, under the same rule as a YubiKey: never the only source, because the chip is closed. With a Pico 2 that makes three sources: the RP2350's generator, the secure element's, and the health tests over both.
2. **A wrapping key that never leaves the chip.** The pad is stored in flash encrypted under an AES key held in the secure element. Spending bytes means asking the chip to decrypt that range over an encrypted I2C session. Reading the flash chip alone yields ciphertext.
3. **Counters that cannot be rolled back.** The ledger's head is bound to a monotonic counter in the chip. Copying the flash, spending, and restoring the copy leaves the counter ahead of the ledger, and the device refuses to continue. Pad reuse by flash restore is the biggest hole in a bare microcontroller, and one counter closes it.
4. **A PIN with a lockout.** On the Trust M, the wrapping key is linked to a counter with a threshold; each wrong PIN burns a count and at the threshold the key stops working, which makes the pad unrecoverable. On the SE050 the chip itself counts the attempts.

## How it talks to the phone

The DIY badge has a screen and buttons but no camera and no radio. The ceremony needs each member's codes to reach every other member. Two links do that:

- **USB to the phone or laptop.** The browser app connects to the badge over WebUSB or WebSerial. The app then acts as the badge's camera: it scans the other members' codes with the phone camera and passes them down the cable, and it can mirror the badge's codes on the phone screen. The secret never goes up the cable. Only commitments, reveals, sealed messages, and the ledger position do. This is roadmap item M4d.
- **Badge to badge over a Qwiic cable.** Adafruit's breakout has two Qwiic ports, and an RP2040 or RP2350 can act as an I2C peripheral, so two badges can be chained with one cable for the ceremony. This is the touch link of this shape: a cable instead of magnets and pins, and no radio.

The protocol does not change. The badge speaks the same frames to the app that it would speak over pins.

## Open questions

1. Whether to make the Pico 2 the only supported board and treat the RP2040 as unsupported, to keep one firmware image.
2. How a badge is first provisioned: who writes the wrapping key into the secure element, and whether a factory-fresh Trust M's existing keys are trusted or replaced.
3. Whether the screen hat's four buttons and joystick are enough for a PIN, or whether the phone enters the PIN over USB, which weakens the point of a PIN.
4. A breakout for the SE050, either found or designed, so the T3 build is also solder-free.
