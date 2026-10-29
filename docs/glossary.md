# Glossary

The words these documents use, in plain terms. Where a term has a precise definition, the [protocol](protocol.md) wins.

**Cahoot, circle.** A group of people who made one secret together at a ceremony. The code says circle. The app says cahoot.

**Ceremony.** The in-person meeting where a group makes its secret. Six steps: set up, draw, swap, mix, sign, say the words. In the app, set up is the New ceremony and Join screens, draw and swap happen on the Commit and Reveal screens, sign is the Sign screen, and mix and say the words are the Check screen. See the [ceremony guide](ceremony-guide.md).

**Pad.** The secret itself: a file of random bytes, 16 KiB to 256 KiB in the app, that every member holds in full. Also called a one-time pad. Each byte is used once and then erased.

**Contribution.** The random bytes one device brings to the ceremony. The pad is all contributions combined, so one honest device is enough to make it random.

**Commitment.** A fingerprint of a device's contribution, shared before the contribution itself. It stops anyone from choosing their bytes after seeing everyone else's.

**Sealed reveal.** A member's contribution encrypted to every other member before it leaves the device, with X-Wing (X25519 plus ML-KEM-768). The wire, the cable, or the phone screen never carries a contribution in the clear, so someone recording the ceremony from nearby learns nothing. The pad itself does not depend on this encryption.

**KEM.** Key encapsulation mechanism: a way for one party to send another a fresh secret key using only the other party's public key. Cahoots uses X-Wing, which combines a classical scheme and a post-quantum one, so both would have to fall.

**Check words.** Six English words that every device shows after the ceremony. They are derived from the pad. If everyone sees the same six words, everyone holds the same pad.

**Hello words.** Two words you say and two words another member says back, on a call, to know it is really them. They come from a key that only the two of you hold, made during the ceremony, and they change every time either of you taps Next.

**Pair key.** A key that exactly two members of a circle hold, made from the keys they exchanged during the ceremony. It is what hello words come from. Unlike the pad, it depends on the math of X-Wing.

**Circle ID.** A short public label for a group, derived from the pad. It identifies a group on the feed without revealing anything about the pad.

**Slice, part.** The region of the pad that one member sends from. Everyone holds the whole pad. Giving each member their own region means two people never use the same bytes.

**Spend.** Using pad bytes. Every spend is written to the ledger and the bytes are erased before anything leaves the device.

**Ledger.** Each device's record of which pad bytes have been used. It is how a device recognizes a replayed message.

**Pure-pad mode.** Encrypting a short message with one pad byte per message byte, plus 32 bytes for the one-time key that makes the 16-byte seal. Nothing can break it, now or later, without the pad.

**PSK mode, hedged.** Spending 32 pad bytes to seed a normal key exchange, such as WireGuard or TLS. An attacker then has to break the math and have the pad. PSK stands for pre-shared key.

**Seal, tag.** The 16 bytes on the end of a message that prove it came from a group member and was not changed. Made with a one-time Poly1305 key.

**Feed, Numbers Station.** The public bulletin board that carries encrypted messages. Anyone can read it. Only the group can read the messages. Each entry is chained to the last, so the log cannot be quietly rewritten.

**Relay.** The program that runs the feed. The crate is `cahoots-station`.

**Epoch.** In chat, a period keyed by one 32-byte spend. Any member can start a new one, which rotates everyone's keys.

**Hosted seat, agent seat.** A group member that is a program running in a cloud enclave instead of a person with a device. Its copy of the pad lives in the enclave under a policy the group pinned.

**TVC.** [Turnkey Verifiable Cloud](https://docs.turnkey.com/features/verifiable-cloud/overview): a cloud service that runs programs inside secure enclaves and publishes proof of exactly what is running.

**Badge.** The device Cahoots is designed around: a credit-card-sized board with a small screen, a wired touch link, and a hardware random number generator.

**Touch link.** The wired connection between two badges, through magnetic pins. It is how badges exchange commitments and contributions. No radio is used.

**Device tier.** T0 to T4: how well a device protects the pad once it holds one, from a phone (T0) to inspected silicon (T4). A group is as protected as its least protected device. See [device tiers](tiers.md).

**TRNG, CSPRNG.** The two randomness labels. TRNG means every device made its contribution from a hardware random number generator. CSPRNG means at least one device used software randomness. The label decides which security claim the group can make.

**Seat key.** A signing key (P-256) a device makes for one circle at the ceremony. It is bound into the device's commitment, so it cannot change after the group has seen it, and it signs everything that device says afterwards. One key per circle, never shared across circles. See [protocol section 3.4](protocol.md). On a badge with a secure element the key is made in the chip and never leaves it: the chip signs, and the badge asks it to. The key is the member's identity in the circle. It is also what signs SSH logins and git commits for that member.

**Circle root.** A 32-byte secret every member of a circle can compute and no outsider can. It is HKDF-SHA256 over the content keys of all the reveals (the key each member's reveal is encrypted under), joined in seat order, with the circle ID as salt. It makes the card seal key and the chat key. See [protocol](protocol.md) section 3.6.

**Contact card.** A seat's name, handles (such as an email or a GitHub name) and endpoints (such as an SSH host), signed by the seat key and sealed so that only the circle can read it. Cards travel as files for now. See [protocol section 8.8](protocol.md).

**Chip record.** The badge's own list of which objects in its secure element belong to which circle. It lets the badge find and delete a chip key that no circle uses, and it never deletes a key a kept circle names.

**Attested.** A seat key is attested when the chip that holds it signs a statement, over a fresh random value, that the key was made inside it. A verifier checks the statement against the chip maker's root certificate. Today only the software chip's test certificate verifies; NXP's root waits for the bench. See [device tiers](tiers.md).

**Manifest.** The circle's root document, written at the ceremony and signed by every seat: the seat keys and device tiers, the parameters, and the floor. An outside system that wants to trust a circle registers its manifest. See [protocol section 3.5](protocol.md).

**Floor.** The table in the manifest that says, for each purpose a request can name, whether the device may serve it under a grant alone, only with its owner present, or never, and how many pad bytes it may spend on it. It is the one rule no policy outside the circle can loosen.

**Grant, approval, fact, receipt.** The four things a seat signs, as CBOR with a COSE signature, defined in [protocol section 8](protocol.md). A grant lets another key ask for named purposes in this circle until a log position, and can be passed on only narrower. An approval is one seat's yes or no to one exact operation, over the fields the device showed. A fact is an attested reading in an envelope. A receipt records a spend.

**Assurance.** How sure a verifier can be that an owner meant one act, on a four-level scale from claimed, through key and present, to present with a PIN. A policy asks for a level and never for a method. See [device tiers](tiers.md).

**Log position.** The place in the circle's feed that every grant, approval and fact names as its time, instead of a clock. Checkpoints that make it checkable from outside are planned (roadmap C6).

**Test vectors.** Published example inputs and outputs. Every implementation must reproduce them so that different devices agree.

**HAL.** Hardware abstraction layer. The four things a board must provide to run the badge software: randomness, pad storage, a screen, and a link.

**Simulator.** A desktop program that behaves like a badge, used to develop and test without hardware.

**Fountain code.** The way the phone app sends a large message as a loop of QR codes: enough of the frames, in any order, rebuild the whole message.
