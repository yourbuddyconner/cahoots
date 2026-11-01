# Project status

This is a snapshot of the repository’s documented status on October 31, 2026. Cahoots is a development project, not a finished secure messaging service.

## What you can try

| Area | Available | Verification or limitation |
|---|---|---|
| Browser ceremony | Create a circle through the relay, the default: the host shows one code, each joiner scans it and picks a seat, the host scans each joiner's code, and the rounds go through the relay. Or with codes only, by animated QR or paste, with no network. | Automated end-to-end checks cover both ways, with codes pasted in place of the camera. Camera scanning remains unverified end to end. |
| Short messages | Encrypt directly with the pad and exchange by code or text. | Browser checks cover decryption and refusal of a replay. |
| iOS app | Cahoots for iPhone: relay and codes-only ceremonies, chat, hello words, the seat key in the Secure Enclave. Ready for its first TestFlight build. | Two simulators hold a relay ceremony and chat through a local station (`scripts/ios-walk.sh`). A phone-hosted ceremony with a badge through the bridge has run with the Pico 2 on the bench and with a simulated badge (`scripts/bridge-bench.sh`, with and without `--sim`). The camera, Face ID and background refresh are not yet tried on a device. Pairing with a badge, by tap and press, is built: the app pairs with a badge's chip over NFC and keeps each of the badge's circles as a card circle that taps for its keys. `scripts/pair-bench.sh` ran the phone's chip client on a laptop against the real chip: it paired, opened the phone slot's secure channel, got an HMAC from a circle's root, and found the slot closed after the badge unpaired it. A real tap from the app on a phone has not been tried. The simulator has no NFC. |
| Group chat | Chat keyed by the circle root, over a Numbers Station relay. Keys come in generations, and any member can reset them for everyone. Each device publishes a card in the chat with its name, its face and its kind, and the phone and the browser show them for every member. | The app ships pointing at `relay.cahoots.network`, which is deployed; the end-to-end check chats through a local station. |
| Relay | Run the station locally, in Docker, or from the [deploy recipe](../deploy/relay/README.md). | Per-client rate limit and a log size cap are built in. No deployed enclave or external log anchoring. |
| WireGuard integration | Announce peers and exchange preshared keys. | A two-host tunnel is exercised in CI. |
| File encryption | age plugin and HPKE adapter. | Development integrations; read their current [adapter notes](../crates/ADAPTERS.md). |

## Hardware progress

A breadboard Pico 2 has completed a ceremony with a laptop acting as the rest of the pile, saved a circle, and loaded it from flash. `scripts/bridge-bench.sh` puts the board in a ceremony a phone hosts, through `cahoots-host bridge`. On 2026-10-31 the board took seat 2 of 3 in a circle a simulator hosted, showed the same six words as the phones, and kept the circle. The same day, `scripts/pair-bench.sh` paired the laptop to the board as a phone, on the Pico 2 with its SE051, applet 7.2.0. The laptop ran the phone's chip client against the chip through a bench-only pass-through. In no session, the chip refused each of nine things a stranger's tap might try, from a MAC with a circle's root to a write over a public object. The laptop and the badge showed the same four words, and pressing both on the badge paired them. The laptop then opened the phone slot's secure channel, got an HMAC from a circle's root, and read a device grant and its own key in every circle's public object. After the badge unpaired it, the old key no longer opened the slot. The chip's memory sets how many circles a badge keeps in it: on the bench SE051 each circle takes about 2.7 KB, so an empty chip holds about seven circles. The bench chip held six before it refused a seventh with "the chip is full. Forget a circle first." The Pico has 16 places for circles, but that does not mean 16 circles on this chip. What waits for a person is a real tap: the app on a phone, held to the board's antenna, pairing and then reading a card circle's chat.

The V1 PCB, enclosure design, and firmware exist. The secure-element driver builds, and the chip APIs have software-chip tests. A ceremony with the seat key in the real chip, and signing with it, have run on the bench. Attestation against NXP’s root still needs bench verification.

The Baochip firmware target is a placeholder. Higher device tiers, PIN-based theft protection, and pad storage protected by the secure element are goals, not properties of the current browser or breadboard build.

## Work that remains

- Verify camera-based ceremonies and ceremonies between real badges.
- Bring up the V1 secure element and verify hardware attestation.
- Protect stored pad data and prevent rollback of the spend ledger on hardware.
- Complete hosted-member deployment, policy integration, and log anchoring.
- Finish revocation and the other lifecycle features listed in the roadmap.

The [development roadmap](plan.md) retains milestone IDs, acceptance criteria, and design decisions. It is useful when contributing, but it is not the recommended starting point for learning to use Cahoots.

For build and test commands, see the [repository README](../README.md).
