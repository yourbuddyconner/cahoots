# Threat model

This reference describes the attackers Cahoots is designed to address and the limits of those protections. Start with [Security and limits](security.md) for a user-facing explanation.

## Scope of the claims

**Pure-pad messages:** confidentiality can be information-theoretic when the pad is truly random, secret, and never reused. Integrity uses a one-time Poly1305 authentication key. A `CSPRNG` circle has computational confidentiality.

**Chat and integrations:** chat keyed by the circle root, WireGuard, HPKE, and age use conventional cryptography. The pure-pad confidentiality and integrity claims do not apply to their payloads. See the [chat specification](chat.md) and [adapter reference](../crates/ADAPTERS.md).

**Ceremony recordings:** contributions are encrypted using X-Wing, which combines X25519 and ML-KEM-768. A recorder’s ability to recover a contribution depends on that encryption, even when the resulting pad qualifies for pure-pad claims.

## Network attackers and the relay

An outsider without circle keys cannot read or change authenticated messages undetected under the relevant mode’s assumptions. Message sizes, timing, public identifiers, and network activity remain visible.

The relay cannot decrypt messages or verify their circle tags. It can refuse overlapping pure-pad spend ranges at the address level, but clients must check authentication and their own ledgers.

A relay operator can delay, drop, or withhold messages. A reader can compare a history against a head hash it previously retained, but an ordinary hash-chained log does not independently prevent rollback or conflicting histories. External anchoring and the enclave deployment are planned, not current guarantees.

## Attacks during a ceremony

- **Choosing a contribution after seeing others:** commit-then-reveal prevents this. An honest random contribution prevents the other members from choosing the combined pad.
- **Replacing a seat key or tier after commitment:** each commitment binds the key and declared tier. Every seat signs the manifest. Conflicting signatures cause refusal to complete.
- **Submitting a bad signature first:** devices retain the first signature that verifies, not simply the first received.
- **Recording the screen or wire:** the recorder sees public commitments and keys plus encrypted reveals. Recovering the pad requires defeating the reveal encryption.
- **Substituting keys on a cable:** commitment checks reject inconsistent data. People must still compare the six check words before saving.

## Member and device compromise

Every member holds the whole pad and can disclose it. The circle authentication tag proves group membership, not an individual sender. Chat sender indices are also claims, not proof of identity against other members.

A stolen or compromised device can expose its pad, local history, and software-held signing keys. Secure-element-backed keys depend on the hardware implementation; real-chip bench verification is unfinished. Revocation and device PIN/lockout workflows are development work.

Browser erasure is best effort. Restoring old send state or using two copies can reuse pad ranges and invalidate security guarantees. The spent-range ledger rejects detected replays; it is not a substitute for safe state storage.

Coercion is outside the model: a button press under threat looks like an ordinary confirmation. Emanations and observation of decrypted screens are also outside the protections offered here.

## Signed objects and hello words

A forged grant or approval fails signature verification against the manifest. Delegated grants can only narrow permissions. An approval signs the fields the device displayed, rather than an unchecked caller-supplied hash. These use conventional P-256 signatures.

Hello words authenticate access to a pair key established during the ceremony. An outsider with only a voice clone or old recording cannot derive a fresh response under the pair-key assumptions. Someone with the unlocked device can read the words. Both participants must move to the same fresh hello number to stop reusing an observed response.

## Hosted members

A hosted software member receives a copy of its circle’s pad. Compromising its enclave exposes the circles it joined. Circles without such a member do not entrust their pad to the hosted-member service.

Enclave attestation, pinned policy, and log anchoring rely on conventional cryptography. Planned deployment does not establish those guarantees for the current relay.

## Randomness requirements

- Hardware devices MUST generate their contribution from the hardware TRNG and run the NIST SP 800-90B Repetition Count and Adaptive Proportion health tests on raw output. A failed test aborts the ceremony.
- The ESP32-S3 RNG needs an active entropy source. Firmware MUST enable the ADC noise source and keep the radio off.
- External hardware randomness is never the only source; XOR it with the device or platform source.
- Dice rolls MAY be hashed and displayed as an audit string. They MUST NOT be described as pad entropy.
- A circle qualifies as `TRNG` only when every member supplies the required hardware randomness. A per-device source label alone does not establish the circle’s label.

The [protocol](protocol.md) defines the exact ceremony and spending rules. [Project status](status.md) describes which implementation paths have been verified.
