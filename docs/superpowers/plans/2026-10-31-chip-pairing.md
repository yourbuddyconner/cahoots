# Chip Sessions, Pairing and the Card Circle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A phone taps a badge once, the owner presses both buttons, and from then on the phone reads and posts in the badge's circles by tapping for keys, with the pad and the seat key never leaving the chip.

**Architecture:** The badge authenticates to its chip with a user-id session over the I2C bus it alone reaches, every object names only the identities that may touch it, and no named identity id is ever left empty for a tap to claim. One phone slot per badge holds an AES key the phone made at pairing; the phone opens that slot's SCP03 session over NFC and asks the chip for HMACs under secure messaging. The firmware drops the chip's enable pin when idle so a phone's field boots the chip onto its antenna, and a bench pass-through lets the laptop drive the real applet with the phone crate's own client. The phone keeps a badge's circles as card circles with no pad, no root and no seat key, chatting under the generation key from a tap.

**Tech Stack:** Rust 1.94 workspace (`crates/chip`, `crates/badge`, `crates/rp2350`, `crates/objects`, `crates/phone`, `crates/chat`, `crates/custodian`), hand-encoded SE05x APDUs in the phone crate (no `se05x` crate there), the `aes`, `cmac` and `cbc` crates, Core NFC in SwiftUI.

**Spec:** `docs/superpowers/specs/2026-10-31-badge-and-phone-design.md`, sections 2, 3, 4, 5, 9, 10, 11. Plan 3 of 3, rewritten on 2026-10-31 after a builder's review. The rulings that departed from the spec are in the last section and are carried into the spec by Task 7.

## Global Constraints

