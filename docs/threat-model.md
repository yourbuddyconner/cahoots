# Threat model

## Covered

- **A network attacker** sees ciphertext and lengths. They can't read messages or change them undetected. Confidentiality comes from the pad and integrity from the one-time Poly1305 tag.
- **Whoever runs the feed, and whoever hosts the enclave,** can drop or delay messages. They can't read or forge messages in a circle made only of people. Because the head of the log is anchored, they also can't quietly remove entries or show different members different histories.
- **A compromised enclave** exposes only the circles that gave a hosted software member a seat.
- **A malicious member** can't bias the pad during the ceremony, because of commit-then-reveal.
- **A forged roster.** Every seat's key and device tier are bound into its commitment and the manifest is signed by every seat, so nobody can add a seat, change a tier or swap a key after the group has committed, and a device that receives two different signatures from one seat refuses to finish.
- **A garbled or malicious signature** during the ceremony cannot block a seat: a device keeps the first signature that verifies, not the first it sees.
- **A forged grant or approval** fails against the manifest. A grant passed on can only get narrower, and an approval signs the fields the device showed, not a hash handed to it.
- **A replayed message** is detected by the spent-range ledger and flagged as possible reuse.
- **A voice clone, or anyone who overheard a call,** cannot produce the hello words for a pair, and cannot answer with the other side's words. Moving the hello number on makes every overheard pair useless. A stolen, unlocked device still shows the words: that is the device compromise below, and the words do not defend against it.
- **Someone within antenna or camera range during the ceremony** sees commitments and public keys, which are public, and sealed reveals, which they would have to break X-Wing (X25519 and ML-KEM-768) to open, once per member. They do not get the pad. Someone spliced into the cable who substitutes keys is caught by the commitment check and the ceremony aborts.

## Not covered

- Members can read everything in their circle. That is the design.
- The circle tag proves that some member wrote a message. It does not prove which member. Proving a specific sender needs pairwise pads, which are out of scope for v1. Hello words are the exception: they use the pair keys from the sealed reveals, so they do name an individual, with X-Wing strength rather than the pad's.
- A stolen or compromised device exposes that device's pad and, until the key store chip (roadmap M4f), its seat key, with which the thief can sign as that seat until the circle revokes it (roadmap C6). A device PIN and a lockout are future work.
- Erasure on phone storage is best effort.
- Coercion of members. A press under threat looks like any other press; a duress PIN is roadmap open decision 18.
- Emanations after the ceremony: a device spending pad bytes, or drawing a decrypted message on its screen, can be read up close by the same kind of attacker. E-paper and on-die storage reduce the exposure but do not remove it.
- Phone screens show the six check words. They are derived from the pad by a hash and reveal nothing useful about it.

## Claims the UI may make

- **Confidentiality.** For a `TRNG` circle: secure against unlimited computing power, as long as pad bytes are never reused, against anyone who did not record the ceremony from inside the room. Against someone who did, as strong as X-Wing. For a `CSPRNG` circle: computationally secure.
- **Integrity inside the circle.** Information-theoretic, through the one-time Poly1305 key.
- **The hosted member's policy and the feed's log** rely on enclave attestation and signatures. That is ordinary modern cryptography. The pad is the layer that does not depend on math, and the enclave is the layer that governs the hosted member.

## Randomness

- Each device MUST generate its contribution from its hardware TRNG and run the NIST SP 800-90B continuous health tests (Repetition Count Test and Adaptive Proportion Test) on the raw output. A failed test aborts the ceremony.
- The ESP32-S3 hardware RNG is true random only while an entropy source runs. Firmware MUST enable the ADC noise source and keep the radio off.
- An external hardware source is never the only source. Its output is XORed with the device or platform source. Hardware tokens have shipped with reduced randomness before (Yubico advisory YSA-2019-02), and that is the reason.
- Dice rolls MAY be hashed and shown as an audit string. They MUST NOT be described as pad entropy.
