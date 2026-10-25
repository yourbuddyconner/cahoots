# Adapters

These are thin crates that feed `cahoots-provider` output into standard pre-shared-key slots. Adapters never read the pad. They call `psk` / `psk_for` and derive a protocol-specific key with the `wireguard`, `hpke`, or `age` context, so a PSK released for one protocol can't be reused in another.

| Crate | Slot | Status |
|---|---|---|
| `hpke/` (`cahoots-hpke`) | HPKE RFC 9180 `mode_psk`, `psk_id = circle_id \|\| slice \|\| offset`. Suites: X25519 and X-Wing (X25519 + ML-KEM-768). | done (S1) |
| `wireguard/` (`cahoots-wireguard`, binary `cahoots-wg`) | WireGuard `PresharedKey`: issue for a peer seat, accept the token on the other, by paste or over the Numbers Station as control messages; `announce` and `peers` carry public keys and endpoints; `--apply` runs `wg set`; addresses derived from the circle ID. | done (S1, N1); `scripts/wg-demo.sh` brings up a two-host tunnel in CI |
| `age-plugin-cahoots/` | age plugin (C2SP age-plugin). Recipient `age1cahoots1...`, identity `AGE-PLUGIN-CAHOOTS-1...`, both encode a circle ID. The pad lives in `CAHOOTS_DIR`. | done (S1); tested through the plugin protocol with age-core's client connection |
| `meshtastic/` (`cahoots-meshtastic`, binary `cahoots-mesh`) | Meshtastic channel key: one member issues to everyone (`peer_b = 255`), each member accepts the same handle and derives the same AES-256 key. Prints the channel URL and the `--ch-set psk base64:` command. Rotation is issue again. | done (N4) |
| `tls-psk/` | TLS 1.3 external PSK importer (RFC 9258) | not started (S4) |
| `mls/` | MLS external PSK proposals (RFC 9420) | not started (S4) |

All four finished adapters are **PSK mode**: the data is computationally secure. Only pure-pad messages (`cahoots-core`) are one-time-pad encrypted.

## Try the age plugin

```sh
cargo install --path crates/age-plugin-cahoots     # puts age-plugin-cahoots on PATH
export CAHOOTS_DIR=~/.cahoots                        # a cahoots-provider directory
age-plugin-cahoots --circle <circle id hex>          # prints the recipient and identity
age -r age1cahoots1... -o secret.age secret.txt
age -d -i identity.txt secret.age
```

Each encryption spends 32 pad bytes on the sender. Each decryption spends the same 32 bytes on the reader, so a file can be decrypted once per member.
