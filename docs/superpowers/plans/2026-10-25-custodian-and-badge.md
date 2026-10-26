# Custodian contract and badge crate: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One contract and one wire protocol for anything that holds a pad, and one crate holding the badge's behaviour so every board runs the same code.

**Architecture:** A new `no_std` crate `cahoots-custodian` takes the provider's `PadProvider` trait (renamed `Custodian`), adds `circles()`, and adds a CBOR request/response protocol with a `serve` function and a `Consent` hook. A new `no_std` crate `cahoots-badge` holds the state machine now in `crates/esp32s3/src/main.rs`, generic over a `Board` trait. The ESP32-S3 crate and the host simulator each become a `Board`.

**Tech Stack:** Rust 1.94 (workspace), `minicbor` with `alloc` and `derive` for CBOR, existing `cahoots-core`, `cahoots-hal`, `zeroize`.

**Spec:** `docs/superpowers/specs/2026-10-25-custodian-architecture.md`

## Global Constraints

- Both new crates are `#![no_std]` with `extern crate alloc` and `#![forbid(unsafe_code)]`, like `cahoots-hal`.
- Both are workspace members and build for `thumbv7em-none-eabi` with `--no-default-features` (CI's no_std check).
- `cargo fmt --all --check`, `cargo clippy --workspace --all-features --all-targets -- -D warnings` and `cargo test --workspace --all-features` pass after every task.
- Public names exactly as the spec gives them: `Custodian`, `CircleInfo`, `Request`, `Response`, `ErrorCode`, `Consent`, `serve`, `KIND_REQUEST = 40`, `KIND_RESPONSE = 41`, `Board`, `Badge`, `Circle`, `BoardError`.
- `cahoots-provider`'s public API does not change for its users: `PadProvider`, `PskHandle`, `PadError`, `PSK_LEN`, `LOW_PAD_PERCENT`, `MemProvider`, `FileProvider` remain importable from it.
- No em dashes in docs or doc comments. Prose in the plain voice of `.claude/skills/writing-blog-posts/SKILL.md`.
- The `crates/esp32s3` crate is outside the workspace and needs the `esp` toolchain; its task is verified by `cargo build --release` in that directory with `source ~/.cargo/esp-env.sh`, and by flashing if a Nano is on the bench.

## Review Focus

1. A request names a circle the custodian does not hold: the response is `Error { code: UnknownCircle }` and nothing is spent. Test in Task 3.
2. Request bytes are not valid CBOR, or valid CBOR of the wrong shape: the response is `Error { code: Malformed }`, never a panic. Test in Task 3.
3. Consent is refused: the response is `Error { code: Denied }`, `remaining` is unchanged, and the ledger has no new record. Test in Task 3.
4. A request frame arrives on the down link while the badge is on the menu's words screen or any menu item other than Serve: it is answered `Busy`. A request that arrives during the pile or the ceremony is ignored, because the ring owns the links then, and the laptop retries. The ceremony is unaffected. Test in Task 6.
5. The menu's "key as code" spends exactly 32 bytes once per B press and shows them with `show_qr`; a second B press spends a different range. Test in Task 6.

---

### Task 1: `cahoots-custodian` crate with the contract moved from the provider

**Files:**
- Create: `crates/custodian/Cargo.toml`, `crates/custodian/src/lib.rs`, `crates/custodian/src/contract.rs`
- Modify: `Cargo.toml` (workspace `members` and `[workspace.dependencies]`), `crates/provider/Cargo.toml`, `crates/provider/src/lib.rs:12-25,53-78,80-160` (the moved items become re-exports; the `impl ... for Pad<S, L>` moves), `crates/provider/src/mem.rs`, `crates/provider/src/file.rs` (add `circles()`)
- Test: `crates/custodian/tests/contract.rs`, existing `crates/provider/tests/*`

**Interfaces:**
- Produces: `cahoots_custodian::{Custodian, CircleInfo, PskHandle, PadError, PSK_LEN, LOW_PAD_PERCENT}`; `impl<S: PadStorage, L: SpentLedger> Custodian for cahoots_core::pad::Pad<S, L>` with `circles()` returning one `CircleInfo { tier: 0 }`; `cahoots_provider::PadProvider` is the same trait.

- [ ] **Step 1: Write the failing test** in `crates/custodian/tests/contract.rs`

```rust
use cahoots_custodian::{Custodian, PadError, PSK_LEN};
// Build a core Pad over MemStorage/MemLedger with the 3-member "vectors" params from
// crates/vectors (n = 3, pad_len = 192, name "vectors") and 192 random bytes, seat 0.
#[test]
fn pad_is_a_custodian() {
    let mut pad = /* as above */;
    let infos = pad.circles();
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].seat, 0);
    assert_eq!(infos[0].n, 3);
    assert_eq!(infos[0].tier, 0);
    let before = pad.remaining(&infos[0].circle_id);
    let (h, key) = pad.psk(&infos[0].circle_id, b"test").unwrap();
    assert_eq!(h.slice, 0);
    assert_eq!(key.len(), PSK_LEN as usize);
    assert_eq!(pad.remaining(&infos[0].circle_id), before - u64::from(PSK_LEN));
    assert_eq!(pad.psk_for(&h, b"test"), Err(PadError::AlreadySpent));
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p cahoots-custodian --test contract`
Expected: FAIL, the crate does not exist.

- [ ] **Step 3: Create the crate and move the contract**

`crates/custodian/Cargo.toml`: package `cahoots-custodian`, workspace version/edition/license, deps `cahoots-core` (workspace), `zeroize` (workspace, features `alloc`), `minicbor = { version = "0.25", default-features = false, features = ["alloc", "derive"] }` (use the current 0.2x release if 0.25 is not the latest). Feature `std = ["cahoots-core/std"]`.

`src/contract.rs`: `PskHandle`, `PadError`, `PSK_LEN`, `LOW_PAD_PERCENT`, the `Custodian` trait exactly as the spec, `CircleInfo` with `#[derive(Clone, Debug, PartialEq, Eq)]`, and the blanket `impl Custodian for Pad<S, L>` moved from `crates/provider/src/lib.rs` (its `to_psk` helper moves too; `PadError::Storage(String)` keeps the `String` via `alloc`). `circles()` for `Pad` returns one entry: `circle_id()`, `my_slice()` as seat, `params().n`, `params().name`, tier 0.

`src/lib.rs`: `#![no_std]`, `#![forbid(unsafe_code)]`, `extern crate alloc`, `pub mod contract; pub use contract::*;`, crate docs stating the invariant from the spec.

Workspace: add `"crates/custodian"` to `members` and `cahoots-custodian = { path = "crates/custodian", default-features = false }` to `[workspace.dependencies]`.

Provider: depend on `cahoots-custodian` with feature `std`; replace the moved definitions with `pub use cahoots_custodian::{Custodian as PadProvider, CircleInfo, PskHandle, PadError, PSK_LEN, LOW_PAD_PERCENT};`. Implement `fn circles(&self) -> Vec<CircleInfo>` on `MemProvider` and `FileProvider` from the circles they hold, tier 0.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p cahoots-custodian -p cahoots-provider -p cahoots-wireguard -p cahoots-hpke -p age-plugin-cahoots -p cahoots-chat`
Expected: PASS, including every existing provider and adapter test unchanged.

- [ ] **Step 5: no_std check**

Run: `cargo build -p cahoots-custodian --no-default-features --target thumbv7em-none-eabi`
Expected: success.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/custodian crates/provider
git commit -m "custodian: the pad contract, moved from the provider and named"
```

---

### Task 2: Wire messages in CBOR

**Files:**
- Create: `crates/custodian/src/wire.rs`
- Modify: `crates/custodian/src/lib.rs` (`pub mod wire;`)
- Test: `crates/custodian/tests/wire.rs`

**Interfaces:**
- Produces: `wire::{Request, Response, ErrorCode}` with `#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]`, array-encoded (`#[cbor(array)]`), variant indices in spec table order starting at 0; `Request::encode(&self) -> Vec<u8>`, `Request::decode(&[u8]) -> Result<Request, ErrorCode>` (returns `ErrorCode::Malformed`), the same two on `Response`; `pub const KIND_REQUEST: u8 = 40; pub const KIND_RESPONSE: u8 = 41;`.
- `CircleId` is `[u8; 16]` and `PskHandle` encodes as its 21 `to_bytes()` form. Keys and bytes are CBOR byte strings.

- [ ] **Step 1: Write the failing tests** in `crates/custodian/tests/wire.rs`

```rust
#[test]
fn every_request_round_trips() { /* one of each Request variant: encode then decode equals the original */ }
#[test]
fn every_response_round_trips() { /* one of each Response variant, including Error for every ErrorCode */ }
#[test]
fn garbage_is_malformed() {
    assert_eq!(Request::decode(&[0xFF, 0x00]), Err(ErrorCode::Malformed));
    assert_eq!(Request::decode(&[]), Err(ErrorCode::Malformed));
}
#[test]
fn identify_is_small() { assert!(Request::Identify.encode().len() <= 4); }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p cahoots-custodian --test wire`
Expected: FAIL, `wire` does not exist.

- [ ] **Step 3: Implement `wire.rs`**

The enums as the spec's table, derived with minicbor. `PskHandle` gets `impl<C> Encode<C>` / `Decode` as a 21-byte string in `contract.rs`, or a `#[cbor(with = ...)]` helper; either is fine as long as the round trip holds. Also `impl From<PadError> for ErrorCode` mapping each `PadError` variant to its code (`Storage(_)` to `Storage`).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p cahoots-custodian --test wire`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/custodian
git commit -m "custodian: the request and response messages in CBOR"
```

---

### Task 3: `serve` with consent

**Files:**
- Create: `crates/custodian/src/serve.rs`
- Modify: `crates/custodian/src/lib.rs`
- Test: `crates/custodian/tests/serve.rs`

**Interfaces:**
- Consumes: Task 1's `Custodian`, Task 2's messages.
- Produces: `pub trait Consent { fn allow(&mut self, req: &Request) -> bool; }`, `pub struct AlwaysAllow;` implementing it, and `pub fn serve<C: Custodian, K: Consent>(c: &mut C, consent: &mut K, request: &[u8]) -> Vec<u8>` returning an encoded `Response`. `Identify` and `Remaining` never ask for consent; the four spending requests do.

- [ ] **Step 1: Write the failing tests** in `crates/custodian/tests/serve.rs`, using the same `Pad` fixture as Task 1

```rust
#[test]
fn identify_lists_the_circle() { /* serve(Identify) decodes to Identity with one CircleInfo, tier 0 */ }
#[test]
fn psk_spends_and_returns_a_handle() { /* Psk -> Response::Psk; remaining dropped by 32; a second Psk returns a different handle.offset */ }
#[test]
fn unknown_circle_is_an_error_and_spends_nothing() { /* Psk with circle_id [9u8; 16] -> Error UnknownCircle; remaining unchanged */ }
#[test]
fn malformed_bytes_are_an_error() { assert_eq!(decode(serve(&mut pad, &mut AlwaysAllow, &[0xFF])), Response::Error { code: ErrorCode::Malformed }); }
#[test]
fn denied_consent_spends_nothing() {
    struct Never; impl Consent for Never { fn allow(&mut self, _: &Request) -> bool { false } }
    /* Spend{len: 16} -> Error Denied; remaining unchanged */
}
#[test]
fn spend_past_the_slice_is_exhausted() { /* Spend{len: slice_len + 1} -> Error Exhausted; remaining unchanged */ }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p cahoots-custodian --test serve`
Expected: FAIL.

- [ ] **Step 3: Implement `serve.rs`**

Decode; on error return `Error { Malformed }`. For spending requests call `consent.allow(&req)` first and return `Error { Denied }` if false. Map `PadError` through `ErrorCode::from`. Encode the response.

- [ ] **Step 4: Run the tests, then the whole workspace**

Run: `cargo test -p cahoots-custodian && cargo clippy --workspace --all-features --all-targets -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/custodian
git commit -m "custodian: serve a request with consent"
```

---

### Task 4: `cahoots-badge` crate with the `Board` trait and `boot`

**Files:**
- Create: `crates/badge/Cargo.toml`, `crates/badge/src/lib.rs`, `crates/badge/src/board.rs`, `crates/badge/src/badge.rs`
- Create: `crates/badge/tests/common/mod.rs` (a `FakeBoard`), `crates/badge/tests/boot.rs`
- Modify: `Cargo.toml` (members, workspace dependency `cahoots-badge`)

**Interfaces:**
- Consumes: `cahoots_hal::{Entropy, PadStore, Display, Link, ledger::RecordStore, health, pile, ring, store}`, `cahoots_custodian`.
- Produces: `board::{Board, BoardError, Circle}` exactly as the spec; `badge::Badge<B: Board>` with `pub fn new(board: B) -> Self`, `pub fn boot(&mut self)`, `pub fn board(&mut self) -> &mut B`, `pub fn circle(&self) -> Option<&Circle>`. `BoardError` is `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum BoardError { Storage, Link }`.
- `FakeBoard` (test-only): in-memory `Vec<u8>` pad and record region, a `Vec<String>` of status lines and a `Vec<[String; 6]>` of words shown, a `Vec<Vec<u8>>` of QR payloads, scripted button presses as a `VecDeque<Button>`, `OsEntropy`-style randomness from a seeded xorshift, two `cahoots_host`-free in-memory links (a pair of `VecDeque<u8>` queues; `peer_present` is a flag).

- [ ] **Step 1: Write the failing test** in `crates/badge/tests/boot.rs`

```rust
#[test]
fn boot_without_a_circle_shows_the_selftest_and_waits() {
    let mut badge = Badge::new(FakeBoard::new());
    badge.boot();
    let b = badge.board();
    assert!(b.status.iter().any(|s| s.contains("circle: none yet")));
    assert!(b.words.is_empty());
}
#[test]
fn boot_with_a_circle_shows_its_words() {
    // FakeBoard pre-loaded with a 3-member circle record and a 192-byte pad
    badge.boot();
    assert_eq!(badge.board().words.len(), 1);
    assert_eq!(badge.board().words[0], cahoots_core::checkwords::check_words(&params, &pad).map(String::from));
}
#[test]
fn a_held_at_boot_forgets_the_circle() {
    // FakeBoard pre-loaded, a_down = true during boot
    badge.boot();
    assert!(badge.circle().is_none());
    assert!(badge.board().load_circle().is_none());
    assert!(badge.board().pad.iter().all(|&b| b == 0));
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p cahoots-badge --test boot`
Expected: FAIL.

- [ ] **Step 3: Implement the crate**

`board.rs`: the trait and structs from the spec, verbatim. `badge.rs`: `Badge { board: B, circle: Option<Circle> }`; `boot` is the self-test block of `crates/esp32s3/src/main.rs:85-150` made generic: common lines (`trng: health tests pass` over a 4 KiB fill through `entropy()`, `circle: ...`) plus `board.selftest()` lines, shown with `show_status` joined by newlines, `delay_ms(2500)`, the A-held wipe, then words from `open_pad` for a stored circle. The common status strings are the ones in that file.

- [ ] **Step 4: Run the tests and the no_std build**

Run: `cargo test -p cahoots-badge && cargo build -p cahoots-badge --no-default-features --target thumbv7em-none-eabi`
Expected: PASS and success.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/badge
git commit -m "badge: the Board trait and boot, with a fake board for tests"
```

---

### Task 5: `run_once`: pile, ceremony, keep or discard

**Files:**
- Modify: `crates/badge/src/badge.rs`
- Test: `crates/badge/tests/ceremony.rs`

**Interfaces:**
- Produces: `pub fn run_once(&mut self) -> Result<(), String>` with the flow of `crates/esp32s3/src/main.rs:163-266`: status `"plug into a pile. B on top starts"`, `Pile::new(down, up, name, pad_len, false)` from `board.proposal()`, seat status `"seat {s}. B when the pile is whole"`, B calls `pile.start()`, `into_ceremony`, `ring.run(display, || delay_ms(1))`, then `"B keep this circle . A discard"`; keep writes the pad with `write_pad`, fresh `records(true)`, `save_circle`, status `"saved. read the words aloud"`; discard drops the result and shows `"discarded"`.

- [ ] **Step 1: Write the failing test** in `crates/badge/tests/ceremony.rs`

Two `FakeBoard`s joined by their in-memory links (board 1's up is board 2's down, as a two-card pile), each inside a `Badge`, stepped alternately on one thread. Script: card 2 (top) gets a B press after both report a seat, both get B at the keep prompt.

```rust
#[test]
fn two_fake_cards_complete_a_ceremony_and_keep_it() {
    // step both badges' run_once in lockstep until both return Ok
    assert_eq!(b1.circle().unwrap().circle_id, b2.circle().unwrap().circle_id);
    assert_eq!(b1.board().words.last(), b2.board().words.last());
    assert_eq!(b1.circle().unwrap().seat, 0);
    assert_eq!(b2.circle().unwrap().seat, 1);
}
#[test]
fn a_at_the_prompt_discards() { /* same, but card 1 presses A: b1.circle() is None and b1.board().load_circle() is None */ }
```

Structure it as a step machine so two badges can be driven on one thread: `pub fn step(&mut self) -> Step` with `#[derive(Debug, Clone, PartialEq, Eq)] pub enum Step { Idle, InPile, InCeremony, Prompt, Kept, Discarded, Failed(String) }`, holding the `Pile` or `RingCeremony` in a private state enum between calls, and `pub fn run_once(&mut self) -> Result<(), String>` as a loop over `step` that calls `board.delay_ms(1)` between steps. The ring's `run` helper blocks, so drive it with `start` and `poll` from `cahoots_hal::ring::RingCeremony` instead.

- [ ] **Step 2: Run to see it fail**, then **Step 3: implement**, then **Step 4: run the tests**

Run: `cargo test -p cahoots-badge`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/badge
git commit -m "badge: pile, ceremony, keep or discard, generic over the board"
```

---

### Task 6: The idle menu: words, key as code, serve

**Files:**
- Modify: `crates/badge/src/badge.rs`
- Test: `crates/badge/tests/menu.rs`

**Interfaces:**
- Produces: `pub fn idle_step(&mut self)`, called in a loop once `circle()` is `Some`. Menu items in order `Words`, `KeyAsCode`, `Serve`; A moves to the next item and shows its name on the status line (`"words"`, `"key as code . B"`, `"serve . B"`); B activates. `Words` shows the six words. `KeyAsCode` does `custodian.spend(circle, 32)` and `show_qr(&bytes)` then shows `"shown. A for menu"`. `Serve` reads frames from the down link: a frame of kind `KIND_REQUEST` is decoded, shown as one status line (`"spend 32 for {circle name}? B yes . A no"` for spending requests), answered through `serve` with a `Consent` that returns true on B and false on A, and the response goes back as a `KIND_RESPONSE` frame. `Identify` and `Remaining` are answered without a prompt. Any `KIND_REQUEST` frame that arrives while not in `Serve` is answered `Error { Busy }`.
- The custodian is `cahoots_core::pad::Pad::with_circle_id(params, seat, circle_id, StorePad::new(board.open_pad(len)), PersistentLedger::open(board.records(false)?, params.slices())?)`, built once when the menu is entered and kept for the idle loop's life.

- [ ] **Step 1: Write the failing tests** in `crates/badge/tests/menu.rs`

```rust
#[test]
fn key_as_code_spends_32_and_shows_a_qr() {
    // boot with a stored circle; press A once (to "key as code"), then B
    assert_eq!(badge.board().qr.len(), 1);
    assert_eq!(badge.board().qr[0].len(), 32);
    // B again: a second, different 32 bytes
    assert_eq!(badge.board().qr.len(), 2);
    assert_ne!(badge.board().qr[0], badge.board().qr[1]);
}
#[test]
fn serve_answers_identify_without_a_prompt() { /* enter Serve; push a KIND_REQUEST frame with Request::Identify on the down link; one KIND_RESPONSE frame comes back decoding to Identity with the circle */ }
#[test]
fn serve_psk_waits_for_b() { /* push Psk; no response until B is pressed; then Response::Psk; remaining dropped by 32 */ }
#[test]
fn serve_psk_refused_with_a() { /* push Psk; press A; Response::Error Denied; remaining unchanged */ }
#[test]
fn a_request_outside_serve_is_busy() { /* on the words screen, push Identify; the response is Error Busy */ }
```

- [ ] **Step 2: Run to see them fail**, **Step 3: implement**, **Step 4: run**

Run: `cargo test -p cahoots-badge && cargo clippy --workspace --all-features --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/badge
git commit -m "badge: the idle menu; a key as a code, and serving a laptop with consent"
```

---

### Task 7: The host simulator as a board

**Files:**
- Create: `crates/host/src/board.rs` (`HostBoard`)
- Modify: `crates/host/src/lib.rs` (`pub mod board;`), `crates/host/src/main.rs` (a `badge` command), `crates/host/Cargo.toml` (depend on `cahoots-badge`, `cahoots-custodian`), `crates/host/README.md`
- Test: `crates/host/tests/badge.rs`

**Interfaces:**
- Consumes: `cahoots_badge::{Board, Badge, Circle}`; the host's `FilePadStore`, `FileRecordStore`, `PngDisplay`, `SocketLink`.
- Produces: `HostBoard::new(state_dir: &Path, listen: &str, next: &str, name: &str, pad_len: u32, seed: Option<u64>) -> HostBoard`; buttons from a file `buttons.txt` in the state dir polled each `delay_ms` (each line `A` or `B` is one press, consumed), so a test or a script can press them; `cahoots-host badge --state DIR --listen ADDR --next ADDR --name NAME --pad-len N [--seed S]` runs `boot` then `run_once` then the idle loop; circle record as `circle.txt` using the existing `Device` format.

- [ ] **Step 1: Write the failing test** in `crates/host/tests/badge.rs`

```rust
#[test]
fn two_host_boards_complete_a_ceremony_through_the_badge_crate() {
    // two HostBoards in two threads over a socket pair, as the existing simulator test does;
    // write "B" into each buttons.txt at the right moments (poll the display.png or the status
    // via a status.txt the HostBoard also writes); assert both circle.txt agree on circle_id
    // and that Device::open on each state dir gives the same check words.
}
```

- [ ] **Step 2: Run to see it fail**, **Step 3: implement `HostBoard` and the command**, **Step 4: run**

Run: `cargo test -p cahoots-host`
Expected: PASS, existing simulator and pile tests included.

- [ ] **Step 5: Commit**

```bash
git add crates/host
git commit -m "host: the simulator is a Board; cahoots-host badge runs the badge crate"
```

---

### Task 8: The ESP32-S3 crate becomes a board

**Files:**
- Create: `crates/esp32s3/src/board.rs` (`NanoBoard`)
- Modify: `crates/esp32s3/src/main.rs` (shrinks to init plus `Badge::new(NanoBoard { ... })`, `boot`, loop of `run_once` then `idle_step`), `crates/esp32s3/Cargo.toml` (path deps on `../badge` and `../custodian`), `crates/esp32s3/README.md` ("What the board does" gains the menu)
- Delete: the self-test, `run_once` and `selftest_flash` bodies from `main.rs` once moved (`selftest_flash` becomes `NanoBoard::selftest`)

**Interfaces:**
- Consumes: Task 4 to 6.
- `NanoBoard` wraps `display::Epd`, `Buttons`, `Tested<entropy::Hw>`, `SharedFlash`, `Delay`; `links()` returns `(Box<dyn Link>::new(UsbLink), Box::new(Absent))`; `tier()` returns 1 and the screen label becomes `"TRNG T1"` (the docs call this board T1); `proposal()` returns `("nano", 16 * 1024)`; `log` goes through the existing `log!` macro.

- [ ] **Step 1: Build before changing**

Run (in `crates/esp32s3`, after `source ~/.cargo/esp-env.sh`): `cargo build --release`
Expected: success, as a baseline.

- [ ] **Step 2: Implement `NanoBoard` and shrink `main.rs`**

`main.rs` keeps peripheral init (SPI, pins, TRNG source, flash open, heap, panic handler) and ends with:

```rust
let mut badge = Badge::new(NanoBoard { /* fields */ });
badge.boot();
loop {
    if badge.circle().is_none() {
        if let Err(e) = badge.run_once() { log!("{e}"); badge.board().display().show_status(&e); badge.board().delay_ms(1500); }
    } else {
        badge.idle_step();
    }
}
```

- [ ] **Step 3: Build**

Run: `cargo build --release`
Expected: success. Then, if a Nano is attached: `cargo run --release`, and `cargo run -p cahoots-host -- pile --cards 3 --serial /dev/cu.usbmodem* --name bench --state /tmp/pile` from the workspace root completes a ceremony as the README describes.

- [ ] **Step 4: Commit**

```bash
git add crates/esp32s3
git commit -m "esp32s3: the Nano is a Board; main is peripheral setup and the badge loop"
```

---

### Task 9: Docs and the roadmap

**Files:**
- Modify: `README.md` (crate table: add `crates/custodian` and `crates/badge`; the `crates/provider` row says it holds the memory and file custodians), `docs/plan.md` (a milestone `M4e Custodian` done: contract, wire protocol, badge crate, host and Nano boards; `M4d` reworded to use the custodian protocol over USB), `crates/ADAPTERS.md` (adapters consume `Custodian`), `docs/protocol.md` (a short section "Custodian messages" listing the request and response table and the two frame kinds, marked informative), `site/docs/docs.js` (no change unless a new doc page is added)

- [ ] **Step 1: Make the edits** in the plain voice; no new terms beyond `custodian`, `consumer`, `badge crate`.
- [ ] **Step 2: Verify** `cargo run -p cahoots-vectors -- --check` still passes (nothing in vectors changed) and the docs site renders the protocol page locally.
- [ ] **Step 3: Commit**

```bash
git add README.md docs crates/ADAPTERS.md
git commit -m "docs: the custodian contract and the badge crate"
```
