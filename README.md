# Cahoots

Cahoots makes encryption keys in person. A group meets once, builds a shared pad of random bytes with hardware badges or phones, and then spends those bytes as keys for the tools it already runs: WireGuard tunnels, encrypted files, group chat, approvals, and short messages that no amount of computing power can read. Keys rotate from the pad for years with no server in between.

## The mental model

A one-time pad is a file of random bytes that a group of people share. To encrypt a message, you XOR it with fresh pad bytes. To decrypt, you XOR it with the same bytes. Each byte is used once and then destroyed. If the pad is truly random and never reused, no amount of computing power can break the encryption. That includes quantum computers.

The hard part is getting the same random file to every member without anyone else seeing it. Cahoots does this in person, at a **ceremony**:

1. A group meets in a room. Each person holds a Cahoots badge or a phone.
2. Each device generates random bytes from its hardware random number generator.
3. The devices swap those bytes over a wired touch link, or by showing each other animated codes. Each contribution is encrypted to the other members before it leaves the device, so a camera or an antenna in the room learns nothing.
4. Every device XORs all contributions into one shared pad.
5. Every device signs the list of seats with a key it made for this group, so the group has a signed roster and every member a key to sign with later.
6. The members read six check words aloud to confirm every device holds the same pad.

![The ceremony in six steps](docs/diagrams/ceremony.svg)

The group is now a **cahoot** (a circle). The pad is a key budget. Spending 32 bytes seeds a session of any size in WireGuard, HPKE, age, or the pad-seeded chat. An attacker then has to break both the math and the pad. Spending one byte per byte encrypts a short message that nothing can break. The device records every spend in a ledger and erases the bytes, so no key is ever used twice.

![The pad: everyone holds all of it, each member sends from their own part](docs/diagrams/pad.svg)

Ciphertext travels over the **Numbers Station**, an append-only public feed. Anyone can read it. Only circle members can decrypt or forge an entry.

![Anyone can read the feed. Only the group can read the messages.](docs/diagrams/station.svg)

A circle can also give a seat to a hosted software member, such as an AI agent. That member's copy of the pad lives inside a [Turnkey Verifiable Cloud (TVC)](https://docs.turnkey.com/features/verifiable-cloud/overview) enclave, and a pinned policy decides each read and send. This integration is optional. Circles without a hosted seat have a simple guarantee: nobody who runs the feed can read the circle.

## Repository layout

Every Rust crate lives under `crates/`. The PWA lives under `web/app`.

