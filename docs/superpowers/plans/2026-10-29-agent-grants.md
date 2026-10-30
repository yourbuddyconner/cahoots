# Agent grants the badge consumes: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A person issues an agent a grant on the badge once; the badge then signs that agent's git commits under the grant without a press, counts each use in the chip, and stops at the budget, the expiry or a revocation.

**Architecture:** The grant is the existing version 2 CWT with `aud`, `exp` and one `cahoots:sign` authorization detail; the objects crate gains a second grant kind and an AuthZEN-shaped evaluator with no board dependency. The badge keeps grant records and a monotonic best-known time in a new flash region, a counter per grant in the chip, a nonce per link, and answers a new `SshSignUnderGrant` request by evaluating, incrementing, signing and drawing. The host gets an agent key, `grant` commands, and an agent that sends the grant and a proof with each git signature.

**Tech Stack:** Rust (workspace pinned 1.94, `no_std` + alloc in objects/chip/custodian/badge), minicbor for the wire, the core crate's COSE_Sign1 (`cahoots_core::cose::{sign1, verify1}`), p256 via `SeatKey`, the SE051 driver's counters, the host's `cahoots-host` CLI and tests with the simulated badge and `SoftChip`.

**Spec:** `docs/superpowers/specs/2026-10-29-agent-grants-design.md`

## Global Constraints

- Every crate that the badge builds stays `no_std` + alloc; no new dependencies in objects, chip, custodian or badge.
- Copy on screens is plain words; anything the badge cannot check is marked `(laptop)`.
- `authorization_details` stays at private CWT claim key -65539; `aud` is key 3 and `exp` key 4 (RFC 8392).
- A sign grant has exactly one detail, type `cahoots:sign`, actions `["git-commit"]` only in this version, `budget` 1..=1000, and no `cahoots-log` claim; a pad grant is unchanged.
- `GRANTS_MAX` = 8 live grants per badge; a ninth request answers `Denied`.
- The count of uses is read from the chip (`counter_get`), never from flash, on a badge with a chip.
- `known_time` never decreases; it is written to flash only when a grant is issued or used and the stored value is more than 3600 s behind.
- Decoders are strict: an unknown field, a missing required field, or a non-canonical encoding is `Malformed`.
- Commit after every task; `cargo test --workspace` and `cargo clippy --workspace --all-targets` clean before each commit.

## Review Focus

- A proof signed over a previous nonce (a replayed request) must be refused even though the grant is valid; Task 5's test `a_replayed_proof_is_refused`.
- A commit whose author line matches but whose committer line is someone else must still sign (the grant names the author); Task 5's test `committer_may_differ_from_author`.
- A restored flash image with an older grants record must not refund uses on a badge with a chip; Task 5's test `a_restored_record_does_not_refund_the_chip_counter`.
- A laptop time earlier than `known_time` must not un-expire a grant; Task 5's test `an_earlier_laptop_time_does_not_lower_known_time`.
- A grant request with `standing: true` *and* an `exp` is contradictory and must be refused before the screen; Task 5's test `standing_with_an_expiry_is_refused`.

---

### Task 1: Two kinds of grant claims in the objects crate

**Files:**
- Modify: `crates/objects/src/cwt.rs` (the `Detail` and `Claims` types, `encode`, `decode`, `meaning`)
- Modify: `crates/objects/src/grant.rs` (`to_v2`, `from_v2`)
- Test: `crates/objects/tests/cwt_sign.rs` (new)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  ```rust
  pub struct Detail { pub r#type: String, pub budget: Option<u32>,
                      pub actions: Option<Vec<String>>, pub author: Option<String>,
                      pub locations: Option<Vec<String>>, pub agent: Option<String> }
  pub struct Claims { pub iss: String, pub sub: String, pub aud: Option<String>, pub exp: Option<u64>,
                      pub cnf: SeatPublic, pub cti: [u8; 16], pub authorization_details: Vec<Detail>,
                      pub log_not_after: Option<u64>, pub parent: Option<[u8; 32]>, pub depth: u8, pub grantor: Grantor }
  pub const SIGN_TYPE: &str = "cahoots:sign";
  pub const AUD_PREFIX: &str = "cahoots:seat:";           // "cahoots:seat:<circle hex>:<seat>"
  pub fn audience(circle_id: &CircleId, seat: u8) -> String;
  pub fn seat_of(aud: &str) -> Option<(CircleId, u8)>;
  pub enum Kind { Pad { purposes: Vec<u8>, budget: u32 }, Sign(SignDetail) }
  pub struct SignDetail { pub actions: Vec<String>, pub author: String, pub locations: Vec<String>, pub agent: String, pub budget: u32 }
  impl Claims { pub fn kind(&self) -> Result<(CircleId, Kind), ObjectError>; }
  ```
  Detail map keys are written in canonical text-key order: `actions`, `agent`, `author`, `budget`, `locations`, `type`. Claim keys in canonical order: 1, 2, 3, 4, 7, 8, then -65537 down. A pad grant encodes exactly as before (byte-identical: the existing `objects.json` vectors must still pass).

