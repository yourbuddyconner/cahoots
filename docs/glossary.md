# Glossary

Terms used in the guides. For exact formats and requirements, use the [protocol specification](protocol.md).

## People and devices

**Circle / cahoot:** the group created by a ceremony, together with its shared secret and membership record. “Cahoot” is the name used in the app.

**Ceremony:** the exchange that creates a circle. Devices commit to their contributions, exchange encrypted reveals, sign the membership record, and show check words. See [Create a circle](ceremony-guide.md).

**Seat:** one device’s numbered place in the circle. A hosted program can also occupy a seat.

**Badge / card:** a dedicated Cahoots device. The hardware is still in development; see [Hardware](hardware.md).

**Hosted seat:** a software member with its own copy of the circle secret, held in an enclave under a pinned policy. It changes whom the circle trusts.

## Secrets and messages

**Pad:** the shared secret bytes. Every member holds the whole pad. Spending bytes consumes them; they must never be reused for a different operation.

**Contribution:** the random bytes supplied by one device. Contributions are combined to make the pad.

**Commitment:** a hash shared before a contribution is revealed. It prevents a member from changing their contribution after seeing the others.

**Sealed reveal:** a contribution encrypted for the other members. Recording the QR codes or wire traffic does not expose it in plaintext.

**Check words:** six words derived from the ceremony result. Every person must check that all devices show the same words in the same order.

**Slice:** the region of the pad assigned to one sender. Each member sends from its own slice, but every member has the full pad for receiving.

**Spend:** consuming pad bytes for a message or key. The device records the spend and erases the bytes before releasing the result.

**Ledger:** the record of consumed pad ranges. It helps prevent reuse and detect replays.

**Pure-pad mode:** encrypting a short message directly with fresh pad bytes, plus a one-time authentication key. Its strongest confidentiality claim requires truly random secret bytes that are never reused.

**PSK:** preshared key. Cahoots can spend 32 pad bytes to seed keys for conventional encryption. Chat and the file/network integrations use this approach.

**Seal / tag:** authentication data attached to a message. The circle tag establishes that someone with the shared secret created the message; it does not identify a particular person within the circle.

**Epoch:** one period of chat keys, started by a 32-byte pad spend. It can carry many chat messages.

**Relay / Numbers Station / feed:** the server and public log used to deliver encrypted messages. The relay does not have the circle’s keys. See [Run a relay](relay.md).

## Identity and security

**Circle ID:** a public identifier derived from the pad. The relay uses it to group encrypted messages.

**Seat key:** a separate P-256 signing key for one seat in one circle. It signs the manifest and signed objects. This is conventional public-key cryptography, not one-time-pad encryption.

**Manifest:** the membership record signed by every seat during the ceremony. It records keys, parameters, declared device tiers, and policy limits.

**Pair key:** a secret shared by exactly two members, established through X-Wing during the ceremony. It is separate from the group pad.

**Hello words:** words derived from a pair key and a hello number, used to check someone during a call. Both devices must use the same number. See [the browser guide](browser.md#check-someone-on-a-call).

**TRNG / CSPRNG:** hardware-random and software-random labels. A circle qualifies as `TRNG` only when every device contributed hardware randomness. The app also shows a per-device source label; that label alone does not establish the whole circle’s status.

**X-Wing / KEM:** X-Wing combines X25519 and ML-KEM-768. A KEM, or key encapsulation mechanism, lets devices establish encryption keys using public keys. Cahoots uses it to protect contributions and establish pair keys.

**Device tier:** a description of protections against theft and tampering, from T0 to T4. A declared tier is not proof that those protections were verified. See [Device tiers](tiers.md).

**Attestation:** a chip’s signed statement about a key it holds. It does not prove who holds the device or that its firmware is trustworthy. Real-chip attestation remains unverified in the current project.

**Assurance:** the evidence that an owner authorized an operation, distinct from the device’s tier. See [Device tiers](tiers.md#assurance-for-an-operation).

## Implementation terms

**Grant / approval / fact / receipt:** signed objects for delegated permissions, decisions on specific operations, readings, and spend records. See [protocol section 8](protocol.md#8-signed-objects).

**Floor:** policy limits in the manifest that outside policy cannot loosen.

**Circle root:** a secret derived from the ceremony’s reveal keys. It derives other keys, including contact-card sealing keys. It is not another name for the pad.

**Contact card:** signed member details such as names, handles, and endpoints. Publishing and displaying contact cards on badges is separate development work.

**HAL:** hardware abstraction layer: the interfaces for randomness, storage, display, and device links.

**Test vectors:** fixed inputs and expected outputs used to check that implementations agree.