| Path | Crate | What it holds | Status |
|---|---|---|---|
| `crates/core` | `cahoots-core` | The protocol: parameters, commit-then-reveal ceremony, XOR combine, check words, circle ID, message codec with one-time Poly1305, spent-range ledger, spend ordering, touch-link framing. `no_std` + `alloc`. | done (M1) |
| `crates/vectors` | `cahoots-vectors` | JSON test vectors generated from core. Every client must reproduce them. | done (M1) |
| `crates/objects` | `cahoots-objects` | The four objects a seat signs, as canonical CBOR in COSE_Sign1: grants, approvals, facts and receipts, each verified against the circle's signed manifest, with a JSON projection and vectors. `no_std`. | done (M1c) |
| `crates/custodian` | `cahoots-custodian` | The `Custodian` trait (`psk`, `spend`, `remaining`), the CBOR request and response messages, and `serve`, which answers a request only when the device consents. `no_std`. | done (M4e) |
| `crates/provider` | `cahoots-provider` | The memory and encrypted-file custodians, the HKDF session-key derivation, and the old `PadProvider` name for `Custodian`. | done (S1) |
| `crates/hpke` | `cahoots-hpke` | HPKE `mode_psk` adapter (X25519 and X-Wing suites). | done (S1) |
| `crates/age-plugin-cahoots` | `age-plugin-cahoots` | age plugin: recipient `cahoots:<circle>`. | done (S1), untested against the `age` CLI |
| `crates/wireguard` | `cahoots-wireguard` | WireGuard `PresharedKey` helper and the `cahoots-wg` CLI: announce, peers, issue, accept over the feed or by paste; `--apply` runs `wg set`. | done (S1, N1); `scripts/wg-demo.sh` runs a two-host tunnel in CI |
| `crates/relay` | `cahoots-relay` | Numbers Station client shared by the CLIs. | done |
| `crates/wasm` | `cahoots-wasm` | Browser bindings: ceremony, circle, fountain-coded animated QR, QR rendering. | done (M2) |
| `web/app` | | Offline PWA phone and laptop client, including hello words for calls. | done (M2), camera scanning untested |
| `crates/hal` | `cahoots-hal` | HAL traits, the ceremony driver over any set of touch links (a pile, a ring, a pair), pile discovery (seat and count from the stack), SP 800-90B health tests, persistent ledger. `no_std`. | done (M4a) |
| `crates/badge` | `cahoots-badge` | The badge crate: the `Board` trait and the badge program that runs on it. It boots, runs the ceremony, keeps the circle, and shows the words, a key as a code, or serves custodian requests. `no_std`. | done (M4e) |
| `crates/host` | `cahoots-host` | Desktop simulator: socket Link, file PadStore, PNG Display. Four processes, each with a socket to both neighbours, run a ceremony. | done (M4a) |
| `crates/esp32s3` | `cahoots-esp32s3` | `esp-hal` firmware for the Arduino Nano ESP32 with the 1.54" e-paper and two buttons: the four HAL traits on real hardware, the pile over USB with the laptop as the other cards. Built with the esp toolchain, outside the workspace. | in progress (M4b, first board) |
| `crates/rp2350` | `cahoots-rp2350` | `rp235x-hal` firmware for the Raspberry Pi Pico 2 on V1's pin map: the e-paper, two buttons, the chip's TRNG, the pad and ledger in flash through the boot ROM, USB to a laptop and two UARTs up and down a pile. | build-verified (M4b); not yet run on a board |
| `crates/dabao` | | Xous app for the Baochip-1x Dabao, the V2 card. | placeholder (M4c, M5) |
| `crates/chat` | `cahoots-chat` | Pad-seeded end-to-end chat: epoch keys, sender ratchets, ChaCha20-Poly1305, plus the `cahoots-chat` CLI demo. | done (C1, C3) |
| `crates/station` | `cahoots-station` | The Numbers Station relay: hash-chained log, reuse refusal, long-poll feed, display page, Dockerfile. | done (C2); [TVC](https://docs.turnkey.com/features/verifiable-cloud/overview) deploy pending |
| `crates/fuzz` | | `cargo-fuzz` targets for every parser. Not a workspace member. | done |
| `docs/` | | Protocol spec, threat model, use cases, plan, ceremony guide. | |

## Build and test

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build -p cahoots-core --no-default-features --target thumbv7em-none-eabi   # no_std check
cargo build -p cahoots-core --target wasm32-unknown-unknown
cargo run -p cahoots-vectors -- --check     # committed vectors are current
```

The pinned toolchain is in `rust-toolchain.toml`. CI runs these commands, with `--all-features` on clippy and test, and also builds `cahoots-hal` and `cahoots-chat` for `thumbv7em-none-eabi`.

## Security limits

- Confidentiality is information-theoretic only for circles labelled `TRNG`, and only while no pad byte is reused. `CSPRNG` circles are computationally secure.
- The circle tag proves that some member wrote a message, not which member.
- A stolen device exposes that device's pad.
- Erasure on phone storage is best effort.
- Circles that seat an agent trust the enclave with their pad.

See `docs/threat-model.md` for the full list.

## What it is for

`docs/tiers.md` defines the device tiers, from a phone (T0) to inspected silicon (T4), and the rule that a circle is only as protected as its least protected member. `docs/use-cases.md` lists concrete uses: group chat, and bootstrapping WireGuard and mesh networks from a ceremony. `docs/gatherings.md` is the concept of a card for every gathering: storage, messages from old circles, the web of trust a card accumulates, and a social network with no accounts. `docs/attested-data.md` surveys what a network of attested sensor readings, signed by the badge's key store chip, could be used for. `docs/diy-badge.md` is how to build a badge, in any of the three hardware families. `docs/plan.md` is the roadmap. `docs/glossary.md` defines the terms.

## Working rules

Correctness of the crypto is the product. Every spend path erases before output. Every tag compare is constant time. The message decoder is property-tested against arbitrary bytes. The code uses standards where they exist: Poly1305 (RFC 8439), BIP-39 words, HPKE (RFC 9180), and HKDF (RFC 5869).