- [ ] **Step 1: Write the failing tests** in `crates/objects/tests/cwt_sign.rs`:
  - `a_sign_grant_round_trips`: build `Claims` with `aud: Some(audience(&cid, 2))`, `exp: Some(1_764_374_400)`, one `Detail { r#type: SIGN_TYPE, actions: Some(vec!["git-commit"]), author: Some("Conner Swann <me@connerswann.me>"), locations: Some(vec!["github.com/yourbuddyconner/cahoots"]), agent: Some("claude on connerswann-mbp"), budget: Some(50), ..}`, `log_not_after: None`; assert `Claims::decode(&c.encode()) == Ok(c)` and `c.kind()` is `Kind::Sign` with those fields and the circle.
  - `a_pad_grant_encodes_as_before`: take the pad grant from `crates/vectors`'s `objects.json` (read the file from `../vectors/objects.json`, key `grant_v2.payload` hex) and assert `Claims::decode` then `encode` reproduces the bytes, and `kind()` is `Kind::Pad`.
  - `a_sign_grant_with_a_log_claim_is_malformed`, `a_pad_grant_without_a_log_claim_is_malformed`, `a_mixed_grant_is_malformed` (a sign detail plus a pad detail), `a_sign_grant_with_two_details_is_malformed`, `an_unknown_detail_field_is_malformed` (hand-encode a detail map with key `"foo"`), `a_sign_grant_without_budget_is_malformed`, `an_aud_that_is_not_a_seat_is_malformed`.
- [ ] **Step 2: Run** `cargo test -p cahoots-objects --test cwt_sign` → FAIL (fields do not exist).
- [ ] **Step 3: Implement.** Add the fields and constants above; make `LOG_NOT_AFTER` optional in `decode`; write `aud` (key 3, tstr) and `exp` (key 4, uint) in `encode` between `sub` and `cti`; extend `read_details`/`write_details` with the four optional text-keyed fields; replace `meaning()` with `kind()`: a pad grant is `log_not_after: Some`, last detail `PAD_TYPE`, `aud: None`, `exp: None`; a sign grant is one `SIGN_TYPE` detail with `actions` (non-empty, each in `["git-commit","ssh-login","file"]`), `author` (non-empty), `budget` in 1..=1000, `agent` (may be empty), `locations` (may be empty), `aud: Some` naming the `iss` circle, `log_not_after: None`, `depth == 0`; anything else `Malformed`. Keep `Grant::from_v2` for pad grants (`Kind::Pad`) and make it return `Malformed` for a sign grant.
- [ ] **Step 4: Run** `cargo test -p cahoots-objects` and `cargo test -p cahoots-vectors` → PASS (the existing vectors unchanged).
- [ ] **Step 5: Commit** `objects: a grant is one of two kinds: a pad grant as before, or a sign grant with aud, exp and one cahoots:sign detail`.

### Task 2: The sign grant, the evaluator and the proof

**Files:**
- Create: `crates/objects/src/authz.rs`
- Modify: `crates/objects/src/lib.rs` (`pub mod authz;`)
- Test: `crates/objects/tests/authz.rs` (new)

**Interfaces:**
- Consumes: Task 1's `Claims`, `Kind`, `SignDetail`; `cahoots_core::cose::{sign1, verify1, kid1}`, `SeatKey`, `SeatPublic`, `key_id`.
- Produces:
  ```rust
  pub const PROOF_DOMAIN: &str = "cahoots/proof/v1";
  #[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Action { GitCommit, SshLogin, File }
  impl Action { pub fn name(self) -> &'static str; pub fn parse(s: &str) -> Option<Action>; }
  pub struct SignGrant { pub circle_id: CircleId, pub seat: u8, pub grantee: SeatPublic, pub cti: [u8; 16],
                         pub actions: Vec<Action>, pub author: String, pub locations: Vec<String>,
                         pub agent: String, pub budget: u32, pub exp: Option<u64> }
  impl SignGrant {
      pub fn claims(&self) -> Claims;                                  // grantor Seat(seat), depth 0, parent None
      pub fn from_claims(c: &Claims) -> Result<SignGrant, ObjectError>;
      pub fn sign<S: SeatSigner>(&self, key: S, seat: u8) -> Vec<u8>;  // COSE_Sign1, kid [seat], grant::DOMAIN
      pub fn verify(signed: &[u8], seat_key: &SeatPublic, circle_id: &CircleId, seat: u8) -> Result<SignGrant, ObjectError>;
  }
  pub struct Subject { pub key_id: [u8; 32] }
  pub struct Resource { pub circle_id: CircleId, pub location: Option<String> }
  pub struct Context { pub author: String, pub time: Option<u64>, pub used: u64, pub revoked: bool }
  pub struct EvalRequest { pub subject: Subject, pub action: Action, pub resource: Resource, pub context: Context }
  #[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Reason { Allowed, WrongKey, WrongCircle, WrongAction, WrongAuthor, WrongLocation, Expired, Spent, Revoked }
  pub struct Decision { pub decision: bool, pub reason: Reason, pub remaining: u32 }
  pub fn evaluate(grant: &SignGrant, req: &EvalRequest) -> Decision;
  pub fn location_matches(granted: &str, given: &str) -> bool;       // strip scheme, "git@", ":"→"/", trailing ".git" and "/"; case-insensitive host
  pub fn proof_sign<S: SeatSigner>(key: S, data_hash: &[u8; 32], nonce: &[u8; 16]) -> Vec<u8>;   // payload CBOR [bstr32, bstr16], kid = key_id(public)
  pub fn proof_verify(proof: &[u8], cnf: &SeatPublic, data_hash: &[u8; 32], nonce: &[u8; 16]) -> bool;
  ```
  `evaluate` checks in this order and stops at the first failure: `revoked` → Revoked; key id ≠ `key_id(grantee)` → WrongKey; circle ≠ → WrongCircle; action not in `actions` → WrongAction; `author` ≠ (exact string) → WrongAuthor; `locations` non-empty and (`location` None or none matches) → WrongLocation; `exp` Some and `time` Some and `time >= exp` → Expired; `used >= budget` → Spent; else Allowed with `remaining = budget - used - 1`. `exp` Some with `time` None is Allowed (the badge supplies its time; a verifier without one cannot judge).

