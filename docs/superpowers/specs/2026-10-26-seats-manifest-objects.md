# Seats, the manifest, and the signed objects: design

*Written 2026-10-26. Conner asked for Cahoots to be shaped so that a circle is something a decision service can register, a badge press is an assurance a policy can demand, and a reading from a chip the room met is a fact a policy can use, without Cahoots itself knowing what a policy is. The reasoning is in the session notes "Cahoots in an ecosystem"; this page is the part that is being built now. He said the ceremony may change freely: this is a V1.*

## 1. Summary

Every seat gets a per-circle signing key, made at the ceremony and bound into its commitment with a declared device tier. The ceremony gains a last phase in which every member signs the same manifest: the roster of seat keys and tiers, the circle's parameters, and a floor table. The signed manifest is the circle's root document, the one artefact an outside system registers. A new crate defines four signed objects a seat emits, grants, approvals, facts and receipts, as CBOR with COSE signatures, with a JSON projection and test vectors. The badge, the simulator, the Nano and the phone app all run the new ceremony and keep the seat key and the manifest with the circle.

Out of scope here, planned next: grants and approvals on the badge's consent screen, the Identify answer advertising profiles and assurance, the hash-chained log with checkpoints, sealing objects under pad-derived period keys, chip-held keys (M4f).

## 2. Seat keys

A seat key is a P-256 ECDSA key pair. P-256 because the key store chip planned for V1 generates and signs P-256 natively and because ES256 is the signature every token and web stack accepts; the post-quantum hybrid is open decision 16. The seat secret is made from 32 bytes of the device's randomness at the ceremony, one key per circle, never reused across circles.

`cahoots_core::seat`:

- `SeatKey::from_seed(seed: &[u8; 32]) -> Result<SeatKey, SeatError>`; the seed is the scalar, rejected if zero or not below the curve order (`SeatError::BadSeed`), in which case the caller draws another.
- `SeatKey::public(&self) -> SeatPublic`; `SeatPublic(pub [u8; 33])`, SEC1 compressed.
- `SeatKey::sign(&self, msg: &[u8]) -> [u8; 64]`: deterministic ECDSA (RFC 6979) over SHA-256 of `msg`, raw `r || s`.
- `SeatPublic::verify(&self, msg: &[u8], sig: &[u8; 64]) -> bool`; `SeatPublic::parse(&[u8]) -> Result<SeatPublic, SeatError>` rejects a byte string that is not a valid compressed point.
- `SeatKey` zeroizes on drop; `SeatKey::to_bytes(&self) -> Zeroizing<[u8; 32]>` for storage, `from_seed` reads it back.

## 3. COSE_Sign1 profile

Every signature Cahoots emits is a COSE_Sign1 (RFC 9052) so that any COSE library verifies it. `cahoots_core::cose`:

- Structure: the untagged CBOR array `[protected: bstr, unprotected: {}, payload: bstr / nil, signature: bstr]`.
- Protected header, canonical CBOR: `{1: -7, 4: kid}`; `1` is `alg`, `-7` is ES256, `4` is `kid`, a byte string the caller chooses. For a seat it is the single byte of the seat index. For a grantee key it is the SHA-256 of the key's 33 bytes.
- `Sig_structure = ["Signature1", protected, external_aad, payload]`, encoded as canonical CBOR, signed with `SeatKey::sign`.
- `sign1(key: &SeatKey, kid: &[u8], payload: &[u8], external_aad: &[u8], detached: bool) -> Vec<u8>`.
- `verify1(public: &SeatPublic, bytes: &[u8], detached_payload: Option<&[u8]>, external_aad: &[u8]) -> Result<Vec<u8>, CoseError>` returns the payload; `CoseError::{Malformed, Algorithm, Signature}`. The protected header must decode to exactly `alg` ES256 plus `kid`, any other content is `Malformed`; `kid1(bytes) -> Result<Vec<u8>, CoseError>` reads the kid without verifying.
- A test cross-checks a vector against the `coset` crate (dev dependency, std only) in both directions: ours verifies theirs and theirs verifies ours.

Canonical CBOR throughout means RFC 8949 §4.2.1: definite lengths, shortest integer encodings, map keys sorted by their encoded bytes. `minicbor` writes definite lengths and shortest integers; the map key order is the encoder's responsibility and every map in this design has integer keys written in ascending order.

## 4. The ceremony

### 4.1 Commitment, version 2

`COMMIT_DOMAIN = b"cahoots/commit/v2"`.

`commit(params, index, kem_pk, seat_pk, tier, contribution) = SHA-256(domain || params.encode() || index || kem_pk || seat_pk || tier || contribution)`, with `seat_pk` the 33 bytes and `tier` one byte.

The tier is the device's declared tier, 0 to 4 as in the tiers page, a claim like the sender index. It is in the commitment so nobody can change their declared tier or their key after committing.

Ring `Commit` frame payload: `index u8 | commitment 32 | kem public key 1216 | seat public key 33 | tier u8`. The pile layer and the phone app carry the same bytes after the index.

### 4.2 Sealed reveal, version 2

