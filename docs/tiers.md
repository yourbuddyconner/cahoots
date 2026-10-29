# Device tiers: concentric rings of security

Cahoots is open hardware and open protocol. Anyone can build a badge from whatever is on their bench, and a phone works with no hardware at all. The badge we make is the fully verified one. These tiers say what each kind of device protects, so that members know what they are getting and the software never claims more than the weakest device in the room can deliver.

## The rule that makes tiers matter

Every member holds the whole pad. A circle's protection against device theft or tampering is therefore the protection of its least protected member. The circle tier is the minimum of its members' device tiers, and the ceremony shows it before anyone confirms.

The randomness label (`TRNG` when every device used hardware randomness, `CSPRNG` otherwise) is a separate question, and it is also decided by the weakest member. A circle carries both labels.

## The tiers

![Device tiers](diagrams/tiers.svg)

| Tier | Device | What it protects | What it does not |
|---|---|---|---|
| **T0 Software** | A phone or laptop browser running the PWA | The protocol: correct spends, erase before send, constant-time tags, reuse detection | Anything about the platform. Erasure is best effort. A compromised OS reads the pad. |
| **T1 Open MCU** | Any microcontroller running `cahoots-hal` with a raw pad partition: the [DIY Arduino build](diy-badge.md) on an unfused Nano ESP32, and [V1](diy-badge.md) on a Pico 2 until the SE051's key work is done | Erase is real: spent bytes are overwritten in a raw partition and read back | A stolen device is a copy of the pad. Firmware can be replaced. The TRNG is whatever the chip has, with our health tests on top. |
| **T2 Sealed MCU** | The same MCU with secure boot and flash encryption fused, or any MCU with a secure element beside it that holds the pad's wrapping key and a counter the ledger is bound to. V1 carries the SE051 on its first board, and reaches this tier when the chip holds the pad's key and the counter (being built, M4f in the [roadmap](plan.md)); no Cahoots board is at it today | A dumped flash image is ciphertext. Only signed firmware runs. A restored flash copy is refused because the counter is ahead. A thief needs the PIN before the lockout | Without a secure element, the lockout counter lives in flash and can be rolled back. With one, the firmware still decides when to burn an attempt. |
| **T3 Key store** | A chip with a hardware key store, one-way counters, and a crypto engine that never exposes keys: the Baochip-1x, which is the chip in [V2](diy-badge.md) | The pad-wrapping key never leaves the engine. Failed PINs burn a one-way counter. After N failures the slot is erased and the pad with it. Boot stages lock the slot before the application runs | Physical attacks on the die. Flaws in the key-store controller, which still needs verification. |
| **T4 Inspected** | A T3 device whose silicon a member has compared with the published design under an infrared microscope, running firmware whose hash the circle pinned | The chip is what the RTL says it is. The firmware is what the circle agreed to | Supply-chain attacks between inspection and use. The inspection itself is a skill. |

Our badge targets T3 at launch and T4 for anyone willing to look.

## What each tier changes in the software

- **The ceremony screen** shows each member's tier and the resulting circle tier, next to the randomness label and the agent-seat warning, and members confirm all three. The tiers are in the manifest today; the screen that shows them is planned.
- **The manifest** records each seat's declared tier, and the commitment binds it, so it cannot change after committing. It is a claim, like the sender index. A device with a key store chip can back the claim with the chip's own proof that the seat key lives inside it, shown in the room at the sign step (roadmap M4f); a phone cannot.
- **The UI claim** follows the circle tier. A T0 circle says "erasure is best effort". A T2 or higher circle may say "a stolen device does not expose the pad without the PIN". No tier changes the confidentiality claim, which comes from the randomness label.
- **The burn procedure** (`docs/use-cases.md`) applies to every tier. Higher tiers make it needed less often.

## Assurance

A tier says what a device protects. Assurance says how sure a verifier can be that the owner meant one act. Every approval carries one of four levels.

| Level | Name | Meaning |
|---|---|---|
| 0 | claimed | Nothing was checked. |
| 1 | key | A signature from a key under a grant. |
| 2 | present | Both buttons pressed on a badge. |
| 3 | present with PIN | Both buttons pressed after a PIN. |

Higher includes lower: a level 3 approval satisfies a policy that asks for level 2. The device tier travels separately, in the manifest, so a verifier can ask for both. A policy elsewhere compares levels and never methods. It says "level 2 or more", and it does not name buttons or PINs. The objects are in [protocol](protocol.md#8-signed-objects).

## Building your own

The HAL is four traits: `Entropy`, `PadStore`, `Display`, `Link` (`crates/hal`). A new board is a crate that implements them. The host simulator is the reference implementation and also the test harness: a board passes the device suite by completing a ceremony over its links against simulated peers. The tier a board earns is decided by what its `PadStore` and boot chain can do. What the crate says about itself does not count. The README for each target states its tier and why.
