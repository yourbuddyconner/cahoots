# Seats, the manifest, and the signed objects: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every seat has a per-circle P-256 key bound into its commitment with its tier; the ceremony ends with every member signing one manifest; four signed objects (grant, approval, fact, receipt) exist as CBOR with COSE_Sign1 signatures, with vectors; every client keeps the seat key and the manifest.

**Architecture:** `cahoots_core` gains `seat`, `cose` and `manifest` modules and an `Attest` phase; a new `cahoots-objects` crate holds the four objects; the ring driver, the host, the badge, the Nano, the wasm crate and the app carry the new frame and store the new record fields.

**Tech Stack:** `p256` 0.13 (`ecdsa`, `arithmetic`, no default features), `minicbor` 2.3 (`alloc`, `derive`), `coset` as a dev dependency for the cross-check, existing crates.

**Spec:** `docs/superpowers/specs/2026-10-26-seats-manifest-objects.md`

## Global Constraints

- `cahoots-core`, `cahoots-hal`, `cahoots-badge`, `cahoots-custodian`, `cahoots-chat` and the new `cahoots-objects` stay `#![no_std]` + alloc, `#![forbid(unsafe_code)]`, and build with `--no-default-features --target thumbv7em-none-eabi`; `cahoots-core` and `cahoots-wasm` build for `wasm32-unknown-unknown`.
- Names, key numbers, domain strings and header values exactly as the spec: `COMMIT_DOMAIN = b"cahoots/commit/v2"`, `REVEAL_VERSION = 2`, protected header `{1: -7, 4: kid}`, manifest AAD `cahoots/manifest/v1`, object domains `cahoots/grant/v1`, `cahoots/approval/v1`, `cahoots/fact/v1`, `cahoots/receipt/v1`, nonce domain `cahoots/fact-nonce/v1`, `Frame::Attest = 5`, record magic `CAH2`, wasm state version 4.
- Canonical CBOR everywhere: definite lengths, shortest integers, integer map keys written ascending; every `decode` re-encodes and compares.
- Seat secrets are `Zeroizing`; no key material in logs, traces or `note_screen` lines.
- No em dashes in docs or comments. `cargo fmt --all --check`, `cargo clippy --workspace --all-features --all-targets -- -D warnings`, `cargo test --workspace --all-features`, the thumbv7em build (`cargo build -p cahoots-core -p cahoots-hal -p cahoots-chat -p cahoots-custodian -p cahoots-badge -p cahoots-objects --no-default-features --target thumbv7em-none-eabi`), the wasm build (`cargo build -p cahoots-core -p cahoots-wasm --target wasm32-unknown-unknown`) pass after each task; `cargo build --release` in `crates/esp32s3` (with `source ~/.cargo/esp-env.sh`) passes after Task 8 and later.

## Review Focus

1. A member cannot change its seat key or tier after committing: a reveal whose header carries a different seat key or tier than the commitment aborts with `CommitMismatch`. Test in Task 2.
2. Every member builds byte-identical manifest bytes, so attestations verify across members: a three-member ceremony where each member's `manifest()` equals the others'. Test in Task 3.
3. A grant chain can only narrow: a child with one extra purpose, a larger budget, a later `not_after`, the wrong `depth`, or a wrong `parent` is refused. Test in Task 5.
4. Our COSE_Sign1 is real COSE: `coset` verifies ours and we verify `coset`'s. Test in Task 1.
5. A late reveal still stands in for a missing commitment with the version 2 header, and an attestation arriving before our phase is `Attest` is buffered, not refused. Test in Task 3 (core) and Task 6 (ring).

---

### Task 1: Seat keys and COSE_Sign1

**Files:**
- Create: `crates/core/src/seat.rs`, `crates/core/src/cose.rs`
- Modify: `crates/core/Cargo.toml` (add `p256 = { version = "0.13", default-features = false, features = ["ecdsa", "arithmetic"] }`, `minicbor = { version = "2.3", default-features = false, features = ["alloc", "derive"] }`; dev-dependency `coset = "0.3"`), `crates/core/src/lib.rs` (`pub mod seat; pub mod cose;`)
- Test: `crates/core/tests/seat.rs`, `crates/core/tests/cose.rs`