- [ ] **Step 1: Write the failing tests** in `crates/objects/tests/authz.rs`: `claims_round_trip` (`SignGrant` → `claims()` → `from_claims` equal, and `Claims::decode(encode)`); `sign_and_verify` (a `SeatKey::from_seed`, `sign`, `verify` with the right and the wrong seat key, the wrong circle, the wrong seat); `evaluate_allows_and_counts_down` (budget 2, used 0 → remaining 1, used 1 → 0, used 2 → Spent); one test per `Reason` using a request that differs in exactly that field; `location_matches_normalises` (`"https://github.com/a/b.git"`, `"git@github.com:a/b.git"`, `"github.com/a/b"` all match `"github.com/a/b"`, and `"github.com/a/c"` does not); `proof_round_trip_and_replay` (verify true with the same nonce, false with another nonce, another hash, another key).
- [ ] **Step 2: Run** `cargo test -p cahoots-objects --test authz` → FAIL (module missing).
- [ ] **Step 3: Implement** `crates/objects/src/authz.rs` per the interfaces. The proof payload is `[data_hash, nonce]` as a CBOR array of two byte strings, signed with `sign1(key, &key_id(public), payload, PROOF_DOMAIN, false)`; `proof_verify` uses `kid1` to check the kid is `key_id(cnf)` and `verify1` under `cnf`, then decodes the payload and compares both fields.
- [ ] **Step 4: Run** `cargo test -p cahoots-objects` → PASS.
- [ ] **Step 5: Commit** `objects: the sign grant, an AuthZEN-shaped evaluator, and the agent's proof of possession`.

### Task 3: A grant counter in the chip

**Files:**
- Modify: `crates/chip/src/ids.rs` (`KeyKind::Grant = 4`)
- Modify: `crates/chip/src/ops.rs` (`counter_get` on `ChipOps`)
- Modify: `crates/chip/src/soft.rs` (the software chip; find it with `grep -rn "impl ChipOps for SoftChip"`)
- Modify: `crates/chip/src/se05x.rs` (`counter_get` via `read_object`, big-endian 8 bytes, as `counter_next` already reads back)
- Test: `crates/chip/tests/counter.rs` (new, or the existing soft-chip test file if one tests counters: `grep -rln counter_next crates/chip/tests`)

**Interfaces:**
- Produces: `fn counter_get(&mut self, id: KeyId) -> Result<u64, ChipError>;` on `ChipOps` (`NoSuchKey` if absent), and `KeyKind::Grant`.

- [ ] **Step 1: Write the failing test** `counter_get_reads_without_incrementing`: `put_counter(id, 0)`, `counter_get == 0`, `counter_next == 1`, `counter_get == 1`, `counter_get` of a missing id is `Err(NoSuchKey)`; id is `KeyId::new(KeyKind::Grant, 3)` and `ids()` lists it.
- [ ] **Step 2: Run** `cargo test -p cahoots-chip --test counter` → FAIL.
- [ ] **Step 3: Implement** in the trait, the software chip and the SE051 driver (the driver: `exists` then `read_object`, refuse a length other than 8).
- [ ] **Step 4: Run** `cargo test -p cahoots-chip` and `cd crates/rp2350 && cargo build --release --features bench` → PASS / builds.
- [ ] **Step 5: Commit** `chip: counter_get reads a counter without moving it; a Grant key kind`.

### Task 4: The wire: grant requests and answers

**Files:**
- Modify: `crates/custodian/src/wire.rs` (`ChipRequest`, `ChipResponse`, `needs_consent`, `length_ok`)
- Modify: `crates/custodian/src/contract.rs` (`ChipApps` gains three methods with `NoChip` defaults on `Pad`)
- Modify: `crates/custodian/src/serve.rs` (dispatch)
- Test: `crates/custodian/tests/wire.rs` (add to the existing round-trip tests; `ls crates/custodian/tests`)

