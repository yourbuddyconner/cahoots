# Device tiers: concentric rings of security

Cahoots is open hardware and open protocol. Anyone can build a badge from whatever is on their bench, and a phone works with no hardware at all. The badge we make is the fully verified one. These tiers say what each kind of device protects, so that members know what they are getting and the software never claims more than the weakest device in the room can deliver.

## The rule that makes tiers matter

Every member holds the whole pad. A circle's protection against device theft or tampering is therefore the protection of its **least protected member**. The circle tier is the minimum of its members' device tiers, and the ceremony shows it before anyone confirms.

The randomness label (`TRNG` when every device used hardware randomness, `CSPRNG` otherwise) is a separate question, and it is also decided by the weakest member. A circle carries both labels.

## The tiers

![Device tiers](diagrams/tiers.svg)

| Tier | Device | What it protects | What it does not |
|---|---|---|---|
| **T0 Software** | A phone or laptop browser running the PWA | The protocol: correct spends, erase before send, constant-time tags, reuse detection | Anything about the platform. Erasure is best effort. A compromised OS reads the pad. |
| **T1 Open MCU** | Any microcontroller running `cahoots-hal` with a raw pad partition, for example an unfused ESP32-S3 | Erase is real: spent bytes are overwritten in a raw partition and read back | A stolen device is a copy of the pad. Firmware can be replaced. The TRNG is whatever the chip has, with our health tests on top. |
| **T2 Sealed MCU** | The same MCU with secure boot and flash encryption fused, and a PIN with a lockout counter in firmware | A dumped flash image is ciphertext. Only signed firmware runs. A thief needs the PIN before the lockout | The flash key is per device and not PIN-bound in hardware. The lockout counter lives in flash, so a determined attacker with chip access can roll it back. |
| **T3 Key store** | A chip with a hardware key store, one-way counters, and a crypto engine that never exposes keys: the Baochip-1x | The pad-wrapping key never leaves the engine. Failed PINs burn a one-way counter. After N failures the slot is erased and the pad with it. Boot stages lock the slot before the application runs | Physical attacks on the die. Flaws in the key-store controller, which must be verified against the RTL (see `docs/plan.md`, M5). |
| **T4 Inspected** | A T3 device whose silicon a member has compared with the published design under an infrared microscope, running firmware whose hash the circle pinned | The chip is what the RTL says it is. The firmware is what the circle agreed to | Supply-chain attacks between inspection and use. The inspection itself is a skill. |

Our badge targets T3 at launch and T4 for anyone willing to look.

## What each tier changes in the software

- **The ceremony screen** shows each member's tier and the resulting circle tier, next to the randomness label and the agent-seat warning. Members confirm all three.
- **The manifest** records each principal's declared tier. It is a claim, like the sender index. T3 and T4 devices can back the claim with a boot attestation posted as a control message; T0 to T2 cannot.
- **The UI claim** follows the circle tier. A T0 circle says "erasure is best effort". A T2 or higher circle may say "a stolen device does not expose the pad without the PIN". No tier changes the confidentiality claim, which comes from the randomness label.
- **The burn procedure** (`docs/use-cases.md`) applies to every tier. Higher tiers make it needed less often.

## Building your own

The HAL is four traits: `Entropy`, `PadStore`, `Display`, `Link` (`crates/hal`). A new board is a crate that implements them. The host simulator is the reference implementation and also the test harness: a board passes the device suite by completing a ring ceremony against simulated peers. The tier a board earns is decided by what its `PadStore` and boot chain can actually do, not by what the crate says. The README for each target states its tier and why.
