# Threat model

## Covered

- **A network attacker** sees ciphertext and lengths. They can't read messages or change them undetected. Confidentiality comes from the pad. Integrity comes from the one-time Poly1305 tag.
- **Whoever runs the feed, and whoever hosts the enclave,** can drop or delay messages. They can't read or forge messages in a circle made only of people. Because the head of the log is anchored, they also can't quietly remove entries or show different members different histories.
- **A compromised enclave** exposes only the circles that gave a hosted software member a seat.
- **A malicious member** can't bias the pad during the ceremony, because of commit-then-reveal.
- **A replayed message** is detected by the spent-range ledger and flagged as possible reuse.
- **Someone within antenna or camera range during the ceremony** sees commitments and public keys, which are public, and sealed reveals, which they would have to break X-Wing (X25519 and ML-KEM-768) to open, once per member. They do not get the pad. Someone spliced into the cable who substitutes keys is caught by the commitment check and the ceremony aborts.

## Not covered

- Members can read everything in their circle. That is the design.
- The circle tag proves some member wrote a message, not which member. Proving a specific sender needs pairwise pads, which are out of scope for v1.
- A stolen or compromised device exposes that device's pad. A device PIN and a lockout are future work.
- Erasure on phone storage is best effort.
- Coercion of members.
- Emanations after the ceremony: a device spending pad bytes, or drawing a decrypted message on its screen, can be read up close by the same kind of attacker. E-paper and on-die storage reduce the exposure; nothing removes it.
- Phone screens show the six check words. They are derived from the pad by a hash and reveal nothing useful about it.

## Claims the UI may make

- **Confidentiality.** For a `TRNG` circle: secure against unlimited computing power, as long as pad bytes are never reused, against anyone who did not record the ceremony from inside the room. Against someone who did, as strong as X-Wing. For a `CSPRNG` circle: computationally secure.
- **Integrity inside the circle.** Information-theoretic, through the one-time Poly1305 key.
- **The hosted member's policy and the feed's log** rely on enclave attestation and signatures. That is ordinary modern cryptography. The pad is the layer that does not depend on math. The enclave is the layer that governs the hosted member.

## Randomness

- Each device MUST generate its contribution from its hardware TRNG and run the NIST SP 800-90B continuous health tests (Repetition Count Test and Adaptive Proportion Test) on the raw output. A failed test aborts the ceremony.
- The ESP32-S3 hardware RNG is true random only while an entropy source runs. Firmware MUST enable the ADC noise source and keep the radio off.
- A YubiKey is never the only source. Its output is XORed with the device or platform source. The 2019 YubiKey FIPS series 4 reduced-randomness advisory (YSA-2019-02) is the reason.
- Dice rolls MAY be hashed and shown as an audit string. They MUST NOT be described as pad entropy.