**Interfaces:**
- Produces, on `ChipRequest`:
  ```rust
  #[n(6)] Nonce,
  #[n(7)] GrantRequest { #[n(0)] action: String, #[n(1)] author: String, #[n(2)] location: Option<String>,
                         #[n(3)] agent: String, #[n(4)] budget: u32, #[n(5)] exp: Option<u64>, #[n(6)] standing: bool,
                         #[cbor(n(7), with = "minicbor::bytes")] cnf: [u8; 33], #[n(8)] time: u64 },
  #[n(8)] SshSignUnderGrant { #[cbor(n(0), with = "minicbor::bytes")] data: Vec<u8>,
                              #[cbor(n(1), with = "minicbor::bytes")] content: Vec<u8>,
                              #[cbor(n(2), with = "minicbor::bytes")] grant: Vec<u8>,
                              #[cbor(n(3), with = "minicbor::bytes")] proof: Vec<u8>,
                              #[n(4)] location: Option<String>, #[n(5)] time: u64 },
  ```
  on `ChipResponse`: `#[n(4)] Nonce { #[cbor(n(0), with = "minicbor::bytes")] nonce: [u8; 16] }`, `#[n(5)] Grant { #[cbor(n(0), with = "minicbor::bytes")] grant: Vec<u8>, #[n(1)] slot: u8 }`, `#[n(6)] GrantSignature { #[cbor(n(0), with = "minicbor::bytes")] signature: Vec<u8>, #[n(1)] remaining: u32 }`.
  `needs_consent`: `Nonce` false, `GrantRequest` true, `SshSignUnderGrant` **false** (the badge decides; a fallback to a prompt happens inside the badge, see Task 5). `length_ok`: `SshSignUnderGrant` may be `MAX_REQUEST_LEN` like `SshSign`.
  On `ChipApps`:
  ```rust
  fn nonce(&mut self, circle: &CircleId) -> Result<[u8; 16], ErrorCode>;
  fn grant_request(&mut self, circle: &CircleId, req: &GrantRequestFields) -> Result<(Vec<u8>, u8), ErrorCode>;
  fn ssh_sign_under_grant(&mut self, circle: &CircleId, req: &UnderGrantFields) -> Result<(Vec<u8>, u32), ErrorCode>;
  ```
  with `pub struct GrantRequestFields { action, author, location, agent, budget, exp, standing, cnf, time }` and `pub struct UnderGrantFields { data, content, grant, proof, location, time }` (plain structs in `wire.rs`, built by `serve` from the variants, so the trait does not take nine arguments).