**Interfaces:**
- Produces: `seat::{SeatKey, SeatPublic, SeatError}` and `cose::{sign1, verify1, kid1, CoseError}` exactly as spec §2 and §3.

- [ ] **Step 1: Failing tests.** `seat.rs`: `a_seed_round_trips_through_to_bytes`, `zero_seed_is_rejected`, `sign_and_verify`, `a_flipped_bit_fails_verification`, `public_key_is_33_bytes_compressed_and_parses`, `signatures_are_deterministic` (same msg twice gives equal bytes). `cose.rs`: `sign1_round_trips_embedded_payload`, `sign1_round_trips_detached_payload`, `wrong_external_aad_fails`, `protected_header_is_alg_es256_and_kid` (decode the first array element with minicbor and check `{1: -7, 4: kid}` exactly), `coset_verifies_ours`, `we_verify_cosets` (build a COSE_Sign1 with `coset::CoseSign1Builder` using `p256::ecdsa::SigningKey` from the same seed and the same protected header, then `verify1`), `extra_protected_header_entry_is_malformed`.
- [ ] **Step 2: Run, see them fail.**
- [ ] **Step 3: Implement** `seat.rs` with `p256::ecdsa::{SigningKey, VerifyingKey, Signature}` and `signature::{Signer, Verifier}`, `SeatKey` wrapping `SigningKey` with `Zeroizing` of the scalar bytes on drop; `cose.rs` writing the array and `Sig_structure` with minicbor (`Encoder` with `array`, `bytes`, `map`, `i8`), reading with `Decoder`.
- [ ] **Step 4: Run** the two test files, then the workspace checks and the thumbv7em and wasm builds. Expected: PASS.
- [ ] **Step 5: Commit** `core: seat keys (P-256) and COSE_Sign1 with a coset cross-check`.

---

### Task 2: Commitment and reveal, version 2

