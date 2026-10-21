# Adapters

Thin crates that feed `cahoots-provider` output into standard PSK slots (handoff 10.2). Adapters never read the pad. They call `psk` / `psk_for` and derive a protocol-specific key with the `wireguard`, `hpke`, or `age` context, so a PSK released for one protocol can't be reused in another.

| Crate | Slot | Status |
|---|---|---|
| `hpke/` (`cahoots-hpke`) | HPKE RFC 9180 `mode_psk`, `psk_id = circle_id \|\| slice \|\| offset`. Suites: X25519 and X-Wing (X25519 + ML-KEM-768). | done (S1) |
| `wireguard/` (`cahoots-wireguard`, binary `cahoots-wg`) | WireGuard `PresharedKey`: issue on one peer, accept the handle token on the other, render `[Peer]` or pipe into `wg set`. Rotation schedule helper. | done (S1); `wg` interop test is `#[ignore]` until `wireguard-tools` is installed |
| `age-plugin-cahoots/` | age plugin (C2SP age-plugin). Recipient `age1cahoots1...`, identity `AGE-PLUGIN-CAHOOTS-1...`, both encode a circle ID. The pad lives in `CAHOOTS_DIR`. | done (S1); tested through the plugin protocol with age-core's client connection |
| `tls-psk/` | TLS 1.3 external PSK importer (RFC 9258) | not started (S4) |
| `mls/` | MLS external PSK proposals (RFC 9420) | not started (S4) |

All three finished adapters are **PSK mode**: the data is computationally secure. Only pure-pad messages (`cahoots-core`) are one-time-pad encrypted.

## Try the age plugin

```sh
cargo install --path crates/age-plugin-cahoots     # puts age-plugin-cahoots on PATH
export CAHOOTS_DIR=~/.cahoots                        # a cahoots-provider directory
age-plugin-cahoots --circle <circle id hex>          # prints the recipient and identity
age -r age1cahoots1... -o secret.age secret.txt
age -d -i identity.txt secret.age
```

Each encryption spends 32 pad bytes on the sender. Each decryption spends the same 32 bytes on the reader, so a file can be decrypted once per member.
