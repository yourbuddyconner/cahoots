# Project status

This is a snapshot of the repository’s documented status on November 1, 2026. Cahoots is a development project, not a finished secure messaging service.

## What you can try

| Area | Available | Verification or limitation |
|---|---|---|
| Browser ceremony | Create a circle through the relay, the default: the host shows one code, each joiner scans it and picks a seat, the host scans each joiner's code, and the rounds go through the relay. Or with codes only, by animated QR or paste, with no network. | Automated end-to-end checks cover both ways, with codes pasted in place of the camera. Camera scanning remains unverified end to end. |
| Short messages | Encrypt directly with the pad and exchange by code or text. | Browser checks cover decryption and refusal of a replay. |
| iOS app | Cahoots for iPhone: relay and codes-only ceremonies, chat, hello words, the seat key in the Secure Enclave. A build has been uploaded to App Store Connect. | Two simulators hold a relay ceremony and chat through a local station (`scripts/ios-walk.sh`). A phone-hosted ceremony with a badge through the bridge has run with the Pico 2 on the bench and with a simulated badge (`scripts/bridge-bench.sh`, with and without `--sim`). The camera, Face ID and background refresh are not yet tried on a device. Pairing with a badge, by tap and press, is built: the app pairs with a badge's chip over NFC and keeps each of the badge's circles as a card circle. On 2026-11-01 a phone paired with the bench badge by tap, and the badge's two circles became card circles in the app. Since then a Card screen in Settings has one button, "Tap the card": one tap fetches every key a chat waits for, passes on the bumps the badge signed, tops up the words to eight hello numbers with each other seat, and keeps circles the badge kept after pairing. The words need no tap until they run low. `scripts/pair-bench.sh` runs the app's own pairing code on a laptop against the real chip: its second tap kept the bench badge's three circles in 6.2 seconds. The Card screen and hello words from the card have not run on a phone yet. The simulator has no NFC. |
| Group chat | Chat keyed by the circle root, over a Numbers Station relay. Keys come in generations, and any member can reset them for everyone. Each device publishes a card in the chat with its name, its face and its kind, and the phone and the browser show them for every member. | The app ships pointing at `relay.cahoots.network`, which is deployed. The end-to-end check chats through a local station. |
| Relay | Run the station locally, in Docker, or from the [deploy recipe](../deploy/relay/README.md). | Per-client rate limit and a log size cap are built in. No deployed enclave or external log anchoring. |
| WireGuard integration | Announce peers and exchange preshared keys. | A two-host tunnel is exercised in CI. |
| File encryption | age plugin and HPKE adapter. | Development integrations. Read their current [adapter notes](../crates/ADAPTERS.md). |

## Hardware progress

On 2026-11-04 the DIY badge on a perfboard joined a circle a browser hosted, over its USB cable, with the browser as the bridge: the page's "Add the badge on USB" took the badge's seat, and both showed the same six words. The same night the badge took a circle from the laptop's simulated pile and paired a phone by tap, after its wiring was fixed with a pin test the firmware now carries as a feature.

A breadboard Pico 2 has completed a ceremony with a laptop acting as the rest of the pile, saved a circle, and loaded it from flash. `scripts/bridge-bench.sh` puts the board in a ceremony a phone hosts, through `cahoots-host bridge`. On 2026-10-31 the board took seat 2 of 3 in a circle a simulator hosted, showed the same six words as the phones, and kept the circle. The same day, `scripts/pair-bench.sh` paired the laptop to the board as a phone, on the Pico 2 with its SE051, applet 7.2.0. The laptop ran the phone's chip client against the chip through a bench-only pass-through. In no session, the chip refused each of nine things a stranger's tap might try, from a MAC with a circle's root to a write over a public object. The laptop and the badge showed the same six words, and pressing both on the badge paired them. The laptop then opened the phone slot's secure channel, got an HMAC from a circle's root, and read a device grant and its own key in every circle's public object. After the badge unpaired it, the old key no longer opened the slot. On 2026-11-01 a phone paired with the board by tap, through a transistor that cuts the chip's power so the phone's field can start it.

The badge now rests on a home page: its circles by name, and whether a phone can tap. The left button opens a two minute window for a tap, which cuts the chip's power. Both buttons open the menu, and pairing takes four presses of both buttons. A note goes home by itself. On 2026-11-01 `scripts/pair-bench.sh` pressed through these pages on the board from home and ended "pair bench: ok". A pile kept a new circle, the chip refused an unpaired tap each of ten things in no session, and the laptop paired with the app's own pairing code. Its second tap kept the badge's three circles in 6.2 seconds: 83 commands, 32 of them hello MACs. Then the badge unpaired the laptop, and the script forgot the circle through the badge's own pages. The chip's memory sets how many circles a badge keeps in it: on the bench SE051 a circle of three seats takes about 3 KB. The badge keeps a placeholder at the public id of every circle slot, so a stranger's tap finds none of them empty, and that costs one circle: the bench chip holds five where it held six. The Pico has 16 places for circles, but that does not mean 16 circles on this chip. What waits for a person is the Card screen's tap on a phone, and hello words from the card.

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
