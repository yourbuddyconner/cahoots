# Test vectors

Generated from `cahoots-core`. Regenerate with `cargo run -p cahoots-vectors`. CI runs `cargo run -p cahoots-vectors -- --check` and fails if these files are stale.

| File | Covers |
|---|---|
| `ceremony.json` | Params encoding, per-seat P-256 keys and tiers, version 2 commitments and sealed reveals, each member's attestation, the manifest body and its ID, the sealed manifest, combined pad, check words, circle ID, a reveal that must abort |
| `checkwords.json` | Check words and circle IDs for several pads |
| `messages.json` | Seal and open for every message type, plus a tag failure |
| `spend.json` | Two sends, sender erasure, receiver replay rejection, ledger contents |
| `frames.json` | Touch-link framing with CRC-32 |

All byte strings are lowercase hex.

## `ceremony.json` fields

Per-member fields are arrays with one entry per member, in seat order: `seat_seeds` (32-byte P-256 seeds), `seat_publics` (33-byte compressed public keys), `tiers` (0, 1, 0), `commitments` and `sealed_reveals` (version 2), and `attestations` (detached COSE_Sign1 over the manifest body). Per ceremony: `manifest` (the canonical body), `manifest_id` (SHA-256 of the body), and `sealed_manifest` (the body with every attestation). The test `the_vectors_file_is_reproduced_from_its_inputs` in `crates/core/tests/ceremony.rs` rebuilds all of them from the seeds.
