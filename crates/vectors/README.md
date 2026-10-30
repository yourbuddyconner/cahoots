# Test vectors

Generated from `cahoots-core` and `cahoots-objects`. Regenerate with `cargo run -p cahoots-vectors`. CI runs `cargo run -p cahoots-vectors -- --check` and fails if these files are stale.

| File | Covers |
|---|---|
| `ceremony.json` | Params encoding, per-seat P-256 keys and tiers, version 2 commitments and sealed reveals, each member's attestation, the manifest body and its ID, the sealed manifest, combined pad, check words, circle ID, a reveal that must abort |
| `checkwords.json` | Check words and circle IDs for several pads |
| `messages.json` | Seal and open for every message type, plus a tag failure |
| `spend.json` | Two sends, sender erasure, receiver replay rejection, ledger contents |
| `frames.json` | Touch-link framing with CRC-32 |
| `objects.json` | The four signed objects: a root grant and a delegated grant, an approval, a fact and a receipt, and a sign grant with an agent's proof, each with its inputs, canonical CBOR payload, COSE_Sign1 and JSON projection, against a three-seat manifest |
| `root.json` | The circle root from the content keys of the ceremony in `ceremony.json`, chat keys for four epochs, and seal keys for two labels, among them the card seal key |
| `ssh.json` | Requests to sign for ssh and git: four logins (bound to a P-256 host key, host-bound, unbound, bound to another session) and a git commit, each with the badge's screen lines and the signature it returns |

All byte strings are lowercase hex.

## `ceremony.json` fields

Per-member fields are arrays with one entry per member, in seat order: `seat_seeds` (32-byte P-256 seeds), `seat_publics` (33-byte compressed public keys), `tiers` (0, 1, 0), `commitments` and `sealed_reveals` (version 2), and `attestations` (detached COSE_Sign1 over the manifest body). Per ceremony: `manifest` (the canonical body), `manifest_id` (SHA-256 of the body), and `sealed_manifest` (the body with every attestation). The test `the_vectors_file_is_reproduced_from_its_inputs` in `crates/core/tests/ceremony.rs` rebuilds all of them from the seeds.

## `objects.json` fields

`manifest` is the canonical body of a three-seat circle whose keys come from `seat_seeds`. Every object is signed by `seat` (1) under kid `[1]`, except the delegated grant, signed by `delegate_seed` under kid SHA-256 of its public key. Each object has `inputs` (every field), `payload` (canonical CBOR), `signed` (the COSE_Sign1 with the payload embedded) and `json` (the informative projection). The test in `crates/objects/tests/vectors.rs` rebuilds all of them from the inputs. `sign_grant` is a sign grant (protocol section 8.3) signed by `seat` for the key of `agent_seed`, with its claims as `payload` and the base its counter starts at as `inputs.base`, and `sign_grant.proof` is that key's proof over a fixed hash and nonce.

## `root.json` fields

`content_keys` are the three reveal content keys of the ceremony in `ceremony.json`, in seat order, and `circle_id` is that ceremony's circle id. `root` is HKDF-SHA256 over the content keys concatenated, with the circle id as salt and `cahoots/root/v1` as info. `chat_keys` gives HMAC-SHA256(root, `chat` || circle_id || epoch as 8 bytes big-endian) for each `epoch`, and `seal_keys` gives HMAC-SHA256(root, label || circle_id) for each `label`. The test `the_root_vectors_are_reproduced` in `crates/core/tests/root.rs` rebuilds them.

## `ssh.json` fields

A badge holds one circle, `bench`, as seat 0, with the seat key from `seat_seed`; `seat_blob` is that key as an SSH public key blob. Each case is one `SshSign` request: `data` (the bytes OpenSSH asks an agent to sign), `host` (the session binding the laptop sends, or null; its host key comes from `host_seed`) and `content` (the commit text for git, or null). `screen` is the badge's consent screen, line by line, with the footer last, and `signature` is the SSH signature blob the badge returns. The test in `crates/badge/tests/ssh_vectors.rs` sends each request through the badge's own checks and compares both.
