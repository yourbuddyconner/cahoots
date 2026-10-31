# Security and limits

Cahoots keeps circle keys on members’ devices. The relay delivers encrypted messages without receiving those keys.

That design does not protect against every attack. Cahoots is experimental; the protections below depend on the message mode, the ceremony, and the devices involved.

## Know which message mode you are using

| Mode | Protection | Important condition |
|---|---|---|
| **Send a message**: pure-pad mode | One-time-pad confidentiality with a one-time authentication key. | Pad bytes must be truly random, kept secret, and never reused. |
| **Chat** | Conventional ChaCha20-Poly1305 encryption with keys derived from the circle root. | This is computational security, not one-time-pad encryption. Chat is as secret as the root: anyone who compromises one member's device can read every message the relay still holds. |
| **Channels** | Conventional ChaCha20-Poly1305 encryption with keys the owner's device draws, and a signature on every post. | Never pad-hedged. Anyone holding the link can read until the owner retires it. |
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

## A channel is ordinary encryption

A [channel](browser.md#channels) is conventional cryptography. Its keys are random bytes the owner's device draws, never pad, and nothing about a channel carries the pure-pad claim, whatever the circle that owns it.

- **Inside a set of keys,** each sender's message keys move forward with every message and the used ones are erased, so one message key opens one message. The device still holds the channel key for its last four sets of keys, so whoever takes the device can read every message of those sets that the relay still keeps. Older sets, whose keys the device has dropped, stay closed.
- **At new keys,** a member who was revoked, or a link that was retired, gets nothing sealed for it. Whoever held them, or stole them, stops reading from that point. The app starts new keys whenever the owner revokes someone or retires a link. New keys alone do not shut out a thief who still holds a live link or a member's keys: the owner must retire that link or revoke that member.
- **Who posted:** every post is signed by its member's key, which a grant ties to the owner circle. Readers can tell which member key posted each message, which circle chat cannot do. The owner sees beside each post the name that person typed when they asked to post, which nobody checks; other readers see a short key id. A channel does not offer anonymous posting.
- **What a link holder sees:** everything any reader sees, for as long as the link works. Anyone the holder forwards it to sees the same, and a link alone cannot count or expire its readers.
- **Who governs:** in this version one seat of the owner circle, on its own, can start new keys, revoke a member, add or retire a link, and replace the channel object. The approval count of a circle with several seats applies only to changing the policy. Give a seat only to someone you trust to run the channel alone.

- **Where keys come from:** every key deposit is signed by the owner's member key, and readers install only deposits whose signer's grant lets it govern the channel. Someone who knows the link, or a relay that lies, cannot hand readers keys of its own.

- **What the relay sees:** channel ids, message sizes and timing, which member key posted each message, which link tokens read and when, and the join queue: each request's name, key and what it asks for. It holds no channel key and cannot read messages, key deposits or link secrets. It can still delay, drop or refuse, as for circles.
- **Who lets people in:** the owner's device, while the app is open there. A relay that checks joiners from an enclave, so that admission works with the owner's phone closed, is defined and not built.

## A hosted member receives the secret

A hosted software member gets its own copy of the pad. The circle then trusts its enclave and pinned policy with that copy. This option changes the trust boundary; it is not needed for a circle made only of people.

## Technical references

- [Threat model](threat-model.md): the detailed attack model and randomness requirements.
- [Protocol](protocol.md): ceremony, spending, authentication, and signed objects.
- [Chat specification](chat.md): key rotation, sender chains, and computational security.
- [Project status](status.md): which paths have been tested and which remain unfinished.
