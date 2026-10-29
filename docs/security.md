# Security and limits

Cahoots keeps circle keys on members’ devices. The relay delivers encrypted messages without receiving those keys.

That design does not protect against every attack. Cahoots is experimental; the protections below depend on the message mode, the ceremony, and the devices involved.

## Know which message mode you are using

| Mode | Protection | Important condition |
|---|---|---|
| **Send a message**: pure-pad mode | One-time-pad confidentiality with a one-time authentication key. | Pad bytes must be truly random, kept secret, and never reused. |
| **Chat** | Conventional ChaCha20-Poly1305 encryption with keys seeded from the pad. | This is computational security, not one-time-pad encryption. |
| File and network integrations | Conventional encryption using a key derived from the circle. | The integration’s security properties still apply. |

Information-theoretic claims apply only to pure-pad messages under the required conditions. They do not apply to ordinary chat, WireGuard, or encrypted files.

## The ceremony has its own protection

Devices encrypt their contributions with X-Wing before exchanging them. Someone recording the codes or wire traffic must break that encryption to recover contributions.

Comparing all six check words is essential. The devices refuse inconsistent commitments and signatures, but the people still need to confirm that their screens agree.

A `TRNG` circle requires hardware randomness from every device. A `CSPRNG` circle includes software randomness and has computational confidentiality. These labels do not describe protection against device theft. See [Device tiers](tiers.md).

A recorder of the ceremony is protected against by X-Wing, even when later pure-pad messages qualify for the stronger confidentiality claim. Do not describe the entire system as unconditionally secure.

## Every member can read the circle

Every member holds the whole pad. A malicious member can disclose messages and secrets.

The group authentication tag proves that a message came from someone with the circle secret. It does not prove which person sent it. Chat’s sender label also does not establish identity against another circle member.

Hello words use pair-specific keys to check a person on a call. They depend on X-Wing and access to the devices, not the pure-pad guarantee.

## Devices are part of the trust boundary

A compromised phone, browser, laptop, or badge can expose the secret it holds. Browser erasure is best effort, and plaintext history remains on the device.

Physical buttons do not prevent coercion. A secure-element design does not establish that the current hardware has been tested against theft or tampering.

Do not use two restored copies of the same send state. They may spend the same pad bytes. Do not expect deletion in the app to prove physical erasure from phone storage.

## The relay cannot promise delivery

A relay can observe traffic, drop messages, delay them, or stop responding. Its hash chain helps readers check history against hashes they already know; it does not by itself prevent an operator from presenting conflicting histories.

The enclave deployment and external log anchoring are planned. They are not a guarantee of the current relay.

## A hosted member receives the secret

A hosted software member gets its own copy of the pad. The circle then trusts its enclave and pinned policy with that copy. This option changes the trust boundary; it is not needed for a circle made only of people.

## Technical references

- [Threat model](threat-model.md): the detailed attack model and randomness requirements.
- [Protocol](protocol.md): ceremony, spending, authentication, and signed objects.
- [Chat specification](chat.md): key rotation, sender chains, and computational security.
- [Project status](status.md): which paths have been tested and which remain unfinished.
