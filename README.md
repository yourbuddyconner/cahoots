# Cahoots

Cahoots is a set of small hardware badges and a phone app for private group messaging with one-time pads.

## The mental model

A one-time pad is a file of random bytes that a group of people share. To encrypt a message, you XOR it with fresh pad bytes. To decrypt, you XOR it with the same bytes. Each byte is used once and then destroyed. If the pad is truly random and never reused, no amount of computing power can break the encryption. That includes quantum computers.

The hard part is getting the same random file to every member without anyone else seeing it. Cahoots does this in person, at a **ceremony**:

1. A group meets in a room. Each person holds a Cahoots badge or a phone.
2. Each device generates random bytes from its hardware random number generator.
3. The devices swap those bytes over a wired touch link. The devices physically touch.
4. Every device XORs all contributions into one shared pad.
5. The members read six check words aloud to confirm every device holds the same pad.

The group is now a **cahoot** (a circle). Any member can encrypt a short message and post it anywhere, including the public **Numbers Station** feed. Only circle members can read it. Any change to it is detected.

A circle can also give a seat to an AI agent. The agent's copy of the pad lives inside a Turnkey Verifiable Cloud (TVC) enclave, and a pinned policy decides each read and send. Circles without an agent seat have a simple guarantee: even Turnkey can't read your circle.

## Repository layout

| Path | Crate | What it holds | Status |
|---|---|---|---|
| `core/` | `cahoots-core` | The protocol: parameters, commit-then-reveal ceremony, XOR combine, check words, circle ID, message codec with one-time Poly1305, spent-range ledger, spend ordering, touch-link framing. `no_std` + `alloc`. | **M1 done** |
| `vectors/` | `cahoots-vectors` | JSON test vectors generated from core. Every client must reproduce them. | **M1 done** |
| `provider/` | `cahoots-provider` | Pad provider API (`psk`, `spend`) over device, phone, file, and TVC backends. | trait only (S1) |
| `firmware/hal/` | `cahoots-hal` | `Entropy`, `PadStore`, `Display`, `Link` traits. | traits only (M4a) |
| `firmware/host/` | `cahoots-host` | Desktop simulator target. | placeholder (M4a) |
| `firmware/esp32s3/` | | `esp-hal` + Embassy firmware, first hardware target. | placeholder (M4b) |
| `firmware/dabao/` | | Xous app for the Baochip-1x Dabao. | placeholder (M4c, M5) |
| `web/` | | Offline PWA phone client, core compiled to WASM. | placeholder (M2) |
| `tvc-app/` | | Cahoots TVC workload: agent seat, policy, square log, vouching. | placeholder (M3) |
| `adapters/` | | HPKE, age, TLS-PSK, MLS, WireGuard adapters feeding provider output into standard PSK slots. | placeholder (S1, S4) |
| `docs/` | | Protocol spec, threat model, ceremony guide. | |

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

## Working rules

Correctness of the crypto is the product. Every spend path erases before output. Every tag compare is constant time. The message decoder is property-tested against arbitrary bytes. Standards are used where they exist: Poly1305 (RFC 8439), BIP-39 words, HPKE (RFC 9180), and HKDF (RFC 5869).
