# Project status

This is a snapshot of the repository’s documented status on October 30, 2026. Cahoots is a development project, not a finished secure messaging service.

## What you can try

| Area | Available | Verification or limitation |
|---|---|---|
| Browser ceremony | Create a circle through the relay, the default: the host shows one code, each joiner scans it and picks a seat, the host scans each joiner's code, and the rounds go through the relay. Or with codes only, by animated QR or paste, with no network. | Automated end-to-end checks cover both ways, with codes pasted in place of the camera. Camera scanning remains unverified end to end. |
| Short messages | Encrypt directly with the pad and exchange by code or text. | Browser checks cover decryption and refusal of a replay. |
| iOS app | Cahoots for iPhone: relay and codes-only ceremonies, chat, hello words, the seat key in the Secure Enclave. Ready for its first TestFlight build. | Two simulators hold a relay ceremony and chat through a local station (`scripts/ios-walk.sh`). The camera, Face ID and background refresh are not yet tried on a device. Pairing with a badge waits for M4j. |
| Group chat | Chat keyed by the circle root, over a Numbers Station relay. | The app ships pointing at `relay.cahoots.network`, which is deployed; the end-to-end check chats through a local station. |
| Relay | Run the station locally, in Docker, or from the [deploy recipe](../deploy/relay/README.md). | Per-client rate limit and a log size cap are built in. No deployed enclave or external log anchoring. |
| WireGuard integration | Announce peers and exchange preshared keys. | A two-host tunnel is exercised in CI. |
| File encryption | age plugin and HPKE adapter. | Development integrations; read their current [adapter notes](../crates/ADAPTERS.md). |

## Hardware progress

A breadboard Pico 2 has completed a ceremony with a laptop acting as the rest of the pile, saved a circle, and loaded it from flash.

The V1 PCB, enclosure design, and firmware exist. The secure-element driver builds, and the chip APIs have software-chip tests. A real-chip ceremony, attestation against NXP’s root, and the related signing workflows still need bench verification.

The Baochip firmware target is a placeholder. Higher device tiers, PIN-based theft protection, and pad storage protected by the secure element are goals, not properties of the current browser or breadboard build.

## Work that remains

- Verify camera-based ceremonies and ceremonies between real badges.
- Bring up the V1 secure element and verify hardware attestation.
- Protect stored pad data and prevent rollback of the spend ledger on hardware.
- Complete hosted-member deployment, policy integration, and log anchoring.
- Finish revocation and the other lifecycle features listed in the roadmap.

The [development roadmap](plan.md) retains milestone IDs, acceptance criteria, and design decisions. It is useful when contributing, but it is not the recommended starting point for learning to use Cahoots.

For build and test commands, see the [repository README](../README.md).
