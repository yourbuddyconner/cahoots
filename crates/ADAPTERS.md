# Adapters

These are thin crates that feed the output of a custodian into standard pre-shared-key slots. Each adapter is a consumer of the `Custodian` contract in `cahoots-custodian`, and it reaches that contract through `cahoots-provider`, which re-exports it as `PadProvider`. Adapters never read the pad. They call `psk` / `psk_for`. WireGuard and age then derive a protocol-specific key with `derive_session_key` and a per-protocol context, so a PSK released for one protocol can't be reused in another. HPKE passes the 32 pad bytes to the key schedule as the PSK and binds the handle through `psk_id`.

| Crate | Slot | Status |
|---|---|---|
| `hpke/` (`cahoots-hpke`) | HPKE RFC 9180 `mode_psk`, `psk_id = circle_id \|\| slice \|\| offset`. Suites: X25519 and X-Wing (X25519 + ML-KEM-768). | done (S1) |
| `wireguard/` (`cahoots-wireguard`, binary `cahoots-wg`) | WireGuard `PresharedKey`: issue for a peer seat, accept the token on the other, by paste or over the Numbers Station as control messages; `announce` and `peers` carry public keys and endpoints; `--apply` runs `wg set`; addresses derived from the circle ID. | done (S1, N1); `scripts/wg-demo.sh` brings up a two-host tunnel in CI |
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

Each encryption spends 32 pad bytes on the sender. Each decryption spends the same 32 bytes on the reader, so a file can be decrypted once by each other member. The sender cannot decrypt its own file and keeps the plaintext.
