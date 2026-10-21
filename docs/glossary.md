# Glossary

The words these documents use, in plain terms. Where a term has a precise definition, the [protocol](protocol.md) wins.

**Cahoot, circle.** A group of people who made one secret together at a ceremony. The code says circle. The app says cahoot.

**Ceremony.** The in-person meeting where a group makes its secret. Five steps: set up, draw, touch, mix, say the words. See the [ceremony guide](ceremony-guide.md).

**Pad.** The secret itself: a file of random bytes, usually 256 KiB, that every member holds in full. Also called a one-time pad. Each byte is used once and then erased.

**Contribution.** The random bytes one device brings to the ceremony. The pad is all contributions combined, so one honest device is enough to make it random.

**Commitment.** A fingerprint of a device's contribution, shared before the contribution itself. It stops anyone from choosing their bytes after seeing everyone else's.

**Sealed reveal.** A member's contribution encrypted to every other member before it leaves the device, with X-Wing (X25519 plus ML-KEM-768). The wire, the cable, or the phone screen never carries a contribution in the clear, so someone recording the ceremony from nearby learns nothing. The pad itself does not depend on this encryption.

**KEM.** Key encapsulation mechanism: a way for one party to send another a fresh secret key using only the other party's public key. Cahoots uses X-Wing, which combines a classical scheme and a post-quantum one, so both would have to fall.

**Check words.** Six English words that every device shows after the ceremony. They are derived from the pad. If everyone sees the same six words, everyone holds the same pad.

**Circle ID.** A short public label for a group, derived from the pad. It identifies a group on the feed without revealing anything about the pad.

**Slice, part.** The region of the pad that one member sends from. Everyone holds the whole pad. Giving each member their own region means two people never use the same bytes.

**Spend.** Using pad bytes. Every spend is written to the ledger and the bytes are erased before anything leaves the device.

**Ledger.** Each device's record of which pad bytes have been used. It is how a device recognizes a replayed message.

**Pure-pad mode.** Encrypting a short message with one pad byte per message byte, plus 32 bytes for the seal. Nothing can break it, now or later, without the pad.

**PSK mode, hedged.** Spending 32 pad bytes to seed a normal key exchange, such as WireGuard or TLS. An attacker then has to break the math and have the pad. PSK stands for pre-shared key.

**Seal, tag.** The 16 bytes on the end of a message that prove it came from a group member and was not changed. Made with a one-time Poly1305 key.

**Feed, Numbers Station.** The public bulletin board that carries encrypted messages. Anyone can read it. Only the group can read the messages. Each entry is chained to the last, so the log cannot be quietly rewritten.

**Relay.** The program that runs the feed. The crate is `cahoots-station`.

**Epoch.** In chat, a period keyed by one 32-byte spend. Any member can start a new one, which rotates everyone's keys.

**Hosted seat, agent seat.** A group member that is a program running in a cloud enclave instead of a person with a device. Its copy of the pad lives in the enclave under a policy the group pinned.

**TVC.** Turnkey Verifiable Cloud: a cloud service that runs programs inside secure enclaves and publishes proof of exactly what is running.

**Badge.** The device Cahoots is designed around: a credit-card-sized board with a small screen, a wired touch link, and a hardware random number generator.

**Touch link.** The wired connection between two badges, through magnetic pins. It is how badges exchange commitments and contributions. No radio is used.

**Device tier.** T0 to T4: how well a device protects the pad once it holds one, from a phone (T0) to inspected silicon (T4). A group is as protected as its least protected device. See [device tiers](tiers.md).

**TRNG, CSPRNG.** The two randomness labels. TRNG means every device made its contribution from a hardware random number generator. CSPRNG means at least one device used software randomness. The label decides which security claim the group can make.

**Manifest.** Planned. A record written at the ceremony that names who may spend which bytes, for what. It will replace the fixed slice layout.

**Test vectors.** Published example inputs and outputs. Every implementation must reproduce them so that different devices agree.

**HAL.** Hardware abstraction layer. The four things a board must provide to run the badge software: randomness, pad storage, a screen, and a link.

**Simulator.** A desktop program that behaves like a badge, used to develop and test without hardware.

**Fountain code.** The way the phone app sends a large message as a loop of QR codes: any enough of the frames, in any order, rebuild the whole message.
