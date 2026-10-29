# Device tiers

A device tier describes protection against theft and tampering. It does not describe message encryption strength or prove that a device has been independently verified.

Every member holds the circle’s secret. One compromised member can expose it, so the least protected member limits the circle’s protection.

## Tiers at a glance

| Tier | Intended protection | Current position |
|---|---|---|
| **T0: software** | Runs the protocol on a phone or laptop. The operating system controls secret storage. | The browser app is T0. Erasure is best effort. |
| **T1: open microcontroller** | Uses raw storage and explicit overwrite of spent bytes. Firmware and flash remain accessible. | The unfused Nano ESP32 and current Pico 2 path belong here. Physical access can expose the pad. |
| **T2: sealed microcontroller** | Uses a verified boot/storage configuration to protect flash; a secure element can bind the spend ledger to a counter. | A development goal. No current Cahoots board has established this protection. |
| **T3: hardware key store** | Keeps wrapping keys inside a crypto engine and uses one-way counters for lockout and rollback protection. | A goal for the later Baochip design, not a current deployment. |
| **T4: inspected hardware** | Adds inspection of the silicon against its published design and firmware agreed by the circle. | A design target, not a verified property of the current badge. |

These are intended properties. A board does not earn a tier simply because it includes a particular chip. Storage, boot, counters, firmware, and physical testing all matter.

For T2, flash encryption alone does not prevent restoring an older ledger. Without a protected counter, lockout and rollback protection may also be rolled back. T3 does not rule out attacks on the die or flaws in its controller. T4 still depends on the inspection and what happens between inspection and use.

## What the circle records

The manifest records each seat’s declared tier, and the ceremony commitment binds it. A member cannot change that declaration after committing.

A declaration is still a claim. Chip attestation can add evidence about a seat key, but it does not attest to the complete device or its firmware. The screen for presenting the full tier information is planned.

## Randomness is a separate label

`TRNG` means every member contributed hardware randomness. `CSPRNG` means at least one used software randomness. This affects the pure-pad confidentiality claim; it does not turn a phone into a theft-resistant device.

A higher device tier does not make chat information-theoretically secure. See [Security and limits](security.md).

## Assurance for an operation

Assurance describes evidence that an owner authorized one act. It travels separately from the device tier.

| Level | Name | Meaning |
|---|---|---|
| 0 | Claimed | No authorization evidence was checked. |
| 1 | Key | A signature from a key acting under a grant. |
| 1, attested | Key, attested | Level 1 plus a secure-element proof about the seat key. |
| 2 | Present | Both badge buttons were pressed. |
| 3 | Present with PIN | Both buttons were pressed after PIN entry. |

A higher numbered level satisfies a request for a lower one. “Key, attested” adds evidence within level 1; it does not establish owner presence.

These levels describe the object/policy model, not a promise that every workflow is implemented. PIN entry and real-chip verification remain development work.

### Limits of attestation

The SE051 attestation design binds a key, chip identity, and fresh challenge to the chip maker’s certificate chain. It does not prove who is holding the badge or which application firmware is running.

Sharing attestations can let two circles recognize that they have seen the same chip. NXP’s root certificate is not bundled in the repository, and real-chip attestation is not verified today. The software-chip test root verifies only simulated devices, which must be reported as software.

For exact signed-object formats, see [the protocol](protocol.md#8-signed-objects). For hardware progress, see [Project status](status.md).