`REVEAL_VERSION = 2`. Header: `version u8 | from u8 | kem pk 1216 | seat pk 33 | tier u8 | k u8`. The content AAD gains `seat pk || tier` after `pk_from`. `Opened` gains `seat_public: SeatPublic` and `tier: u8`. A reveal that stands in for a missing commitment derives the commitment from all of these, as before.

### 4.3 The attest phase

`Phase` becomes `Commit, Reveal, Attest, Complete, Aborted`. When the last reveal folds in, the phase is `Attest`, not `Complete`.

- `Ceremony::new(params, index, contribution, kem_seed, seat_seed: [u8; 32], tier: u8)`; `BadSeed` from the seat key maps to `CeremonyError::BadSeed`.
- `Ceremony::manifest(&self) -> Option<&[u8]>`: the canonical manifest bytes, available from `Attest` on, built once.
- `Ceremony::my_attestation(&self) -> Option<Vec<u8>>`: `cose::sign1(seat_key, &[index], manifest, b"cahoots/manifest/v1", detached = true)`.
- `Ceremony::add_attestation(&mut self, from: u8, bytes: &[u8]) -> Result<(), CeremonyError>`: `WrongPhase` before `Attest`; verifies with `from`'s seat key against our manifest bytes and that the kid is `[from]`; a bad signature is `CeremonyError::BadAttestation(from)` and does not abort; a different valid attestation for a seat already held is `ConflictingAttestation(from)`; our own is accepted as a no-op. When all `n` are held the phase is `Complete`.
- `Ceremony::missing_attestations(&self) -> Vec<u8>`.
- `CeremonyResult` gains `seat_key: SeatKey`, `tier: u8`, and `manifest: SealedManifest`.

Ring driver: `Frame::Attest = 5`, payload `index u8 | COSE_Sign1 bytes`; forwarded and deduplicated like reveals; buffered if it arrives before our phase is `Attest`; the driver sends ours as soon as the phase is `Attest`. `Step::Complete` only once the ceremony is `Complete`. `run` reports `attestations x/n`. `RingCeremony::new(params, index, entropy, links, tier)` draws the seat seed from `entropy` after the KEM seed, redrawing on `BadSeed`.

Phone app: a third animated code after the reveal, pack kind 3, text form `cahoots-attest:<index>:<base64url>`. A member whose ceremony is complete keeps showing the attestation for members still collecting, as it does for the reveal. Steps are numbered 1 to 4: commit, reveal, sign, words.

## 5. The manifest

`cahoots_core::manifest`. The body is a canonical CBOR map:

| key | name | value |
|---|---|---|
| 0 | version | 1 |
| 1 | circle_id | bstr, 16 bytes |
| 2 | params | bstr, `CircleParams::encode()` |
| 3 | seats | array of seat maps, in seat order |
| 4 | floor | array of floor entries, in purpose order |
| 5 | genesis | uint, the log position of the ceremony, 0 |

A seat map: `{0: seat u8, 1: key bstr 33, 2: tier u8, 3: evidence bstr}`. `evidence` is empty in this version and will hold a chip's proof that the key lives in it.

A floor entry: `{0: purpose u8, 1: requirement u8, 2: budget u32}`. `requirement`: 0 never, 1 a grant suffices, 2 the owner must be present. The default floor written at the ceremony has every purpose in `cahoots_core::control::Purpose` at requirement 2 with budget `params.slice_len()`.

`Manifest { circle_id, params, seats: Vec<Seat>, floor: Vec<FloorEntry>, genesis: u64 }` with `encode() -> Vec<u8>` and `decode(&[u8]) -> Result<Manifest, ManifestError>`; decode rejects anything non-canonical by re-encoding and comparing, as the custodian wire does. `Manifest::id(&self) -> [u8; 32]` is SHA-256 of the canonical bytes. `Manifest::seat_key(&self, seat: u8) -> Option<&SeatPublic>`.

`SealedManifest { body: Vec<u8>, attestations: Vec<Vec<u8>> }` with `verify(&self) -> Result<Manifest, ManifestError>`: decodes the body, requires exactly `params.n` attestations, each a COSE_Sign1 with detached payload = body and external AAD `cahoots/manifest/v1`, kid `[seat]`, verified by that seat's key, every seat present once. `encode()`/`decode()` as a CBOR array `[body, [attestations...]]`.

## 6. The signed objects

`crates/objects`, `cahoots-objects`, `#![no_std]` + alloc, `#![forbid(unsafe_code)]`, depends on core. Every object is a canonical CBOR map signed as a COSE_Sign1 with the payload embedded, external AAD = the object's domain string. A signed object is `Vec<u8>`. Each type has `encode`, `decode` (canonical check), `sign(key, kid) -> Vec<u8>`, and a `verify` that takes the manifest.

### 6.1 Assurance

`cahoots_objects::assurance`: `CLAIMED = 0` (nothing checked), `KEY = 1` (a signature from a key under a grant), `PRESENT = 2` (both buttons on a badge), `PRESENT_PIN = 3` (both buttons after a PIN). A lattice: higher includes lower. The device tier travels separately.

### 6.2 Grant