**Files:**
- Modify: `crates/core/src/commit.rs` (domain v2, `commit(params, index, kem_pk, seat_pk: &SeatPublic, tier, contribution)`, `verify_commit` likewise), `crates/core/src/reveal.rs` (version 2 header with `seat pk 33 | tier` after `kem pk`, `HEADER_LEN` updated, `seal` takes `seat_pk: &SeatPublic, tier: u8`, `Opened { from, public_key, seat_public, tier, contribution, shared }`, content AAD gains `seat_pk || tier`), `crates/core/src/ceremony.rs` (`new(params, index, contribution, kem_seed, seat_seed, tier)`, stores `seat_key`, `tier`, `seat_publics: Vec<Option<SeatPublic>>`, `tiers: Vec<Option<u8>>`; `add_commitment(from, c, kem_pk, seat_pk: &[u8], tier)`; `my_seat_public()`, `seat_public_of(i)`, `tier_of(i)`; `CeremonyError::BadSeed`; `add_reveal` verifies the commitment with the reveal's seat key and tier and, when standing in, stores them), `crates/core/tests/{commit,reveal,ceremony,proptests}.rs`
- Test: those files

**Interfaces:**
- Produces: the new `commit`, `seal`, `open`, `Ceremony::new` signatures and the `CeremonyError::BadSeed` variant. (Task 3 adds the attest phase; after this task a ceremony still completes at the last reveal.)

- [ ] **Step 1: Failing tests.** `commit.rs`: `commitment_binds_seat_key_and_tier` (changing either changes the hash) and the domain bytes. `reveal.rs`: `header_carries_seat_key_and_tier`, `opened_reports_them`, `a_changed_tier_in_the_header_fails_to_open` (AAD). `ceremony.rs`: `reveal_with_another_seat_key_than_committed_aborts_with_commit_mismatch`, `reveal_with_another_tier_than_committed_aborts`, `a_late_reveal_stands_in_and_records_seat_key_and_tier`, `bad_seed_is_reported`. Update every existing test for the new signatures.
- [ ] **Step 2: Run, see them fail. Step 3: Implement. Step 4: Run** core tests, workspace checks (the hal, host, wasm and vectors crates will fail to compile: fix their call sites minimally by passing `[0x11; 32]`-style seeds and tier 0 so the workspace builds; Tasks 6, 7 and 9 do the real wiring).
- [ ] **Step 5: Commit** `core: the commitment and the sealed reveal carry the seat key and the tier`.

---

### Task 3: The manifest and the attest phase

**Files:**
- Create: `crates/core/src/manifest.rs`
- Modify: `crates/core/src/lib.rs`, `crates/core/src/ceremony.rs` (Phase `Attest`; `manifest()`, `my_attestation()`, `add_attestation()`, `missing_attestations()`; `CeremonyError::{BadAttestation(u8), ConflictingAttestation(u8)}`; `CeremonyResult` gains `seat_key`, `tier`, `manifest: SealedManifest`), `crates/core/tests/ceremony.rs`, `crates/core/tests/proptests.rs`
- Test: `crates/core/tests/manifest.rs`, `crates/core/tests/ceremony.rs`

**Interfaces:**
- Produces: `manifest::{Manifest, Seat, FloorEntry, SealedManifest, ManifestError, Requirement}` as spec §5; `Manifest::default_floor(params) -> Vec<FloorEntry>`; the ceremony API of spec §4.3.

- [ ] **Step 1: Failing tests.** `manifest.rs`: `encode_is_canonical_and_decodes` (and a hand-built non-canonical byte string, for example a map key out of order, is `ManifestError::NotCanonical`), `id_is_sha256_of_the_bytes`, `default_floor_lists_every_purpose_at_present_with_slice_len_budget`, `sealed_manifest_verifies_with_n_attestations_and_refuses_n_minus_one`, `an_attestation_from_a_seat_not_in_the_roster_is_refused`, `two_attestations_from_one_seat_are_refused`. `ceremony.rs`: `three_members_build_identical_manifest_bytes`, `the_phase_is_attest_after_the_last_reveal_and_complete_after_the_last_attestation`, `an_attestation_before_attest_is_wrong_phase`, `a_bad_attestation_is_reported_and_does_not_abort`, `finish_returns_the_sealed_manifest_that_verifies`, `the_result_carries_the_seat_key_whose_public_is_in_the_roster`.
- [ ] **Step 2: Run, see them fail. Step 3: Implement. Step 4: Run** core tests and the workspace checks (fix the hal/host/wasm/vectors call sites minimally again, so that a ceremony is driven through attestation in their existing tests; the ring driver's real attest frame is Task 6, so for now their tests may call `my_attestation`/`add_attestation` directly where they hold several ceremonies in one process).
- [ ] **Step 5: Commit** `core: the manifest, and an attest phase in which every seat signs it`.

---

### Task 4: Ceremony vectors

**Files:**
- Modify: `crates/vectors/src/main.rs` (seat seeds from `fill`, tiers 0, 1, 0; the v2 commitments and reveals; `manifest_hex`, `manifest_id`, `attestations`), `crates/vectors/ceremony.json` (regenerated), `crates/core/tests/ceremony.rs` (the vector-reproduction test covers the new fields), `crates/vectors/README.md`
- Test: `crates/core/tests/ceremony.rs`

- [ ] **Step 1: Extend the reproduction test** for the new fields; run; see it fail against the old JSON. **Step 2: Regenerate** with `cargo run -p cahoots-vectors`. **Step 3: Run** the test and the workspace. **Step 4: Commit** `vectors: seat keys, version 2 commitments and reveals, the manifest and its attestations`.

---

### Task 5: The objects crate

**Files:**
- Create: `crates/objects/Cargo.toml` (`cahoots-objects`, features `default = ["std"]`, `std = ["cahoots-core/std", "dep:serde_json"]`), `crates/objects/src/{lib.rs,assurance.rs,grant.rs,approval.rs,fact.rs,receipt.rs,json.rs}`, `crates/objects/tests/{grant.rs,approval.rs,fact.rs,receipt.rs,vectors.rs}`
- Modify: `Cargo.toml` (workspace member and `cahoots-objects` dependency entry), `crates/vectors/src/main.rs` and `crates/vectors/objects.json` (new), `.github/workflows/*.yml` (add `-p cahoots-objects` to the thumbv7em build line)
- Test: the five test files

**Interfaces:**
- Produces: spec §6 exactly: `assurance::{CLAIMED, KEY, PRESENT, PRESENT_PIN}`; `Grant { circle_id, grantor: Grantor, grantee: SeatPublic, purposes: Vec<u8>, budget: u32, not_after: u64, depth: u8, parent: Option<[u8; 32]>, nonce: [u8; 16] }` with `Grantor::{Seat(u8), Key(SeatPublic)}`, `encode/decode/sign/verify_chain`, `GrantError`; `Approval`, `Fact` (`fact_nonce`), `Receipt` with `encode/decode/sign/verify`, `ObjectError`; `to_json` under `std`.

- [ ] **Step 1: Failing tests.** `grant.rs`: `a_root_grant_signed_by_a_seat_verifies_against_the_manifest`, `a_delegated_grant_narrows_and_verifies`, five refusals from Review Focus 3, `a_root_grant_from_a_key_not_in_the_roster_is_refused`, `purposes_must_be_ascending_without_repeats` (decode refuses), `encode_is_canonical`. `approval.rs`: `verifies_and_reports_the_fields_in_order`, `a_tampered_field_fails`, `assurance_above_present_pin_is_refused`. `fact.rs`: `nonce_is_recomputed_and_a_wrong_one_fails`, `attested_false_requires_empty_evidence`. `receipt.rs`: round trip and a signature from the wrong seat failing. `vectors.rs`: reproduce `objects.json`.
- [ ] **Step 2: Run, see them fail. Step 3: Implement**, generate `objects.json`. **Step 4: Run** the crate's tests, the workspace, the thumbv7em build including `-p cahoots-objects`. **Step 5: Commit** `objects: grants, approvals, facts and receipts as COSE-signed CBOR, with vectors`.

---

### Task 6: The ring driver

**Files:**
- Modify: `crates/hal/src/ring.rs` (`Frame::Attest = 5`; the payload table in the module doc; `new`/`new_buffered` gain `tier: u8` and draw the seat seed from `entropy` after the KEM seed, redrawing on `BadSeed`; `seen_attest`, `pending_attests`, `attested_self`; `maybe_attest()`; `handle` for `Attest`; `Step::Complete` only at `Phase::Complete`; `run` reports `attestations x/n`), `crates/hal/src/pile.rs` if it names the commit payload length, `crates/hal/tests/ring.rs`, `crates/host/src/{device.rs,pile.rs}` (pass tier 0)
- Test: `crates/hal/tests/ring.rs`

- [ ] **Step 1: Failing tests.** `three_devices_complete_through_attestation_and_the_manifests_agree`, `an_attest_frame_before_our_phase_is_attest_is_buffered_and_applied`, `an_attest_frame_with_a_bad_signature_does_not_abort_the_ring`, `run_reports_attestations`; update existing tests for the new constructor.
- [ ] **Step 2: Run, see them fail. Step 3: Implement. Step 4: Run** hal and host tests, the workspace, the thumbv7em build. **Step 5: Commit** `hal: the ring carries attestations; a ceremony completes when every seat has signed the manifest`.

---

### Task 7: The badge, the host board and the host device keep the seat key and the manifest

**Files:**
- Modify: `crates/badge/src/badge.rs` (`Circle { params, seat, circle_id, words, seat_key: [u8; 32], tier, manifest: Vec<u8> }`; the ceremony step passes `board.tier()` and keeps the new fields at the keep prompt), `crates/badge/src/board.rs` (docs only, if the record is described), `crates/badge/tests/common/mod.rs` and `crates/badge/tests/{ceremony,menu}.rs` (a kept circle has the seat key and a manifest that verifies), `crates/host/src/board.rs` (write and read the three new lines), `crates/host/src/device.rs` (`circle.txt` lines `seat_key=`, `tier=`, `manifest=`; `Device::open` reads them; `Device::manifest()`), `crates/host/tests/*.rs` as needed, `crates/host/README.md`
- Test: badge and host tests

- [ ] **Step 1: Failing tests.** badge `ceremony.rs`: `keeping_the_circle_stores_the_seat_key_and_a_manifest_that_verifies`, `the_manifest_lists_this_badge_at_its_tier`; host `badge.rs`: the circle files of the two simulated cards carry manifests with the same id. **Step 2: Run, see them fail. Step 3: Implement. Step 4: Run** badge, host, workspace, thumbv7em. **Step 5: Commit** `badge, host: the circle record keeps the seat key, the tier and the sealed manifest`.

---

### Task 8: The Nano's record

**Files:**
- Modify: `crates/esp32s3/src/store.rs` (magic `CAH2`, the record gains `seat_key 32 | tier u8 | manifest len u16 LE | manifest`; `CAH1` loads as no circle; the circle sector is 4 KiB, check the record fits for n = 8 and say the limit in a comment), `crates/esp32s3/src/board.rs` (`load_circle`/`save_circle`), `crates/esp32s3/README.md` (the record line)
- Verify: `source ~/.cargo/esp-env.sh && cargo build --release` in `crates/esp32s3`

- [ ] **Step 1: Baseline build (fails for the new fields). Step 2: Implement. Step 3: Build. Step 4: Commit** `esp32s3: the circle record keeps the seat key and the manifest (CAH2)`.

---

### Task 9: The phone app

**Files:**
- Modify: `crates/wasm/src/lib.rs` (`Ceremony::new(..., kem_seed, seat_seed, tier)`; `phase()` returns `"attest"`; `attestation() -> Vec<u8>`; `add_attestation(from, bytes)`; `missing_attestations()`; `finish` keeps the seat key seed, the tier and the sealed manifest in `Circle`; state version 4 with version 3 still loading; `has_seat_key()`, `manifest_id_hex()`), `web/app/app.js` (a third round: pack kind 3 `[3, index, cose...]`, text form `cahoots-attest:<i>:<b64url>`; `screenRevealDone` becomes the attest screen; a member whose ceremony is complete keeps showing the attestation; step titles 1 to 4; seat seed from `crypto.getRandomValues`; tier 0), `web/app/sw.js` (bump VERSION), `web/e2e.mjs` (drive the third round), `docs/web.md` if it lists the steps
- Test: `cargo test -p cahoots-wasm` (native tests, if any), `node web/e2e.mjs` after `npm install` in `web/` and `wasm-pack build --target web --release --out-dir ../../web/app/pkg --no-typescript` in `crates/wasm`

- [ ] **Step 1: Update `web/e2e.mjs`** for the third round; run; see it fail. **Step 2: Implement. Step 3: Run** the wasm build, the e2e script, the workspace. **Step 4: Commit** `wasm, app: the third round signs the manifest; the circle keeps its seat key`.

---

### Task 10: Docs

**Files:** `docs/protocol.md` (§2 unchanged; §3 the version 2 commitment and reveal, §3.4 "Attest" and the manifest; new §8 "Signed objects" with the four tables, the COSE profile, the JSON projection; §7 vectors list gains `objects.json`), `docs/tiers.md` (an "Assurance" section with the lattice), `docs/plan.md` (a row "M1c Seats and manifest" done, with what follows: consent-screen grants and approvals, Identify advertising, the log with checkpoints, sealing), `README.md` (one sentence on the objects crate in the crate list), `crates/objects/README.md` (new, short).

- [ ] **Step 1: Edit** in the plain voice. **Step 2: Commit** `docs: seats, the manifest, and the signed objects`.
