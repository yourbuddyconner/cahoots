# Cahoots

Cahoots is in-person key agreement and rotation for the protocols you already run. A group meets once, builds a shared pad of random bytes with hardware badges or phones, and then spends those bytes as keys: for WireGuard tunnels, encrypted files, group chat, approvals, and for short messages that no amount of computing power can read.

## The mental model

A one-time pad is a file of random bytes that a group of people share. To encrypt a message, you XOR it with fresh pad bytes. To decrypt, you XOR it with the same bytes. Each byte is used once and then destroyed. If the pad is truly random and never reused, no amount of computing power can break the encryption. That includes quantum computers.

The hard part is getting the same random file to every member without anyone else seeing it. Cahoots does this in person, at a **ceremony**:

1. A group meets in a room. Each person holds a Cahoots badge or a phone.
2. Each device generates random bytes from its hardware random number generator.
3. The devices swap those bytes over a wired touch link. The devices physically touch.
4. Every device XORs all contributions into one shared pad.
5. The members read six check words aloud to confirm every device holds the same pad.

The group is now a **cahoot** (a circle). The pad is a key budget. Spending 32 bytes seeds a session of any size in WireGuard, HPKE, age, or the pad-seeded chat, hedged so an attacker must break both the math and the pad. Spending one byte per byte encrypts a short message that nothing can break. Every spend is recorded in a ledger and the bytes are erased, so no key is ever used twice.

Ciphertext travels over the **Numbers Station**, an append-only public feed. Anyone can read it. Only circle members can decrypt or forge an entry.

A circle can also give a seat to a hosted software member, such as an AI agent. That member's copy of the pad lives inside a Turnkey Verifiable Cloud (TVC) enclave, and a pinned policy decides each read and send. This integration is optional. Circles without a hosted seat have a simple guarantee: nobody who runs the feed can read the circle.

## Repository layout

Every Rust crate lives under `crates/`. The PWA lives under `web/app`.

| Path | Crate | What it holds | Status |
|---|---|---|---|
| `crates/core` | `cahoots-core` | The protocol: parameters, commit-then-reveal ceremony, XOR combine, check words, circle ID, message codec with one-time Poly1305, spent-range ledger, spend ordering, touch-link framing. `no_std` + `alloc`. | done (M1) |
| `crates/vectors` | `cahoots-vectors` | JSON test vectors generated from core. Every client must reproduce them. | done (M1) |
| `crates/provider` | `cahoots-provider` | Pad provider API (`psk`, `spend`) with memory and encrypted-file backends, plus the HKDF session-key derivation. | done (S1) |
| `crates/hpke` | `cahoots-hpke` | HPKE `mode_psk` adapter (X25519 and X-Wing suites). | done (S1) |
| `crates/age-plugin-cahoots` | `age-plugin-cahoots` | age plugin: recipient `cahoots:<circle>`. | done (S1), untested against the `age` CLI |
| `crates/wireguard` | `cahoots-wireguard` | WireGuard `PresharedKey` helper and the `cahoots-wg` CLI. | done (S1), `wg` interop test pending |
| `crates/yk` | `cahoots-yk` | YubiKey OpenPGP `GET CHALLENGE` entropy source and CLI. | done (M2), throughput not yet measured |
| `crates/wasm` | `cahoots-wasm` | Browser bindings: ceremony, circle, fountain-coded animated QR, QR rendering. | done (M2) |
| `web/app` | | Offline PWA phone and laptop client. | done (M2), camera scanning untested |
| `crates/hal` | `cahoots-hal` | HAL traits, ring ceremony driver, SP 800-90B health tests, persistent ledger. `no_std`. | done (M4a) |
| `crates/host` | `cahoots-host` | Desktop simulator: socket Link, file PadStore, PNG Display. Four processes run a ring ceremony. | done (M4a) |
| `crates/esp32s3` | | `esp-hal` + Embassy firmware, first hardware target. | placeholder (M4b) |
| `crates/dabao` | | Xous app for the Baochip-1x Dabao. | placeholder (M4c, M5) |
| `crates/chat` | `cahoots-chat` | Pad-seeded end-to-end chat: epoch keys, sender ratchets, ChaCha20-Poly1305, plus the `cahoots-chat` CLI demo. | done (C1, C3) |
| `crates/station` | `cahoots-station` | The Numbers Station relay: hash-chained log, reuse refusal, long-poll feed, display page, Dockerfile. | done (C2); TVC deploy pending |
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

The pinned toolchain is in `rust-toolchain.toml`. CI runs the same commands.

## Security limits

- Confidentiality is information-theoretic only for circles labelled `TRNG`, and only while no pad byte is reused. `CSPRNG` circles are computationally secure.
- The circle tag proves that some member wrote a message, not which member.
- A stolen device exposes that device's pad.
- Erasure on phone storage is best effort.
- Circles that seat an agent trust the enclave with their pad.

See `docs/threat-model.md` for the full list.

## What it is for

`docs/tiers.md` defines the device tiers, from a phone (T0) to inspected silicon (T4), and the rule that a circle is only as protected as its least protected member. `docs/use-cases.md` lists concrete uses, from group chat to bootstrapping WireGuard and physical mesh networks from a ceremony. `docs/plan.md` has the milestones, including the network-bootstrap track.

## Working rules

Correctness of the crypto is the product. Every spend path erases before output. Every tag compare is constant time. The message decoder is property-tested against arbitrary bytes. Standards are used where they exist: Poly1305 (RFC 8439), BIP-39 words, HPKE (RFC 9180), and HKDF (RFC 5869).
