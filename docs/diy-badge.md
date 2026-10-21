# The DIY badge

A Cahoots badge you can build from three parts you can buy anywhere, with no soldering. Decided on 2026-10-21 after looking at the Wedgie pocket computer (wedgie.dev), which has exactly this shape: a Pico, a screen hat with buttons, and a secure element wedged between them. Cahoots adopts the shape. The parts below are a starting point, not a requirement; the HAL is the contract.

## The shape

| Part | Role | Starting choice |
|---|---|---|
| Microcontroller board | Runs `cahoots-hal`, holds the pad in flash, talks to the phone over USB | Raspberry Pi Pico 2 (RP2350): signed boot, 8 KB of one-time-programmable memory for keys, a hardware random number generator, glitch detectors. A Pico 1 (RP2040) also works, one tier lower. |
| Screen hat | Shows the six words and single codes, takes a PIN and a confirm press | Waveshare Pico-LCD-1.3: 240 x 240 pixels, a joystick, four buttons. Plugs straight onto the Pico. |
| Secure element | A second random source, a key that never leaves the chip, counters a copied flash cannot roll back, and on some chips a PIN enforced in silicon | Infineon OPTIGA Trust M on Adafruit's breakout (no soldering, about $5), or NXP SE050 for the PIN tier. See below. |
| Cable | Qwiic / STEMMA QT with bare wire ends, pushed into the Pico's header | Four wires: ground, 3.3 V, SDA, SCL. |

A snap-together case completes it. Any Pico-shaped board runs the code.

## What the secure element adds

Every member holds the whole pad, and the pad is too big for any secure element (64 KiB to 256 KiB against 10 kB to 50 kB of secure storage). So the pad lives in the microcontroller's flash, and the secure element does four smaller jobs that a bare microcontroller cannot do honestly.

1. **A second source of randomness.** The chip's own random number generator is read over I2C and mixed into the device's contribution, under the same rule as a YubiKey: never the only source, because the chip is closed. With a Pico 2 that makes three sources: the RP2350's generator, the secure element's, and the health tests over both.
2. **A wrapping key that never leaves the chip.** The pad is stored in flash encrypted under an AES key held in the secure element. Spending bytes means asking the chip to decrypt that range over an encrypted I2C session. Reading the flash chip alone yields ciphertext.
3. **Counters that cannot be rolled back.** The ledger's head is bound to a monotonic counter in the chip. Copying the flash, spending, and restoring the copy leaves the counter ahead of the ledger, and the device refuses to continue. Pad reuse by flash restore is the biggest hole in a bare microcontroller, and one counter closes it.
4. **A PIN with a lockout in silicon**, on chips that support it. See the comparison.

## Which chip

| | Infineon OPTIGA Trust M (V3) | NXP EdgeLock SE050 / SE051 | Microchip ATECC608B |
|---|---|---|---|
| Breakout you can buy | Adafruit 4351, Qwiic, about $5 | NXP's Arduino shield only, about $60; no hobbyist breakout found | Several Qwiic breakouts, about $1 for the chip |
| Secure storage | 10 kB | 50 kB | 16 slots, small |
| Random source | TRNG and DRNG | TRNG | NIST SP 800-90 generator |
| Symmetric key that never leaves | One AES-256 slot | AES keys with policies | AES-128 |
| Counters | Four, each up to 600,000 counts; a key can be linked to a counter so it stops working at a threshold | Monotonic counters | Two |
| PIN enforced by the chip | No PIN. Closest: link the wrapping key to a counter, burn one count per PIN attempt, and the key dies at the threshold | Yes. An authentication object with an attempt limit of 1 to 255, decremented before each try, reset only on success | No |
| Certification | CC EAL6+, PSA Level 3 | CC EAL6+, FIPS 140-2 | CC JIL High (chip) |
| Encrypted link to the host | Shielded connection | Secure session | Encrypted read and write |

**Starting choice: Trust M.** It is the one with a no-soldering breakout, and its counters give the two properties that matter most (rollback protection and a key that never leaves). The PIN is enforced by the microcontroller, with the attempt count backed by a chip counter: wrong PINs burn counts, and when the linked key reaches its threshold the pad is unrecoverable. That is a real lockout, with the limitation that the MCU firmware chooses when to burn a count.

**The PIN tier: SE050.** Its attempt counter is in the chip and the chip refuses the session, so a replaced firmware cannot skip the PIN. With 50 kB of secure storage a Small (16 KiB) pad could even live inside the chip. The cost is that no hobbyist breakout exists today, so it is a soldered build or our own board.

Both sit behind one HAL trait, so a build can swap chips without touching the ceremony or the ledger.

## What tier it reaches

Using the definitions in [device tiers](tiers.md):

| Build | Tier | Why |
|---|---|---|
| Pico 1 + screen, no secure element | T1 | Real erase, but a stolen board is a copy of the pad and the flash can be restored. |
| Pico 1 + Trust M | T2 | The pad at rest is ciphertext without the chip. The ledger cannot be rolled back. A PIN with a counter-backed lockout. Firmware can still be replaced. |
| Pico 2 + Trust M | T2, with signed firmware | Signed boot means only firmware the group pinned runs. Flash is still readable, but the pad on it is ciphertext. |
| Pico 2 + SE050 | T3 | The wrapping key never leaves the chip, the chip enforces the PIN attempts, and after the limit the key is gone and the pad with it. |

None of these is T4. That needs a published chip design and someone willing to inspect it, which is the badge we manufacture.

## How it talks to the phone

The DIY badge has a screen and buttons but no camera and no radio. The ceremony needs each member's codes to reach every other member. Two links do that:

- **USB to the phone or laptop.** The browser app connects to the badge over WebUSB or WebSerial, the way wedgie.dev's "connect" button does. The app then acts as the badge's camera and network: it scans the other members' codes with the phone camera and passes them down the cable, and it shows the badge's codes on the phone screen when the badge's own screen is too small. The secret never goes up the cable. Only commitments, reveals, sealed messages, and the ledger position do.
- **Badge to badge over the Qwiic cable.** Adafruit's breakout has two Qwiic ports, and an RP2040 can act as an I2C peripheral, so two badges can be chained with one cable for the ceremony. This is the touch link of this shape. It is a cable instead of magnets and pins, and it uses no radio.

The protocol does not change. The badge speaks the same frames to the app that it would speak over pins.

## What the app gains

Once the app can talk to a badge, anyone with a phone and a $20 kit gets a hardware random source and a pad that is never held by the phone. The app's own tier label stays honest: a circle of phones is T0, a circle of DIY badges is T2 or T3, a mixed circle is the lowest member.

## Open questions

1. Whether to make the Pico 2 the only supported board and treat RP2040 as unsupported, to keep one firmware image.
2. How a badge is first provisioned: who writes the wrapping key into the secure element, and whether a factory-fresh Trust M's existing keys are trusted or replaced.
3. Whether the screen hat's four buttons and joystick are enough for a PIN, or whether the phone enters the PIN over USB (which weakens the point of a PIN).
4. A breakout for the SE050, either found or designed, so the T3 build is also solder-free.
