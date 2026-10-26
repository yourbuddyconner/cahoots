# Test vectors

Generated from `cahoots-core` and `cahoots-objects`. Regenerate with `cargo run -p cahoots-vectors`. CI runs `cargo run -p cahoots-vectors -- --check` and fails if these files are stale.

| File | Covers |
|---|---|
| `ceremony.json` | Params encoding, per-seat P-256 keys and tiers, version 2 commitments and sealed reveals, each member's attestation, the manifest body and its ID, the sealed manifest, combined pad, check words, circle ID, a reveal that must abort |
| `checkwords.json` | Check words and circle IDs for several pads |
| `messages.json` | Seal and open for every message type, plus a tag failure |
| `spend.json` | Two sends, sender erasure, receiver replay rejection, ledger contents |
| `frames.json` | Touch-link framing with CRC-32 |
| `objects.json` | The four signed objects: a root grant and a delegated grant, an approval, a fact and a receipt, each with its inputs, canonical CBOR payload, COSE_Sign1 and JSON projection, against a three-seat manifest |

All byte strings are lowercase hex.

## `ceremony.json` fields

Per-member fields are arrays with one entry per member, in seat order: `seat_seeds` (32-byte P-256 seeds), `seat_publics` (33-byte compressed public keys), `tiers` (0, 1, 0), `commitments` and `sealed_reveals` (version 2), and `attestations` (detached COSE_Sign1 over the manifest body). Per ceremony: `manifest` (the canonical body), `manifest_id` (SHA-256 of the body), and `sealed_manifest` (the body with every attestation). The test `the_vectors_file_is_reproduced_from_its_inputs` in `crates/core/tests/ceremony.rs` rebuilds all of them from the seeds.

## `objects.json` fields

`manifest` is the canonical body of a three-seat circle whose keys come from `seat_seeds`. Every object is signed by `seat` (1) under kid `[1]`, except the delegated grant, signed by `delegate_seed` under kid SHA-256 of its public key. Each object has `inputs` (every field), `payload` (canonical CBOR), `signed` (the COSE_Sign1 with the payload embedded) and `json` (the informative projection). The test in `crates/objects/tests/vectors.rs` rebuilds all of them from the inputs.