Domain `cahoots/grant/v1`.

| key | value |
|---|---|
| 0 | version 1 |
| 1 | circle_id bstr 16 |
| 2 | grantor: uint seat, or bstr 33 key for a delegated grant |
| 3 | grantee key bstr 33 |
| 4 | purposes: array of uint, ascending, no repeats |
| 5 | budget uint, bytes |
| 6 | not_after uint, log position |
| 7 | depth uint, delegations still allowed below this grant |
| 8 | parent: bstr 32, SHA-256 of the parent's signed bytes, or nil for a root grant |
| 9 | nonce bstr 16 |

A root grant is signed by a seat key with kid `[seat]`. A delegated grant is signed by the parent's grantee key with kid = SHA-256 of that key. `verify_chain(chain: &[&[u8]], manifest: &Manifest) -> Result<Grant, GrantError>` takes the chain root first: the root's signer is a seat in the manifest; each link's `parent` is the hash of the previous signed bytes, its signer is the previous grantee, its `purposes` is a subset, its `budget` and `not_after` are not greater, its `depth` is exactly the previous depth minus one, and the previous depth was not zero. Returns the leaf.

### 6.3 Approval

Domain `cahoots/approval/v1`.

| key | value |
|---|---|
| 0 | version 1 |
| 1 | circle_id |
| 2 | seat uint |
| 3 | profile tstr, the operation's profile name |
| 4 | fields: array of `[label tstr, value tstr]` pairs, in the order shown |
| 5 | request_digest bstr 32 |
| 6 | policy_id bstr |
| 7 | verdict bool |
| 8 | assurance uint |
| 9 | log uint |
| 10 | nonce bstr 16 |

Signed by the seat with kid `[seat]`. The fields are the lines the badge rendered, so the evidence says what the person saw. `verify(bytes, manifest) -> Result<Approval, ObjectError>`.

### 6.4 Fact

Domain `cahoots/fact/v1`.

| key | value |
|---|---|
| 0 | version 1 |
| 1 | circle_id |
| 2 | seat |
| 3 | module tstr, the catalogue entry that explains the reading |
| 4 | reading bstr |
| 5 | counter uint |
| 6 | nonce bstr 32 |
| 7 | log uint |
| 8 | attested bool |
| 9 | evidence bstr, the chip's attested-read response, empty when `attested` is false |

`fact_nonce(circle_id, log) = SHA-256("cahoots/fact-nonce/v1" || circle_id || log as 8 bytes LE)`; `verify` recomputes it. Signed by the seat.

### 6.5 Receipt

Domain `cahoots/receipt/v1`.

| key | value |
|---|---|
| 0 | version 1 |
| 1 | circle_id |
| 2 | seat |
| 3 | slice uint |
| 4 | offset uint |
| 5 | len uint |
| 6 | purpose uint |
| 7 | grant bstr 32 or nil |
| 8 | log uint |

Signed by the seat. Sealing receipts to the circle is the next slice; this version defines the clear form.

### 6.6 JSON projection

With the `std` feature, each object has `to_json(&self) -> serde_json::Value`, informative, never verified: a grant becomes `{"authorization_details": [{"type": "cahoots/purpose", "purpose": n, "circle": hex, "budget": b}], "cnf": {"jwk": {"kty": "EC", "crv": "P-256", "x": b64url, "y": b64url}}, "exp_log": not_after, "depth": d, "parent": hex or null}`; an approval, a fact and a receipt become flat objects with the same field names as the tables, byte strings as hex, with `"seat"` and `"circle"`.

### 6.7 Vectors

`crates/vectors/objects.json`: one seat key seed, its public key, one root grant and one delegated grant with their signed bytes, one approval, one fact, one receipt, each with the canonical payload hex and the COSE hex. `ceremony.json` gains per member the seat seed, the seat public key, the tier, the version 2 commitment and reveal, the manifest body hex, the manifest id, and each attestation. Both are generated by `cargo run -p cahoots-vectors` and reproduced by tests in core and objects.

## 7. Keeping the key and the manifest

- `cahoots_badge::Circle` gains `seat_key: [u8; 32]` (the seed, plaintext on a tier 1 board, as the pad is), `tier: u8`, and `manifest: Vec<u8>` (the encoded `SealedManifest`). Boards store and load them with the record. The Nano's record magic becomes `CAH2`; a `CAH1` record is treated as no circle. The host board writes them as further lines in its circle file.
- The host `Device` writes `seat_key=`, `tier=` and `manifest=` lines to `circle.txt`.
- The wasm `Circle` state version becomes 4, carrying the seat key seed, the tier and the sealed manifest; version 3 states still load with no seat key, and `has_seat_key()` says so.
- `RingCeremony` and the badge's ceremony step pass `Board::tier()` as the declared tier; the host is 0, the phone is 0, the Nano is 1.

## 8. Docs

`docs/protocol.md` §3 gains the version 2 commitment and reveal, the attest phase and the manifest; a new §8 "Signed objects" holds the tables above; `docs/tiers.md` gains the assurance lattice; `docs/plan.md` records this as done under a new row and notes the slices that follow; the site's docs list needs no change.
