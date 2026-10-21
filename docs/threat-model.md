# Threat model

## Covered

- **A network attacker** sees ciphertext and lengths. They can't read messages or change them undetected. Confidentiality comes from the pad. Integrity comes from the one-time Poly1305 tag.
- **The Numbers Station operator and the TVC host** can drop or delay messages. They can't read or forge messages in human-only circles. Log anchoring prevents rollback and forked views.
- **A compromised Cahoots enclave** exposes only circles that seat an agent.
- **A malicious member** can't bias the pad during the ceremony, because of commit-then-reveal.
- **A replayed message** is detected by the spent-range ledger and flagged as possible reuse.

## Not covered

- Members can read everything in their circle. That is the design.
- The circle tag proves some member wrote a message, not which member. Proving a specific sender needs pairwise pads, which are out of scope for v1.
- A stolen or compromised device exposes that device's pad. A device PIN and a lockout are future work.
- Erasure on phone storage is best effort.
- Coercion of members.

## Claims the UI may make

- **Confidentiality.** For a `TRNG` circle: secure against unlimited computing power, as long as pad bytes are never reused. For a `CSPRNG` circle: computationally secure.
- **Integrity inside the circle.** Information-theoretic, through the one-time Poly1305 key.
- **The agent policy and the square log** rely on TVC attestation and signatures. That is ordinary modern cryptography. The pad is the layer that does not depend on math. TVC is the layer that governs the agent.

## Randomness

- Each device MUST generate its contribution from its hardware TRNG and run the NIST SP 800-90B continuous health tests (Repetition Count Test and Adaptive Proportion Test) on the raw output. A failed test aborts the ceremony.
- The ESP32-S3 hardware RNG is true random only while an entropy source runs. Firmware MUST enable the ADC noise source and keep the radio off.
- A YubiKey is never the only source. Its output is XORed with the device or platform source. The 2019 YubiKey FIPS series 4 reduced-randomness advisory (YSA-2019-02) is the reason.
- Dice rolls MAY be hashed and shown as an audit string. They MUST NOT be described as pad entropy.