- Object ids keep the `0x4C41_kkss` scheme (`crates/chip/src/ids.rs`). New kinds, each with its slot byte's meaning: `Slot = 5` (badge-wide, slot byte 0, the one phone slot, an AES authentication object), `Hello = 6` (slot byte = `circle_slot * 8 + other_seat`, circle slots 0 to 15, seats 0 to 7), `Public = 7` (slot byte = circle slot, a binary object), `Inbox = 8` (badge-wide, slot byte 0, a binary object), `Owner = 9` (badge-wide, slot byte 0, the badge's user-id authentication object). `KeyId::parse` reports the kind and the raw slot byte; `ChipRecord::reconcile`, `delete_slot` and `delete_all` treat kinds 5, 8 and 9 as badge-wide and never delete them, and treat kind 6 by its circle slot.
- The badge's identity is a user-id authentication object. Its 16-byte secret is `HKDF-SHA256(ikm = the RP2350's unique board id from the ROM, salt = "cahoots/owner/v1", info = empty)`, so a reflash recovers it and nothing has to be stored; the host's and the tests' software badges use a fixed test secret. The object's policy allows read-attributes for any session and nothing else for anyone (the applet cannot delete an authentication object through a session that it authenticates, and the plan does not rely on deleting it). It is written with `max_attempts` of 0, which the applet reads as unlimited, so a tap cannot lock the badge out by failing on purpose.
- No identity id named by a policy is ever empty at the end of an I2C conversation. The firmware creates the phone slot at first boot as a placeholder AES authentication object with a random key nobody holds, policy delete for `Owner` only. Pairing deletes and recreates it with the phone's key inside one owner conversation; unpairing deletes and recreates the placeholder the same way. A tap cannot interleave with an I2C conversation because the chip serves one interface per boot.
- Policies, fixed at creation, name identities by authentication object id, with `REQUIRE_SM` on every entry that names the slot: seat keys allow sign, attest, read-public and delete for `Owner` only; root HMAC objects allow sign (the applet's name for MAC) for `Owner` and `Slot`; hello objects the same; the `Public` object allows read for any session and write and delete for `Owner`; the `Inbox` allows write for any session and read, write and delete for `Owner`; grant use counters and the issue counter allow read, write and delete for `Owner` only. A slot may MAC any data with a root or a hello object; that is understood and documented, and the only keys derived from a root are chat generation keys and seal keys.
- The chip operations a slot's session asks for, by label: `HMAC(root, "chat/gen" || circle_id || generation_id)`; `HMAC(hello_object, "cahoots/words/v1" || speaker || counter_le || 0x01)` whose first four bytes read little-endian give the two words; `HMAC(root, "seal/card" || circle_id)`. A hello object's value is `HMAC-SHA256(32 zero bytes, pair_key)`, the extract step of `cahoots_core::pairwise::words`.
- The phone slot's session is the applet's AESKey session: `CreateSession(slot id)`, SCP03 `INITIALIZE UPDATE` and `EXTERNAL AUTHENTICATE` with the phone's static 16-byte AES keys (ENC, MAC, DEK all equal to one key the phone drew at pairing, as the applet's AESKey object holds one key), then every command under SCP03 secure messaging at security level C-MAC, C-ENC, R-MAC, R-ENC. The phone keeps the key in a keychain item that Face ID gates (`kSecAttrAccessControl` with `userPresence`), so a tap asks for Face ID before the key is read. The EC key session against the enclave key is deferred: the applet's variant of it is not documented in this repository, its key model differs from the spec's assumption, and nothing on the bench can prove it before the TestFlight build. The slot's layout allows an EC key later.
- Upgrade: the first boot of this firmware wipes the chip's objects and the badge's circle records, because every object made before it names every session and would be reachable by a tap once the antenna is live. The badge shows "updated for phones. Make a new circle." once. The chip record format moves from `CAHC` to `CAHC2` with a `phone: Option<PhoneSlot>` field; a `CAHC` record is read as empty.
- The device grant is a CWT grant with a new detail `cahoots:device` whose actions are `read`, `post` and `hello`, scoped to the circle, no channel detail, no `log_not_after`. `Claims::kind` learns `Kind::Device { circle, actions }`. It is signed per circle at pairing and carried in the public object for later use; nothing in this plan verifies it except the tests that pin its format. Channel management from a card circle, and a device acting under a chain, are deferred with the grant's `manage` action.
- One phone per badge in this plan. The chat's sender byte is the seat, so two devices chatting as one seat would reuse keys; the spec's four slots wait for per-device sender separation.
- Chat for a card circle: the phone holds the generation key from a tap. A bump that a badge produced at unpair is a seat-signed payload the phone forwards: `ChatSession::send_bump` accepts a payload from a session without the root when the session holds the current generation's key, seals it under the current epoch, and then moves to `NeedsGeneration(new id)` until `install_generation`. Members with the root verify the seat signature as plan 2 made them.
- The public object holds, as canonical CBOR: `{0: circle_id, 1: relay, 2: sealed manifest, 3: signed card or null, 4: device grant or null, 5: current generation id, 6: pending bump payload or null, 7: badge nonce (16 random bytes drawn at first boot), 8: paired phone's public key or null}`. It is rewritten at keep, pairing and unpairing. The inbox holds `{0: phone public key (33), 1: name, 2: AES key (16)}`, written by any session and readable only by `Owner`.
- The pairing words: four words from `SHA-256("cahoots/pair-phone/v1" || phone_public || badge_nonce)`, drawn from the check-words list the way `cahoots_core::checkwords` draws six.
- A revoked phone learns it is revoked from the public object (field 8 no longer holds its key), not from a status word. A tap that finds no tag within the session's time says "No card found. Hold the phone to the chip." A status word `6985` or `6982` on session open says "The card is busy with its badge. Try again in a moment."
- The firmware: `Board::chip_begin(&mut self) -> Option<&mut dyn ChipOps>` raises the pin, waits the chip's start delay, builds the driver and opens the owner session; `Board::chip_end(&mut self)` closes the session and drops the pin. The badge wraps every chip use in the pair. A tap that lands while the pin is high gets no tag.
- Bench: the firmware's `bench` feature gains an APDU pass-through frame (kind 32, `Apdu`): the laptop sends raw APDUs over USB serial and the firmware forwards them to the chip over I2C in no session and returns the answer, so the phone crate's client runs on the laptop against the real applet. The pass-through is compiled only with the `bench` feature and never in a shipped firmware.
- The software chip (`crates/chip/src/soft.rs`) models identities, sessions, policies (including `REQUIRE_SM`, checked as "the session was opened with secure messaging"), binary objects, AES authentication objects and SCP03 at the level of "the host proved the key", and refuses what the real chip refuses.
- Copy is plain words: no em dashes, no semicolons in anything a person reads, no coined names. The hardware member is a badge or a card, a phone paired to a badge holds "card circles".
- Commit per task, lowercase area prefix and a sentence. iOS 17 floor, UniFFI bindings regenerated by `scripts/ios-framework.sh`.

## Review Focus

- A tap by a phone that is not paired, in no session, can read the public object and write the inbox and nothing else: it cannot MAC a root or a hello object, sign, create an object at the slot's id, or delete the owner object. Test in Task 1 (software chip) and Task 5 (the real applet through the pass-through).
- Unpairing ends the next tap at once. Test in Task 2 and Task 5.
- A circle kept before this firmware is gone after the upgrade and the badge says so. Test in Task 1.
- A card circle whose badge is out of reach after a bump shows "Tap your card to catch up" and nothing worse. Test in Task 4 (`needs_tap`).
- A tap during an I2C conversation finds no tag and the phone says so; the pin is low again within the conversation's end. Test in Task 1 (pin log on the test board) and Task 5 (bench log).

---

### Task 1: Identities, policies, the owner session and the enable pin

**Files:**
- Modify: `crates/chip/src/ops.rs` (`Identity`, `ObjectPolicy`, sessions, binary and auth objects, SCP03 marker), `crates/chip/src/ids.rs` (new kinds and `is_badge_wide`), `crates/chip/src/se05x.rs` (policies per identity with `ObjectId::INVALID` for no session, `WriteUserId`, `CreateSession`, `VerifySessionUserId`, `CloseSession`, `WriteBinary` chunked at the crate's APDU limit, `ReadObject`, `WriteSymmKey` with `is_auth` for the slot placeholder), `crates/chip/src/soft.rs`
- Modify: `crates/badge/src/chip.rs` (`ChipRecord` v2 with `phone: Option<PhoneSlot>`, badge-wide kinds in `reconcile`, `delete_slot`, `delete_all`, first-boot setup: owner object, slot placeholder, inbox), `crates/badge/src/board.rs` (`chip_begin`/`chip_end` replace `chip()`; the chip record store reads `CAHC` as empty and writes `CAHC2`), `crates/badge/src/badge.rs`, `crates/badge/src/apps.rs`, `crates/badge/src/grants.rs` (every chip use through begin and end; the upgrade wipe at first boot with the one-time screen), `crates/host/src/board.rs` and `crates/host/src/card.rs` (the host board implements begin and end over `SoftChip` with the test owner secret)
- Modify: `crates/rp2350/src/main.rs` (the pin and the driver move into `PicoBoard`; the unique board id read from the ROM for the owner secret), `crates/rp2350/src/board.rs` (`chip_begin` raises the pin, 10 ms, builds `Se05xChip`, opens the owner session; `chip_end` closes and drops the pin), `crates/rp2350/src/store.rs` (`CAHC2`)
- Create: `crates/badge/src/test_support.rs` (behind a `test-support` feature: `FakeBoard` moved from `crates/badge/tests/common/mod.rs`, with a pin log and `SoftChip`) so the phone crate's tests in Task 4 can reuse it
- Test: `crates/chip/tests/policies.rs` (new), `crates/badge/tests/chip_seat.rs`, `crates/badge/tests/upgrade.rs` (new)

**Interfaces:**
- Produces in `cahoots_chip`: `pub enum Identity { None, Owner, Slot }`; `pub struct ObjectPolicy { sign: Vec<Identity>, read: Vec<Identity>, write: Vec<Identity>, delete: Vec<Identity>, attest: Vec<Identity>, require_sm: bool }` with `owner_only()`, `owner_and_slot_mac()` (sign for Owner and Slot, delete for Owner, `require_sm: true`), `public_object()`, `inbox()`, `placeholder_slot()`; `ChipOps` gains `open_owner(&mut self, secret: &[u8; 16])`, `close_session(&mut self)`, `put_user_id(id, secret, policy)`, `put_aes_auth(id, key: &[u8; 16], policy)`, `write_binary(id, data, policy)`, `rewrite_binary(id, data)`, `read_binary(id)`, and every creating call takes `&ObjectPolicy`; `ChipError::NotAllowed` for `6982` and `ChipError::BadSession` for a failed session open.
- Produces in `cahoots_chip::soft`: `SoftChip::open_slot_with_sm_for_test()` and `open_slot_without_sm_for_test()` so tests can show `REQUIRE_SM` refusing a slot session that did not set up secure messaging; `SoftChip::no_session_for_test()`.
- Produces in the badge: `Board::chip_begin`/`chip_end`; `chip::first_boot(chip, owner_secret, badge_nonce)` creating the owner object, the placeholder slot and the inbox; `ChipRecord { slots, phone: Option<PhoneSlot>, badge_nonce: [u8; 16], generation: u8 }` where `generation == 2` marks a record made by this firmware and anything else triggers the upgrade wipe; `PhoneSlot { name: String, public: [u8; 33] }`.

- [ ] **Step 1: Write the failing tests.** `crates/chip/tests/policies.rs`: a seat key made `owner_only()` signs in the owner session and returns `NotAllowed` in a slot session and in no session; a root made `owner_and_slot_mac()` MACs in a slot session opened with secure messaging, returns `NotAllowed` in a slot session opened without it and in no session; the public object reads in no session and refuses a write there; the inbox accepts a write in no session and refuses a read there; a no-session attempt to create an object at the slot's id or the owner's id is refused because the id exists (`Status(0x6A89)` or the soft chip's `Exists`); `open_owner` with the wrong secret fails. `crates/badge/tests/chip_seat.rs`: keeping a circle writes the seat key `owner_only()` and the root `owner_and_slot_mac()`, and the chip record round-trips as `CAHC2`. `crates/badge/tests/upgrade.rs`: a board whose chip holds objects made the old way and whose record is `CAHC` boots, wipes them, writes a `CAHC2` record with the owner object, the placeholder slot and the inbox present, and shows "updated for phones. Make a new circle." once; the test board's pin log shows high only between `chip_begin` and `chip_end`.
- [ ] **Step 2: Run and see them fail.** Run: `cargo test -p cahoots-chip -p cahoots-badge`.
- [ ] **Step 3: Implement.** In `se05x.rs` a policy is a `PolicySet` with one `Policy` per identity: `ObjectId::INVALID` for no session, the owner object's id, the slot's id. The user id object is written with `WriteUserId` and the owner session opened with `CreateSession` then `VerifySessionUserId`; session commands go through `run_session_command`. Move `FakeBoard` into `test_support.rs` and point the badge tests at it.
- [ ] **Step 4: Run the tests and build the firmware**

Run: `cargo test -p cahoots-chip -p cahoots-badge -p cahoots-host` and the Pico build as `scripts/bench.sh` does it
Expected: PASS and a firmware binary.

- [ ] **Step 5: Bench.** Flash the board with `scripts/bench.sh`'s flash step. Run `cahoots-host logs --serial /dev/cu.usbmodempico21` and confirm the boot log shows the upgrade wipe once, the owner session opening, the applet version read through it, and the pin dropping ("se051: pin low"). Run the bench ceremony (`scripts/bench.sh --no-flash`) so the board keeps a circle under the new policies, and confirm the log shows the seat key and root written with their policies. Record the log lines in the report.
- [ ] **Step 6: Commit**

```bash
git add crates/chip crates/badge crates/rp2350 crates/host
git commit -m "chip: the badge owns its chip: an owner session, identities on every object, no empty slot, the pin at rest low, and an upgrade wipe"
```

### Task 2: Pairing on the badge, hello objects, the public object and the device grant

**Files:**
- Modify: `crates/badge/src/badge.rs` (keep writes hello objects from pair keys and the public object; the `pair a phone` and `phone` pages at the top-level chooser; `pair_read_inbox`, `pair_accept`, `unpair`), `crates/badge/src/screens.rs` (the pairing screens and the four words), `crates/badge/src/chip.rs`
- Create: `crates/badge/src/public.rs` (the public object's CBOR, `MAX_PUBLIC = 8192`) and `crates/badge/src/pairing.rs` (the words derivation)
- Modify: `crates/hal/src/ring.rs` (if `RingCeremony::finish` drops pair keys, expose them in the result the badge keeps)
- Modify: `crates/objects/src/cwt.rs` (`DEVICE_TYPE`, `DEVICE_ACTIONS = ["read", "post", "hello"]`, `Kind::Device { circle, actions }`, decode and canonical form), `crates/objects/src/grant.rs` (`issue_device_grant(circle_id, seat, grantee: SeatPublic, nonce) -> Claims`, `sign_device_grant`, `verify_device_grant(bytes, manifest) -> Result<(u8 seat, SeatPublic grantee, actions), _>`)
- Modify: `crates/chat/src/session.rs` (`send_bump` from a session without the root when it holds the current generation; after sending it moves to pending `NeedsGeneration(new id)`), `crates/chat/src/bump.rs` unchanged
- Test: `crates/badge/tests/pairing.rs` (new), `crates/objects/tests/device_grant.rs` (new), `crates/chat/tests/generations.rs` (a device forwards a seat-signed bump)

**Interfaces:**
- Consumes Task 1.
- Produces: `Badge::pair_read_inbox() -> Option<Inbox>`, `Badge::pair_accept(inbox) -> Result<(), BadgeError>`, `Badge::unpair()`; `public::PublicObject { circle_id, relay, manifest, card, grant, generation, pending_bump, badge_nonce, phone_public }` with `encode`/`decode`; `pairing::words(phone_public: &[u8; 33], badge_nonce: &[u8; 16]) -> [&'static str; 4]`; `Inbox { public: [u8; 33], name: String, key: [u8; 16] }` with `encode`/`decode`; `cahoots_objects::cwt::Kind::Device`; `cahoots_objects::grant::{issue_device_grant, sign_device_grant, verify_device_grant}`; `ChatSession::send_bump` relaxed as the Global Constraints say.
- Pairing on the badge: the top-level chooser gains `pair a phone` (Both: read the inbox, show "pair with {name}?" and the four words; Both again: in one owner conversation delete the placeholder, write the slot with the inbox's key under `placeholder_slot()`'s delete rule, sign one device grant per circle with each circle's seat key, rewrite each public object with the grant, the phone's public key and the current generation, record `PhoneSlot`, clear the inbox; Left declines and clears the inbox) and `phone` (shows the paired name; Both unpairs: in one owner conversation delete the slot, recreate the placeholder, draw a fresh generation id per circle, sign a bump per circle with the seat key, write each public object with the bump pending and `phone_public` null, record the new generation id per circle so a later tap gets it). If a phone is already paired, `pair a phone` says "unpair the current phone first".

- [ ] **Step 1: Write the failing tests.** `crates/badge/tests/pairing.rs` on the test-support board: keep a circle; a no-session write to the inbox with a key, name and AES key; `pair_read_inbox` returns them; `pair_accept` makes the slot session open with that AES key (soft chip), the public object holds a grant that `verify_device_grant` accepts against the manifest and the phone's public key, and `words` on the phone side equals the badge's; a slot session MACs the root with the chat/gen label and a hello object and cannot sign with the seat key, read the inbox, or delete the owner object; `unpair` makes the slot's old key fail to open, the public object holds a pending bump that `verify_bump` accepts and a null phone key, and a second `pair_accept` works; `pair_accept` with a phone already paired is refused. `crates/objects/tests/device_grant.rs`: encode canonically, decode to `Kind::Device`, refuse unknown actions, verify against the manifest. Chat: a session built with `from_generation` forwards a seat-signed bump, and a root holder receiving it moves; the device then reports `NeedsGeneration`.
- [ ] **Step 2: Run and see them fail.** Run: `cargo test -p cahoots-badge -p cahoots-objects -p cahoots-chat`.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests**

Run: `cargo test -p cahoots-badge -p cahoots-objects -p cahoots-chat -p cahoots-host`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/badge crates/objects crates/chat crates/hal
git commit -m "badge: pair a phone by tap and press, hello objects from pair keys, a public object per circle with a device grant, and a bump at unpair"
```

### Task 3: SCP03 and the chip over NFC in the phone crate

**Files:**
- Create: `crates/phone/src/scp03.rs` (key derivation per GlobalPlatform SCP03 with the applet's constants, C-MAC, C-ENC, R-MAC, R-ENC, the chaining value; tests against the worked examples in the GlobalPlatform SCP03 specification's annex and a round-trip against the software chip's SCP03 model)
- Create: `crates/phone/src/apdu.rs` (hand-encoded APDUs for SELECT, `CreateSession`, `InitializeUpdate`, `ExternalAuthenticate`, `CloseSession`, `ReadObject`, `WriteBinary`, `MACOneShot`, with the applet's TLV tags recorded in the doc comment from the applet's APDU reference; no `se05x` crate dependency)
- Create: `crates/phone/src/nfc.rs` (`Nfc` callback trait, `Tap`, the operations)
- Modify: `crates/phone/Cargo.toml` (`aes`, `cmac`, `cbc`), `crates/phone/src/lib.rs` (`Phone::open` takes an `Nfc` callback), `crates/chip/src/soft.rs` (an APDU front end `SoftChip::apdu(&mut self, apdu: &[u8]) -> Vec<u8>` that decodes the same APDUs so tests route the phone's client into the software chip; it is the chip-side truth the badge tests already use)
- Create: `ios/Cahoots/NfcTransport.swift` (one `NFCTagReaderSession` per tap, `begin(prompt)`, `transceive(apdu)`, `end(message, failed)`), modify `ios/Cahoots/Core.swift` (pass it to `Phone::open`), `ios/Cahoots/EnclaveSigner.swift` (a keychain item for the slot key with `userPresence`: `store_slot_key(key)`, `slot_key() -> key`, both on the `Signer` trait as `fn put_secret(label, bytes)` and `fn get_secret(label) -> bytes`)
- Test: `crates/phone/tests/nfc.rs` with a `FakeNfc` over `SoftChip::apdu`

**Interfaces:**
- Consumes Task 1's policies and Task 2's inbox and public object formats.
- Produces: `#[uniffi::export(callback_interface)] pub trait Nfc: Send + Sync { fn begin(&self, prompt: String) -> Result<(), PhoneError>; fn transceive(&self, apdu: Vec<u8>) -> Result<Vec<u8>, PhoneError>; fn end(&self, message: String, failed: bool) -> Result<(), PhoneError>; }`; `nfc::Tap` with `select()`, `read_public(circle_slot) -> PublicObject`, `list_public() -> Vec<(u8, PublicObject)>` (tries circle slots 0 to 15 and keeps the ones that read), `write_inbox(&Inbox)`, `open_slot(key: &[u8; 16]) -> Result<Session>`, `Session::hmac_root(circle_slot, data)`, `Session::hmac_hello(circle_slot, other, data)`; `PhoneError::Nfc { message }`, `PhoneError::NoCard` ("No card found. Hold the phone to the chip."), `PhoneError::CardBusy` ("The card is busy with its badge. Try again in a moment."); `Signer` gains `put_secret` and `get_secret`.

- [ ] **Step 1: Write the failing tests.** `scp03.rs` unit tests: key derivation and C-MAC against the specification's worked examples (cite the annex and the values in the test), a wrap and unwrap round trip. `nfc.rs` over `FakeNfc`: a no-session tap lists public objects and writes the inbox; `open_slot` with the paired key succeeds and MACs; with a wrong key it fails to open; a MAC request for a seat key id is `NotAllowed`; a session open answered `6985` maps to `CardBusy`; no tag maps to `NoCard`.
- [ ] **Step 2: Run and see them fail.** Run: `cargo test -p cahoots-phone --test nfc`.
- [ ] **Step 3: Implement.** Keep `nfc.rs` free of UI and of the `Phone` lock's other state. Regenerate the bindings; add `NfcTransport.swift`.
- [ ] **Step 4: Run the phone tests and build the app**

Run: `cargo test -p cahoots-phone` and `scripts/ios-build.sh`
Expected: PASS and `** BUILD SUCCEEDED **`.

- [ ] **Step 5: Commit**

```bash
git add crates/phone crates/chip ios/Cahoots
git commit -m "phone: the chip over NFC: SCP03 under a slot's key, reading a card's public things, writing its inbox"
```

### Task 4: Card circles on the phone

**Files:**
- Modify: `crates/phone/src/store.rs` (`CircleRecord.card: Option<CardCircle>` with `CardCircle { badge_name, circle_slot, badge_public: Vec<u8>, generation_key: Option<[u8; 32]> }`; `root` and `pair_keys` empty for a card circle; `create_card_circle` writes no `pad.bin`), `crates/phone/src/chat.rs` (`Circle::load` builds `ChatSession::from_generation`, or `import_generation_state`, for a card circle; `poll` turns `NeedsGeneration` into `needs_tap`; `hello` for a card circle asks the tap), `crates/phone/src/lib.rs` (`pair_card_start(name) -> PairPreview { badge_name, circles: Vec<String>, words: Vec<String> }`: one tap that writes the inbox and lists public objects; `pair_card_finish() -> Vec<String>`: one tap that opens the slot, lists public objects again for the grants, keeps each as a card circle, fetches each generation key, publishes each badge card into its chat, forwards any pending bump then fetches the new key; `forget_card()`; `tap_for_keys(circle)`; `CircleSummary.kind` and `needs_tap`; `history`, `poll`, `send_chat`, `hello_words` for card circles)
- Modify: `crates/phone/Cargo.toml` (dev-dependency `cahoots-badge` with `test-support`)
- Test: `crates/phone/tests/card_circle.rs` (new)

**Interfaces:**
- Consumes Tasks 2 and 3, plan 2's session API.
- Produces the `Phone` methods above; `PhoneError::NotPaired` ("This card no longer knows this phone.") when the public object's phone key is not ours.

- [ ] **Step 1: Write the failing tests.** Pairing end to end against the test-support badge through `FakeNfc`: `pair_card_start` returns the badge's name and four words equal to the badge's; after `pair_accept` on the badge, `pair_card_finish` keeps one card circle with `kind == "card"`, no root in its record and no `pad.bin`, whose chat reads a message another member posted and whose `circle_members` shows the badge's card; a bump posted by a root holder makes `poll` report `needs_tap` and `tap_for_keys` clears it; after the badge unpairs, `tap_for_keys` fails with `NotPaired` and `forget_card` removes the card circles; hello words from a tap equal the other member's; a pending bump in the public object is forwarded by `pair_card_finish` and a root holder moves.
- [ ] **Step 2: Run and see them fail.** Run: `cargo test -p cahoots-phone --test card_circle`.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the phone tests**

Run: `cargo test -p cahoots-phone`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/phone
git commit -m "phone: card circles: pair by tap and press, chat under a generation key from a tap, hello words by tap"
```

### Task 5: The bench pass-through and the real applet

**Files:**
- Modify: `crates/rp2350/src/main.rs` and `link.rs` (bench feature: frame kind 32 `Apdu` in, kind 33 `ApduReply` out; the firmware forwards the APDU to the chip in no session with the pin raised for the exchange), `crates/hal/src/pile.rs` (the two kinds in the table, ignored by the ceremony and the pile), `crates/host/src/link.rs` (`SerialLink::apdu(&mut self, apdu) -> Vec<u8>`)
- Create: `crates/host/src/bin/chip-probe.rs` or a `cahoots-host chip` command: runs the phone crate's `nfc.rs` client over the serial pass-through (`Nfc` implemented over `SerialLink::apdu`), with subcommands `public`, `inbox --name --key`, `open --key`, `hmac --circle --label`
- Create: `scripts/pair-bench.sh`: flash the bench build, keep a circle, write the inbox from the laptop with a test key, press `pair a phone` and Both twice through the bench press frames, then open the slot from the laptop with the test key, MAC the root, confirm the public object carries the grant; then unpair through the bench presses and confirm the old key no longer opens and the public object carries a pending bump
- Modify: `crates/phone` dev-dependencies as needed so the host can depend on the phone crate's `nfc` module (or move `apdu.rs` and `scp03.rs` into `crates/chip` behind a `client` feature so both the phone and the host use them; builder's choice, recorded)
- Test: the bench run; `ios/README.md` and `docs/status.md`

- [ ] **Step 1: Build the bench firmware and flash.** Run `scripts/pair-bench.sh` and fix what the real applet refuses until it ends "pair bench: ok". The things most likely to differ from the software chip: the exact TLVs of `CreateSession` and `WriteUserId`, the policy encoding for `ObjectId::INVALID`, the SCP03 key derivation constants the applet uses, and `WriteBinary`'s chunking. Record each difference in the report and fix the software chip to match the real one.
- [ ] **Step 2: Record** the run in `ios/README.md` ("Tried on the bench") and `docs/status.md`.
- [ ] **Step 3: Commit**

```bash
git add crates scripts/pair-bench.sh ios/README.md docs/status.md
git commit -m "bench: the phone's chip client runs against the real applet through the badge, and pairing works end to end on the board"
```

### Task 6: The app's screens

**Files:**
- Create: `ios/Cahoots/PairCardView.swift`
- Modify: `ios/Cahoots/CirclesView.swift`, `ios/Cahoots/SettingsView.swift`, `ios/Cahoots/ChatView.swift`, `ios/Cahoots/PeopleView.swift`, `ios/Cahoots/AppModel.swift`, `ios/Cahoots/Core.swift`
- Test: `ios/CahootsTests` where the project has view-model tests; the UI is checked on the device by the person

- [ ] **Step 1: Implement the screens** to the spec's section 9 with the sentences from Tasks 3 and 4. The pairing screen: a name field, "Hold the phone to the card", then the four words with "Open 'pair a phone' on your badge and check the words match. Press both buttons, then hold the phone to the card again.", then the circles it brought. The pairing screen's one-line promise: "This phone can read and post in this badge's circles until you unpair it on the badge." Chat in a card circle replaces the composer with "Tap your card to catch up" when `needs_tap`. Settings gains "Pair a card" and "Forget this card". Circles shows a card circle with its badge's face and "card circle" in words.
- [ ] **Step 2: Build**

Run: `scripts/ios-build.sh`
Expected: `** BUILD SUCCEEDED **`.

- [ ] **Step 3: Commit**

```bash
git add ios
git commit -m "ios: pair a card by tap and press, and card circles that tap for their keys"
```

### Task 7: Docs

**Files:**
- Modify: `docs/superpowers/specs/2026-10-31-badge-and-phone-design.md` (the rulings below folded into sections 2, 4, 5, 6), `docs/status.md`, `docs/plan.md` (E1 row, product row 3, M4j split), `docs/protocol.md` (the device grant detail, the public object, the inbox, the chip operations and policies), `ios/README.md` (the person's steps)

- [ ] **Step 1: Write the docs** in plain sentences.
- [ ] **Step 2: Commit**

```bash
git add docs ios/README.md
git commit -m "docs: pairing by tap, the chip's identities and policies, the device grant and the public object"
```

## Rulings made in this plan, against the spec

- The phone's session is an AES-key SCP03 session, not the EC key session the spec chose, because the applet's EC session is undocumented here, its key model is not what the spec assumed, and nothing on the bench can prove it before the build. The EC session stays the intended upgrade.
- One phone per badge, not four, until the chat separates senders per device.
- The device grant carries no channel detail and nothing verifies it yet; channel management from a card circle is deferred.
- A badge's bump at unpair is forwarded by the next phone that taps, and the chat crate lets a device forward a seat-signed bump.
- The first boot of this firmware wipes old circles and the chip, since their objects would be reachable by a tap.
- The owner secret derives from the board's unique id so a reflash recovers it.
- No `se05x` crate in the phone app; its APDUs are hand-encoded, so the LGPL library stays in the firmware only.

## The person's steps after the TestFlight build

1. Badge on USB with the antenna switch on, flashed from this branch. Its old circles are gone; make a new one with the bench ceremony or a pile.
2. Install the build. Settings, "Pair a card", type a name, hold the phone to the chip. Face ID asks once.
3. On the badge, open "pair a phone" and press both. Check the four words match the phone's. Press both again.
4. Hold the phone to the chip again. The circle appears as a card circle with the badge's face.
5. Open its chat. Post from the browser or the other phone and read it on the card circle. Tap when the app asks.
6. On the badge, "phone", press both. The next tap says the card no longer knows this phone.