- [ ] **Step 1: Write the failing tests**: round trips of each new variant through `Request::encode`/`decode` (as the existing tests do), `needs_consent` values, `length_ok` for `SshSignUnderGrant` at `MAX_REQUEST_LEN` true and `MAX_REQUEST_LEN + 1` false, and `serve` returning `NoChip` for the three on a bare `Pad`.
- [ ] **Step 2: Run** `cargo test -p cahoots-custodian` → FAIL.
- [ ] **Step 3: Implement** the variants, the field structs, the trait methods with `NoChip` defaults in the `Pad` impl, and the dispatch in `serve`: `Nonce → ChipResponse::Nonce`, `GrantRequest → ChipResponse::Grant`, `SshSignUnderGrant → ChipResponse::GrantSignature`.
- [ ] **Step 4: Run** `cargo test -p cahoots-custodian` and `cargo build --workspace` → PASS (the badge crate's `ChipApps` impls need the new methods: add `Err(ErrorCode::NoChip)` stubs there for now, replaced in Task 5).
- [ ] **Step 5: Commit** `custodian: Nonce, GrantRequest and SshSignUnderGrant on the chip request family`.

### Task 5: The badge issues, evaluates, counts and sweeps

**Files:**
- Create: `crates/badge/src/grants.rs`
- Modify: `crates/badge/src/board.rs` (trait hooks), `crates/badge/src/apps.rs` (dispatch), `crates/badge/src/screens.rs` (two screens), `crates/badge/src/badge.rs` (nonce per link, boot sweep, time), `crates/badge/src/lib.rs`
- Modify: `crates/badge/tests/common/mod.rs` (`FakeBoard` gets `grants: Option<Vec<u8>>`, `uptime: u64`)
- Test: `crates/badge/tests/grants.rs` (new)

**Interfaces:**
- Consumes: Tasks 1–4.
- Produces, on `Board`:
  ```rust
  fn grants_load(&mut self) -> Result<Option<GrantsRecord>, BoardError> { Ok(None) }
  fn grants_store(&mut self, r: &GrantsRecord) -> Result<(), BoardError> { let _ = r; Ok(()) }
  /// Seconds since boot. A board with no timer returns 0.
  fn uptime_secs(&self) -> u64 { 0 }
  ```
  in `grants.rs` (minicbor, `#[cbor(array)]`):
  ```rust
  pub const GRANTS_MAX: usize = 8;
  pub const BUDGET_MAX: u32 = 1000;
  pub const TIME_WRITE_STEP: u64 = 3600;
  pub struct GrantRecord { pub slot: u8, pub cti: [u8; 16], pub circle_id: CircleId, pub seat: u8, pub key_id: [u8; 32],
                           pub agent: String, pub action: String, pub author: String, pub locations: Vec<String>,
                           pub budget: u32, pub exp: Option<u64>, pub revoked: bool, pub used_soft: u64 }
  pub struct GrantsRecord { pub known_time: u64, pub grants: Vec<GrantRecord> }
  impl GrantsRecord { pub fn encode(&self) -> Vec<u8>; pub fn decode(&[u8]) -> Result<Self, Malformed>; pub fn next_free_slot(&self) -> Option<u8>; }
  pub struct Clock { pub stored: u64, pub told_at_uptime: u64 }   // known_time = max(stored, told + (uptime - told_at_uptime))
  pub fn issue(board: &mut B, circles: &[Circle], c: &Circle, req: &GrantRequestFields) -> Result<(Vec<u8>, u8), ErrorCode>;
  pub fn evaluate_request(board: &mut B, record: &GrantsRecord, c: &Circle, req: &UnderGrantFields, nonce: &[u8; 16]) -> Result<(SignGrant, GrantRecord, u32 /*used*/), Fallback>;
  pub enum Fallback { Prompt(ErrorCode /*logged reason*/), Refuse(ErrorCode) }
  pub fn sweep(board: &mut B) -> Vec<String>;   // at boot: delete counters without records and records without counters; drop expired records; log each
  ```
  Behaviour, fixed by the spec and these tests:
  - `issue`: refuse (`Denied`) before any screen when `action != "git-commit"`, `budget == 0 || budget > BUDGET_MAX`, `exp.is_none() && !standing`, `exp.is_some() && standing`, `cnf` not a valid P-256 point, or the record already holds `GRANTS_MAX` live grants. Otherwise (after the screen, see `apps.rs` below) raise `known_time` to `req.time` if later, pick `next_free_slot`, create the chip counter `KeyId::new(KeyKind::Grant, slot)` at 0 (or `used_soft: 0` with no chip), build and sign the `SignGrant` (cti = 16 random bytes from the board's entropy, `exp` as asked), push the record, store it (writing `known_time` too), return the signed bytes and the slot.
  - `evaluate_request`: `SignGrant::verify` under this seat's public key, circle and seat, else `Refuse(Unparseable)`; find the record by `cti` else `Refuse(Denied)` (log "grant unknown"); revoked → `Refuse(Denied)` ("grant revoked"); `proof_verify` over `Sha256(data)` and `nonce` else `Refuse(Unparseable)`; parse `data` as an `SSHSIG` with namespace `git` and `check_content` against `content`, read `describe_commit`'s author line (the full `Name <email>`; extend `ssh::describe_commit` to also return it if it returns only the name today) else `Prompt(Unparseable)`; `used` = `counter_get` (chip) or `used_soft`; `time` = the clock's known time after raising it with `req.time`; call `authz::evaluate`; on `Expired`/`Spent` delete the counter and the record, store, `Refuse(Denied)`; on `WrongAuthor`/`WrongLocation`/`WrongAction` → `Prompt(Denied)` (the ordinary ssh screen is drawn by the caller); on `WrongKey`/`WrongCircle` → `Refuse(Unparseable)`; on `Allowed` return the grant, the record and `used`.
  - In `apps.rs`: `prompt()` for `GrantRequest` runs `issue`'s pre-checks and returns `Some(screens::grant_request(agent, author, budget, exp_text, circle))` where `exp_text` is `"until <d MMM yyyy> (laptop)"` for an `exp` or `"until revoked"` for standing; for `SshSignUnderGrant` it runs `evaluate_request`: `Ok` → `Ok(None)` (answered now); `Prompt(_)` → the ordinary `ssh_screen` for `data`/`content` (so the holder is asked as for a plain `SshSign`, and the answer is a plain signature: implement by having the serve path treat a consented `SshSignUnderGrant` as `ssh_sign` with `remaining: 0` in the `GrantSignature` answer); `Refuse(code)` → `Err(code)`. `ChipApps::ssh_sign_under_grant` repeats the evaluation (as `ssh_sign` repeats its checks), increments the counter (`counter_next`, or `used_soft += 1` and store), signs, and returns `(ssh::encode_signature(sig), remaining)`; it also calls `board.note_grant_use(agent, subject, remaining, budget)` — a new `Board` default method the badge uses to draw `screens::grant_signed` for one refresh and log `grant <cti[..4] hex>: signed git commit "<subject>", <remaining> left`.
  - Nonce: `Menu` gets `nonce: [u8; 16]` from the board's entropy at `enter_menu`; `ChipRequest::Nonce` answers it; after each use under a grant it is replaced. `BadgeApps::new` takes `nonce: &[u8; 16]` (add a parameter; update its two call sites).
  - Time: `Badge` keeps a `Clock`; `known_time()` = `max(stored, told + (uptime_secs − told_at))`; `raise(time, uptime)`; the record's `known_time` is written when `issue` or a use runs and `known_time() − stored > TIME_WRITE_STEP`.
  - Screens: `screens::grant_request(agent: &str, author: &str, budget: u32, until: &str, circle: &str) -> Screen` with the lines of the spec's example (title `let an agent sign`, agent marked `(laptop)`, `git commits as`, author, `N of them`, until, footer refuse/allow); `screens::grant_signed(agent: &str, subject: &str, remaining: u32, budget: u32) -> Screen` (title `signed for <agent>`, the subject, `<remaining> left of <budget>`, no footer).
  - `sweep` runs from `boot()` after the chip record reconcile.

- [ ] **Step 1: Write the failing tests** in `crates/badge/tests/grants.rs`, with the serving-and-connected badge helper the ssh tests use (copy `serving(chip: bool)` from `tests/ssh.rs` into this file, or move it to `common`): 
  `a_grant_is_issued_after_both_and_the_counter_starts_at_zero` (send `GrantRequest`, the screen shows `let an agent sign`, `claude on connerswann-mbp (laptop)`, `Conner Swann <me@connerswann.me>`, `50 of them`, `until 29 Nov 2026 (laptop)`; press Both; the answer is `Grant { slot: 0, .. }` whose bytes `SignGrant::verify` under the seat's public key; the chip has `KeyId::new(KeyKind::Grant, 0)` at 0); `left_refuses_a_grant` (Denied, no counter, no record); `a_use_signs_without_a_prompt_and_counts` (after issuing: `Nonce`, then `SshSignUnderGrant` with a `git` SSHSIG over a commit text with that author and a proof by the agent key; the answer is `GrantSignature { remaining: 49 }` on the first call of a `ask_now`-style helper that asserts no prompt was drawn; the chip counter is 1; the last screen is `signed for claude on connerswann-mbp` / the subject / `49 left of 50`); `a_replayed_proof_is_refused` (the same request again with the old nonce → `Unparseable`, counter unchanged); `committer_may_differ_from_author`; `another_author_falls_back_to_the_prompt` (the ssh git screen is drawn; Both signs; the answer is `GrantSignature { remaining: 0 }` and the counter is unchanged); `another_key_is_refused_without_a_prompt` (`Unparseable`); `a_spent_grant_is_refused_and_removed` (budget 1: one use, then `Denied`, counter and record gone); `an_expired_grant_is_refused_and_removed` (`exp` 100, request `time` 200); `an_earlier_laptop_time_does_not_lower_known_time` (issue with time 1000, use with time 500 and `exp` 800 → still refused as expired); `a_standing_grant_never_expires` (`standing: true`, screen says `until revoked`, a use with `time: u64::MAX / 2` signs); `standing_with_an_expiry_is_refused` (`Denied` before any screen); `a_ninth_grant_is_denied`; `a_restored_record_does_not_refund_the_chip_counter` (issue budget 2, use once, copy `board.grants` aside, use again, restore the copy, use → `Denied`, the chip counter reads 2); `a_badge_without_a_chip_counts_in_its_record` (`serving(false)`: two uses, `used_soft == 2` in the stored record); `sweep_deletes_orphans` (build a board with a counter at slot 5 and no record, and a record at slot 6 with no counter; `boot()`; both gone; the log names them).
- [ ] **Step 2: Run** `cargo test -p cahoots-badge --test grants` → FAIL.
- [ ] **Step 3: Implement** per the interfaces, in this order: `GrantsRecord` + `Board` hooks + `FakeBoard` fields; `Clock`; `issue`; the two screens; `apps.rs` dispatch and the three `ChipApps` methods; `evaluate_request`; the nonce in `Menu`; `sweep` in `boot()`; `note_grant_use` drawing. Replace the Task 4 stubs.
- [ ] **Step 4: Run** `cargo test -p cahoots-badge` → PASS, including the older suites (the `ssh.rs` and `menu.rs` tests must not change behaviour).
- [ ] **Step 5: Commit** `badge: issues a sign grant on its screen, honours it per use with a chip counter and a nonce, and sweeps what is spent or expired`.

### Task 6: The grants page and revocation

**Files:**
- Modify: `crates/badge/src/badge.rs` (`Page::Grants` between `Key` and `Circles`; the page's body; the revoke prompt), `crates/badge/src/screens.rs` (`grants_page`, `revoke_prompt`)
- Modify: `crates/badge/tests/menu.rs` (the ring is five pages again: Words 1, Key 2, Grants 3, Circles 4, Forget 5; update `on_page` to `/5` and every page number), `crates/badge/tests/circles.rs` (`to_the_list` presses Right three times; `3/4` → `4/5`, `4/4` → `5/5`)
- Test: `crates/badge/tests/grants.rs` (add `revoking_from_the_page_ends_the_grant`)

**Interfaces:**
- Produces: `screens::grants_page(circle: &str, tier: &str, entries: &[(String /*agent*/, String /*"37 of 50 · until 29 Nov"*/)], chosen: usize) -> Screen` (title `grants`, body one line per live grant with `>` on the chosen, `no grants` when empty, footer `prev  A+B  revoke  next` or `page  A+B  choose  page` when empty); `screens::revoke_prompt(agent: &str) -> Screen` (title `revoke this grant?`, the agent, `it stops at once`, footer `back  A+B  revoke  back`).

- [ ] **Step 1: Write the failing test** `revoking_from_the_page_ends_the_grant`: issue a grant; Right twice to `3/5` `grants`; the page shows the agent and `50 of 50`; Both → the revoke prompt; Both → the log has `grant … revoked`, the counter is gone, and a use under the grant answers `Denied`; Left on the prompt goes back without revoking. Update the page-number assertions in `menu.rs` and `circles.rs`.
- [ ] **Step 2: Run** `cargo test -p cahoots-badge` → FAIL.
- [ ] **Step 3: Implement**: the page in the ring, `Menu` gains `grant_index: usize` and `confirm_revoke: bool`; Left/Right on the page move the caret when there are entries (the ring turns only from the first/last entry, as the chooser does); Both opens the prompt; Both again revokes (delete the counter, mark `revoked`, store, log `grant <cti[..4]>: revoked`), Left/Right returns.
- [ ] **Step 4: Run** `cargo test -p cahoots-badge` → PASS.
- [ ] **Step 5: Commit** `badge: a grants page lists live grants and revokes one with both buttons`.

### Task 7: Storage and time on the real boards

**Files:**
- Modify: `crates/rp2350/src/store.rs` (`GRANTS_BASE: u32 = 0x3F_5000`, one sector, magic `b"CAHG"`, `load_grants`/`save_grants` as the chip record's), `crates/rp2350/src/board.rs` (`grants_load`, `grants_store`, `uptime_secs` from the timer)
- Modify: `crates/host/src/board.rs` (`grants.cbor` beside `chip.cbor`; `uptime_secs` from an `Instant` taken at construction)
- Modify: `crates/esp32s3/src/...` only if it fails to build (its board keeps the trait defaults)
- Test: build checks

- [ ] **Step 1: Implement** the three boards' hooks per the interfaces, with the rp2350 asserts `CHIP_RECORD_BASE + SECTOR <= GRANTS_BASE` and `GRANTS_BASE + SECTOR <= 0x40_0000`.
- [ ] **Step 2: Run** `cd crates/rp2350 && cargo build --release && cargo build --release --features bench`, `cargo build -p cahoots-host`, `cargo test -p cahoots-host --test badge_serve` → builds and passes.
- [ ] **Step 3: Commit** `boards: a grants record sector on the Pico 2, grants.cbor on the simulator, and the uptime each board knows`.

### Task 8: The agent side

**Files:**
- Create: `crates/host/src/agentkey.rs` (the agent's P-256 key in a file), `crates/host/src/grants.rs` (request, list, show, verify, storage)
- Modify: `crates/host/src/agent.rs` (send under a grant), `crates/host/src/main.rs` (the `grant` subcommands), `crates/host/src/lib.rs`, `crates/host/README.md` (a "Grants for an agent" section after "Signing a file")
- Test: `crates/host/tests/grant_agent.rs` (new)

**Interfaces:**
- Consumes: Tasks 2 and 4; `BadgeClient::request`.
- Produces:
  ```rust
  // agentkey.rs
  pub fn dir() -> PathBuf;                       // $CAHOOTS_HOME or ~/.config/cahoots
  pub fn load_or_create(dir: &Path) -> Result<SeatKey, String>;   // dir/agent.key: 32 secret bytes hex, mode 0600; created with OsRng
  // grants.rs
  pub struct Stored { pub circle: String, pub cti: [u8; 16], pub bytes: Vec<u8>, pub remaining: Option<u32>, pub agent: String }
  pub fn request(badge: &mut BadgeClient, circle: &str, agent: &str, author: &str, location: Option<&str>, budget: u32, until: Option<u64>, standing: bool, key: &SeatKey) -> Result<Stored, String>;
  pub fn save(dir: &Path, s: &Stored) -> Result<PathBuf, String>;  // dir/grants/<circle>-<cti[..4] hex>.cwt, and .json beside it with remaining and agent
  pub fn load_all(dir: &Path) -> Vec<Stored>;
  pub fn find(dir: &Path, circle: &str) -> Option<Stored>;       // the newest with remaining != Some(0)
  pub fn show(bytes: &[u8]) -> Result<serde_json::Value, String>; // the claims as JSON (iss, sub, aud, exp, cti, details)
  pub fn verify(bytes: &[u8], manifest: &Manifest, seat: u8) -> Result<SignGrant, String>;
  ```
  CLI: `cahoots-host grant request (--serial|--sim) --circle <name> --agent <text> --author <text> [--location <repo>] --budget <n> (--until <YYYY-MM-DD> | --standing)`; `grant list`; `grant show <file>`; `grant verify <file> --state <dir> --circle <name>`. `--until` is midnight UTC of that date as seconds; `time` sent is `SystemTime::now()`.
  Agent: on start, `load_all` for each `--circle`; in `extension` for `sign-with-content@cahoots`, when the key's circle has a stored grant, the SSHSIG namespace is `git`, and the extension carries a `location` (a fourth string the helper now sends: `git config --get remote.origin.url` or empty), send `Nonce` then `SshSignUnderGrant`; on `GrantSignature { remaining }` update the stored `.json` and log `agent: <comment> signed under a grant, <remaining> left` (or `signed after a prompt` when `remaining == 0` and the badge drew the prompt); on `Denied`/`Unparseable` log the badge's reason and, if the grant is spent/expired/revoked (the badge's `Denied`), mark the stored grant `remaining: Some(0)` and retry once as a plain `SshSign`.
  `gitsign.rs`: `sign_one` gains a `location: &str` parameter and the helper reads `remote.origin.url` from the repository it runs in (`git config --get remote.origin.url`, empty on failure) and sends it as the fourth string of the extension; `agent::sign_with_content` gains the parameter.

- [ ] **Step 1: Write the failing test** `crates/host/tests/grant_agent.rs`: with the simulated badge and `serving.start()`: `grant request --sim … --circle bench --agent "claude" --author "Ada Lovelace <ada@example.com>" --budget 2 --until 2030-01-01` while a presser thread answers the `let an agent sign` screen with `B`; the file exists and `grant show` prints `"budget": 2`; then two `cahoots-host-git-sign -Y sign -n git` runs over `COMMIT` (the text from `gitsign.rs`'s test, author Ada) with no press, each producing a `.sig` that verifies, the badge's screen showing `signed for claude` and `1 left of 2` then `0 left of 2`; a third run is answered with the ordinary `sign a git commit` prompt (press `B`) and still verifies; `grant list` shows `0 left`.
- [ ] **Step 2: Run** `cargo test -p cahoots-host --test grant_agent` → FAIL.
- [ ] **Step 3: Implement** per the interfaces; `CAHOOTS_HOME` is set by the test to a temp dir.
- [ ] **Step 4: Run** `cargo test -p cahoots-host` → PASS (the `gitsign`, `agent` and `sign` suites unchanged).
- [ ] **Step 5: Commit** `host: an agent key, grant request/list/show/verify, and git signatures under a grant through the agent`.

### Task 9: Vectors, the protocol document and the roadmap

**Files:**
- Modify: `crates/vectors/src/main.rs` (a `sign_grant` entry: the claims, the signed bytes, a proof over a fixed hash and nonce), `crates/vectors/objects.json` (regenerate the way the crate's README says), the vectors' reproduction tests (`crates/objects/tests/vectors.rs` or wherever `objects.json` is checked: `grep -rln objects.json crates/*/tests`)
- Modify: `docs/protocol.md` section 8.3 (the sign grant: `aud`, `exp`, the `cahoots:sign` detail, the two kinds, the proof under `cahoots/proof/v1`) and section 6 (the three requests and the three answers)
- Modify: `docs/plan.md` (M4h row: status and what landed; the first-product row 2 mention), `README.md` (one line under the chip list: a grant an agent can use on a budget)

- [ ] **Step 1: Add the vector and its reproduction test**; run `cargo test -p cahoots-vectors -p cahoots-objects` → PASS.
- [ ] **Step 2: Write the protocol and roadmap text** in the repository's voice (plain words, one claim per sentence, no em dashes).
- [ ] **Step 3: Run** `node web/site-check.mjs` (the docs render) → all passed.
- [ ] **Step 4: Commit** `grants an agent can use: vectors, the protocol's sign grant and proof, the roadmap`.

### Task 10: The bench

**Files:**
- Modify: `scripts/bench.sh` (after the git commit step: `grant request` with budget 2 and `--until` a month out, a presser from the bench driver, two commits with no press, the chip's counter read back through a new `cahoots-host grant count --serial … --circle <name> <file>` that asks `Nonce` and compares, or by the log line `… 0 left of 2`)
- Modify: `crates/host/src/link.rs` only if the bench driver needs a new screen rule (the consent rule already presses `A+B  allow` after `drawn`; `signed for` has no footer and must not be pressed)

- [ ] **Step 1: Add the steps** and run `scripts/bench.sh` on the badge → the log shows `signed for`, `1 left of 2`, `0 left of 2`, and both commits verify.
- [ ] **Step 2: Commit** `bench: an agent's grant issued on the real badge, two commits signed under it with no press`.

---

## Self-review notes

- Spec coverage: §1 grant → Tasks 1, 2, 9; §2 issuing → Tasks 4, 5; §3 evaluator and nonce → Tasks 2, 5; §4 counter → Tasks 3, 5; §5 records, time, revocation, standing → Tasks 5, 6, 7; §6 limits → Task 5; §7 agent side → Task 8; §8 tests → Tasks 5, 8, 10; constraints (protocol, vectors) → Task 9.
- `describe_commit` today returns the author's name and the subject; Task 5 extends it to return the full author line, which the evaluator compares exactly. Task 8's test uses the `COMMIT` text whose author is `Ada Lovelace <ada@example.com>`.
- The fallback path answers `GrantSignature { remaining: 0 }` after a prompt so the agent's one code path handles both; the agent prints which happened.
