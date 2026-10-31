# Chip Sessions, Pairing and the Card Circle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A phone taps a badge once, the owner presses both buttons, and from then on the phone reads and posts in the badge's circles by tapping for keys, with the pad and the seat key never leaving the chip.

**Architecture:** The badge authenticates to its chip with a user-id session over the I2C bus it alone can reach, and every new object names only the identities that may touch it. Four badge-wide slots hold paired phones' keys; a phone opens a slot's session over NFC and asks the chip for HMACs. The firmware drops the chip's enable pin when idle so a phone's field boots the chip onto its antenna. The phone keeps a badge's circles as card circles with no pad, no root and no seat key, chatting under the generation key from a tap, and acting in the circle under a device grant the badge signed at pairing.

**Tech Stack:** Rust 1.94 workspace (`crates/chip`, `crates/badge`, `crates/rp2350`, `crates/objects`, `crates/phone`, `crates/custodian`), the `se05x` 0.4.2 crate's command encoders used transport-free, Core NFC in SwiftUI.

**Spec:** `docs/superpowers/specs/2026-10-31-badge-and-phone-design.md`, sections 2, 3, 4, 5, 9, 10, 11. Plan 3 of 3. Plan 2's chat generations (`ChatSession::from_generation`, `install_generation`, `NeedsGeneration`, `MessageType::Card`) and cards with faces are consumed here.

## Global Constraints

- The chip's object id scheme stays `0x4C41_kkss` (`crates/chip/src/ids.rs`). New kinds: `Slot = 5` (badge-wide, slots 0 to 3, authentication objects holding a phone's P-256 public key), `Hello = 6` (per circle slot, one object per other seat: the id packs `circle slot` in the low byte and the other seat in bits 8 to 10 of a second kind byte, builder's choice recorded in `ids.rs`), `Public = 7` (per circle slot, a binary object), `Inbox = 8` (badge-wide, slot 0, a binary object), `Owner = 9` (badge-wide, slot 0, the badge's user-id authentication object).
- The badge's identity is a user-id authentication object whose secret is 16 random bytes the firmware draws once and keeps in flash beside the chip record. Every I2C conversation opens a session with it. It never crosses the air: an NFC reader has no session that names it.
- Policies, fixed at creation, name identities by authentication object id: seat keys allow sign, attest, read-public and delete for `Owner` only; root HMAC objects allow sign (the applet's name for MAC) for `Owner` and the four `Slot`s; hello objects the same; the `Public` object allows read for any session and write and delete for `Owner`; the `Inbox` allows write for any session and read, write and delete for `Owner`; `Slot` objects themselves allow delete for `Owner` only. The pad key, when M4j's second half comes, names `Owner` only.
- The chip operations a slot's session may ask for, by label: `HMAC(root, "chat/gen" || circle_id || generation_id)`; `HMAC(hello_object, "cahoots/words/v1" || speaker || counter_le || 0x01)` whose first four bytes are the words step; `HMAC(root, "seal/card" || circle_id)`.
- A hello object's value is the words derivation's extract step, `HMAC-SHA256(zero salt of 32 bytes, pair_key)`, so a tap computes `pairwise::words` without the pair key.
- The device grant is a CWT grant (`crates/objects/src/cwt.rs`) with a new detail `cahoots:device` whose actions are `read`, `post`, `hello`, `manage`, scoped to the circle, plus the `cahoots:channel` detail for every channel of the circle at depth two and budget zero; `log_not_after` is absent (the badge has no log position). Chain verifiers accept a root link that carries both details.
- The public object holds, as canonical CBOR: `{0: circle_id, 1: relay, 2: sealed manifest, 3: signed card or null, 4: [device grant per slot or null], 5: current generation id, 6: [signed revocation per slot or null], 7: [signed bump or null]}`. It is rewritten at keep, pairing and unpairing. The inbox holds `{0: phone public key (33), 1: name}`.
- The firmware drops the enable pin when it has nothing to say to the chip and raises it, waits the chip's start delay, opens the owner session and runs the command, then drops it again. A chip conversation is one call in the chip layer; the rest of the badge sees no change.
- The simulated badge and the software chip (`crates/chip/src/soft.rs`) model identities, sessions and policies and refuse what the real chip refuses, so every test below runs without hardware.
- The phone's slot session is the chip's EC key session against the phone's enclave key, with the SCP03 secure messaging the applet requires after it (C-MAC and C-ENC), implemented in the phone crate on top of the `se05x` crate's command encoders. If the EC key session cannot be made to work against the real chip within Task 5's bench step, the fallback is an AES-key session (the applet's AESKey authentication object, with the `aes-session` handshake the crate already has) whose keys the phone makes at pairing and keeps in a keychain item Face ID gates; the slot's layout and the pairing flow are the same either way.
- Copy is plain words: no em dashes, no semicolons in anything a person reads, no coined names. The hardware member is a badge or a card, the laptop program is the bridge, a phone paired to a badge holds "card circles".
- Commit per task, lowercase area prefix and a sentence. iOS 17 floor, UniFFI bindings regenerated by `scripts/ios-framework.sh`.

## Review Focus

- A tap by a phone that is not paired can read the public object and write the inbox and nothing else: it cannot derive a chat key, sign, or read a hello object. Test in Task 2 (software chip refuses) and Task 7 (bench).
- Unpairing ends the next tap at once, before any bump is published. Test in Task 2.
- A circle kept before this plan cannot be paired and the badge says so instead of failing silently. Test in Task 2.
- A card circle on a phone whose badge is out of reach after a bump shows "Tap your card to catch up" and nothing worse. Test in Task 4.
- The firmware raises the enable pin only around a chip conversation; a tap that lands during one is refused by the chip (it is on I2C) and the phone says "Try again in a moment". Test in Task 1 (the pin's state around a call) and Task 7 (bench).

---

### Task 1: The enable pin and the owner session

**Files:**
- Modify: `crates/rp2350/src/main.rs:191-203` (chip construction), `crates/rp2350/src/board.rs` (`_chip_enable` becomes `chip_enable`, used; `chip()` wraps each call)
- Modify: `crates/chip/src/ops.rs` (`ChipOps` gains `open_owner`, `close`, binary objects, auth objects), `crates/chip/src/se05x.rs` (sessions, policies by identity, `WriteUserId`, `CreateSession`, `VerifySessionUserId`, `CloseSession`, `WriteBinary`, `ReadObject` chunked), `crates/chip/src/ids.rs` (new kinds), `crates/chip/src/soft.rs` (identities, sessions, policies)
- Modify: `crates/badge/src/board.rs` (owner secret in the chip record sector, `CAHC` body gains it), `crates/badge/src/badge.rs` (open the owner session at boot and before each chip use through one helper)
- Test: `crates/chip/tests/soft.rs`, `crates/chip/tests/policies.rs` (new), `crates/badge/tests/chip_seat.rs`

**Interfaces:**
- Produces in `cahoots_chip`: `pub enum Identity { None, Owner, Slot(u8) }`; `ChipOps` gains `fn open_session(&mut self, who: Identity, secret: &[u8]) -> Result<(), ChipError>` (for `Owner` the 16-byte user id; for `Slot` the session is opened by the phone side and `secret` is unused on the badge), `fn close_session(&mut self) -> Result<(), ChipError>`, `fn put_user_id(&mut self, id: KeyId, user_id: &[u8; 16]) -> Result<(), ChipError>`, `fn put_auth_key(&mut self, id: KeyId, public: &[u8; 33]) -> Result<(), ChipError>`, `fn write_binary(&mut self, id: KeyId, data: &[u8], readers: &[Identity], writers: &[Identity]) -> Result<(), ChipError>`, `fn read_binary(&mut self, id: KeyId) -> Result<Vec<u8>, ChipError>`; every creating call takes a `Policy` argument listing identities: `generate(id, curve, policy: &ObjectPolicy)`, `put_hmac_key(id, key, policy)`, `put_counter(id, value, policy)`; `pub struct ObjectPolicy { pub sign: Vec<Identity>, pub read: Vec<Identity>, pub write: Vec<Identity>, pub delete: Vec<Identity>, pub attest: Vec<Identity> }` with `ObjectPolicy::owner_only()`, `ObjectPolicy::owner_and_slots(sign: bool)`.
- Produces in `cahoots_chip::soft`: `SoftChip` keeps `current: Identity` set by `open_session` (owner secret checked; a slot session opened in tests by `SoftChip::open_slot_for_test(slot)`), stores a policy per object, and returns `ChipError::Status(0x6982)` for an operation the policy does not allow the current identity.
- Produces in the firmware: `PicoBoard::with_chip(|chip| ...)` raises the pin, waits 10 ms, constructs the driver, opens the owner session, runs the closure, closes, drops the pin. `Board::chip()` in the badge crate is replaced by `Board::with_chip(&mut self, f: &mut dyn FnMut(&mut dyn ChipOps) -> R) -> Option<R>`; the host's `HostBoard` and the badge tests' `FakeBoard` implement it over `SoftChip` with the owner session opened.
- Produces in the badge: the owner secret made at first boot if the chip record sector has none; `Describe` gains `identity: "user id in flash"`.

- [ ] **Step 1: Write the failing tests.** `crates/chip/tests/policies.rs`: a key generated with `owner_only()` signs in the owner session and returns `Status(0x6982)` in a slot session and in no session; an HMAC object with `owner_and_slots(true)` MACs in a slot session; a binary object written with readers `[None, Owner, Slot(0..4)]` reads in no session; the inbox pattern (writers any, readers owner) refuses a read in a slot session; `open_session(Owner, wrong secret)` fails. `crates/badge/tests/chip_seat.rs`: the badge keeps a circle with the seat key `owner_only()` and the root `owner_and_slots(true)`, and the chip record sector round-trips the owner secret.
- [ ] **Step 2: Run and see them fail.** Run: `cargo test -p cahoots-chip -p cahoots-badge`.
- [ ] **Step 3: Implement** the chip layer, the software chip, the badge helper and the firmware. In `se05x.rs`, a policy is a `PolicySet` with one `Policy` per identity, using `ObjectId::INVALID` for "no session". Chunk binary writes at the crate's `MAX_APDU_PAYLOAD_LENGTH`. The firmware keeps the driver out of `PicoBoard` and constructs it inside `with_chip`; the chip's start delay after the pin rises is the existing 10 ms.
- [ ] **Step 4: Run the tests and build the firmware**

Run: `cargo test -p cahoots-chip -p cahoots-badge -p cahoots-host` and the Pico build as `scripts/bench.sh` does it (`cargo build --release` in `crates/rp2350` with the bench feature off)
Expected: PASS and a firmware binary.

- [ ] **Step 5: Bench check.** With the board on `/dev/cu.usbmodempico21`, flash with `scripts/bench.sh --no-flash` replaced by its flash step, then run `cahoots-host logs --serial /dev/cu.usbmodempico21` and confirm the boot log shows the applet version through the owner session and the pin dropping after it ("se051: session closed, pin low" or the line you choose). Then, with the Pico powered and the Click board's antenna switch on, run the app's NFC check from the phone that is on the bench... the phone is not available to the agent; instead confirm from the log that the pin stays low for at least a second between conversations, which is what a tap needs.
- [ ] **Step 6: Commit**

```bash
git add crates/chip crates/badge crates/rp2350 crates/host
git commit -m "chip: the badge opens a session with its own identity, every object names who may touch it, and the enable pin rests low"
```

### Task 2: Slots, hello objects, the public object and the inbox on the badge

**Files:**
- Modify: `crates/badge/src/chip.rs` (slot kinds, hello and public per circle, inbox and owner badge-wide, `SlotEntry` gains `paired: [Option<String>; 4]` at the badge level in `ChipRecord`), `crates/badge/src/badge.rs` (keep writes hello objects from the ceremony's pair keys and the public object; `pair a phone` and `phones` pages; unpair), `crates/badge/src/screens.rs` (the two pages, the check words screen), `crates/badge/src/apps.rs` (`ChipRequest::DeviceGrant` is not needed; pairing is on the badge's own screen)
- Modify: `crates/objects/src/cwt.rs` (`DEVICE_TYPE = "cahoots:device"`, `DEVICE_ACTIONS = ["read", "post", "hello", "manage"]`, `Kind::Device { circle, actions, channel_actions }`, decode and canonical form), `crates/objects/src/grant.rs` (`issue_device_grant(circle_id, seat, grantee: SeatPublic, nonce) -> Claims` with the channel detail at depth 2), `crates/channel/src/grant.rs` (`from_claims` accepts a root `Kind::Device` link as if it were a channel grant with the channel detail's actions)
- Create: `crates/badge/src/public.rs` (the public object's CBOR, encode and decode, `MAX_PUBLIC = 8192`)
- Test: `crates/badge/tests/pairing.rs` (new), `crates/objects/tests/device_grant.rs` (new), `crates/channel/tests/` (a device root link admits a member key)

**Interfaces:**
- Consumes Task 1.
- Produces: `ChipRecord { slots, phones: [Option<PhoneSlot>; 4] }` with `PhoneSlot { name: String, public: [u8; 33] }`; `Badge::pair_read_inbox()`, `Badge::pair_accept(slot)`, `Badge::unpair(slot)`; `public::PublicObject { circle_id, relay, manifest, card: Option<Vec<u8>>, grants: [Option<Vec<u8>>; 4], generation: [u8; 32], revocations: [Option<Vec<u8>>; 4], bump: Option<Vec<u8>> }`; the check words for pairing are `checkwords`-style four words from `SHA-256("cahoots/pair-phone/v1" || phone_public || owner_user_id_public_commitment)` where the badge's side of the words is `SHA-256(owner user id)` published in the public object as field 8, so the phone computes the same four words; `cahoots_objects::cwt::Kind::Device`; `cahoots_objects::grant::issue_device_grant`.
- Pairing on the badge: the top-level chooser gains a page "pair a phone" (Both: read the inbox, show "pair with {name}?" and the four words; Both again writes the slot, signs one device grant per circle with each circle's seat key, rewrites each public object, clears the inbox; Left declines and clears the inbox) and a page "phones" listing slots by name (Both on a slot unpairs: delete the slot object, sign a revocation of that slot's grants per circle and a bump per circle with a fresh random generation id, write both into the public objects, and move the badge's own chat generation record so a later keep uses the new id). Circles kept before Task 1 (no slot policies) are listed on the pairing page as "made before pairing, make a new circle" and get no grant.

- [ ] **Step 1: Write the failing tests.** `crates/badge/tests/pairing.rs` on the `FakeBoard` with `SoftChip`: keep a circle; a tap (test helper writing the inbox in no session) with a key and name; `pair_read_inbox` returns them; `pair_accept(0)` writes slot 0, and the public object now holds a grant for slot 0 that `verify_chain` accepts against the manifest; a slot session (`open_slot_for_test(0)`) MACs the root with the chat/gen label and a hello object, cannot sign with the seat key, cannot read the inbox; `unpair(0)` deletes the slot so the slot session fails to open, and the public object holds a revocation and a bump; a fifth phone is refused with "all four slots are taken"; a circle kept with the old `ChipOps` (a test helper writing objects with every-session policies) is reported as unpairable. `crates/objects/tests/device_grant.rs`: a device grant encodes canonically, decodes to `Kind::Device`, and refuses unknown actions. Channel test: a root device link with the channel detail followed by a delegated member link verifies with `govern`.
- [ ] **Step 2: Run and see them fail.** Run: `cargo test -p cahoots-badge -p cahoots-objects -p cahoots-channel`.
- [ ] **Step 3: Implement.** The badge keeps pair keys now: `keep` takes them from the `CeremonyResult` (the ring exposes `pair_keys`; add it to `RingCeremony::finish`'s result if the badge path drops it) and writes a hello object per other seat with `owner_and_slots(true)`, then the public object. The four slots are created at first boot as empty authentication objects? No: the SE05x cannot create an empty auth object, so slot objects are written at pairing time and policies name their ids in advance, which the applet allows (a policy may name an object that does not exist yet). Record that in `chip.rs`'s doc.
- [ ] **Step 4: Run the tests**

Run: `cargo test -p cahoots-badge -p cahoots-objects -p cahoots-channel -p cahoots-host`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/badge crates/objects crates/channel
git commit -m "badge: four phone slots, the pairing pages, hello objects from pair keys, and a public object per circle with a device grant"
```

### Task 3: The phone talks to the chip over NFC

**Files:**
- Create: `crates/phone/src/nfc.rs` (`Nfc` callback trait, the APDU client, SELECT, object reads and writes, session opening, HMAC requests)
- Create: `crates/phone/src/scp03.rs` (SCP03 secure messaging: session key derivation, C-MAC, C-ENC, R-MAC, with test vectors from GlobalPlatform's SCP03 specification)
- Modify: `crates/phone/Cargo.toml` (`se05x = { version = "=0.4.2", default-features = false }`, `iso7816`, `aes`, `cmac`, `cbc`), `crates/phone/src/lib.rs` (`Phone::open` takes an `Nfc` callback beside `Signer` and `Http`), `crates/phone/src/signer.rs` (`Signer` gains `fn agree(&self, label: String, peer_public: Vec<u8>) -> Result<Vec<u8>, PhoneError>` for ECDH with the enclave key)
- Modify: `ios/Cahoots/EnclaveSigner.swift` (a `SecureEnclave.P256.KeyAgreement.PrivateKey` beside the signing key, same access control; `agree`), create `ios/Cahoots/NfcTransport.swift` (`Nfc` implemented over one `NFCTagReaderSession` per tap: `begin(prompt)`, `transceive(apdu) -> response`, `end(message)`)
- Test: `crates/phone/tests/nfc.rs` with a `FakeNfc` that routes APDUs to a `SoftChip` through a small APDU decoder in the test (SELECT, ReadObject, WriteBinary, CreateSession, the EC key session commands, MACOneShot), so the phone crate's client is tested against the same software chip the badge tests use

**Interfaces:**
- Consumes Task 1's `SoftChip` identities and Task 2's object layout.
- Produces: `#[uniffi::export(callback_interface)] pub trait Nfc: Send + Sync { fn begin(&self, prompt: String) -> Result<(), PhoneError>; fn transceive(&self, apdu: Vec<u8>) -> Result<Vec<u8>, PhoneError>; fn end(&self, message: String, failed: bool) -> Result<(), PhoneError>; }`; `nfc::Tap<'a>` with `select()`, `read_public(circle_slot) -> PublicObject`, `write_inbox(&Inbox)`, `open_slot(slot, signer, label) -> Result<Session>` (EC key session then SCP03), `hmac_root(circle_slot, data)`, `hmac_hello(circle_slot, other, data)`; `PhoneError::Nfc { message }` and `PhoneError::ChipBusy` ("The card is talking to its badge. Try again in a moment.") for SW `6985`/`6982` on a session open while the chip is on I2C.
- The EC key session, per NXP's SE05x APDU specification (AN12413 section on ECKey sessions): `CreateSession(KP_ECKEY_USER)` then `ECKeySessionGetECKAPublicKey`, the host's ephemeral key agreement with the chip's ECKA key, `ECKeySessionInternalAuthenticate` carrying the host's public key and a signature by the slot's key over the chip's challenge, then SCP03 session keys derived from the shared secret and the two challenges. The builder reads the specification for the exact TLV layouts and records them in `nfc.rs`'s doc comment with the section numbers.

- [ ] **Step 1: Write the failing tests.** `scp03.rs` unit tests against GlobalPlatform SCP03 test vectors for the key derivation and C-MAC; `nfc.rs` tests over `FakeNfc`: a tap in no session reads a public object and writes the inbox; `open_slot` with a slot whose key matches signs the challenge and later HMACs succeed; a slot whose key does not match fails to open; an HMAC with the seat key's id is refused; a session open while the fake chip reports busy maps to `ChipBusy`.
- [ ] **Step 2: Run and see them fail.** Run: `cargo test -p cahoots-phone --test nfc`.
- [ ] **Step 3: Implement.** Keep `nfc.rs` free of UI: it takes an `&dyn Nfc` and an `&dyn Signer`. Build APDUs with the `se05x` crate's command structs serialised to a `Vec<u8>` writer (the crate's `DataStream`), wrap them with SCP03 after the session opens, and parse answers with `Se05XResponse::from_response`. Regenerate the bindings and add `NfcTransport.swift`; `Core.swift` passes it to `Phone::open`.
- [ ] **Step 4: Run the phone tests and build the app**

Run: `cargo test -p cahoots-phone` and `scripts/ios-build.sh`
Expected: PASS and `** BUILD SUCCEEDED **`.

- [ ] **Step 5: Commit**

```bash
git add crates/phone ios/Cahoots
git commit -m "phone: the chip over NFC: a tap reads a card's public things, writes its inbox, and opens a slot with the enclave key"
```

### Task 4: Card circles on the phone

**Files:**
- Modify: `crates/phone/src/store.rs` (`CircleRecord` gains `card: Option<CardCircle>` where `CardCircle { badge_name: String, slot: u8, circle_slot: u8, badge_public: Vec<u8>, generation_key: Option<[u8; 32]> }`; `root` and `pair_keys` become empty for a card circle; `create_card_circle`), `crates/phone/src/chat.rs` (`Circle::load` builds `ChatSession::from_generation` for a card circle; `poll` handles `NeedsGeneration` by marking `needs_tap`; `hello` for a card circle asks the tap), `crates/phone/src/lib.rs` (`pair_card_start(name) -> PairPreview { badge_name, circles: Vec<String>, words: Vec<String> }` which taps, writes the inbox and reads the public objects; `pair_card_finish() -> Vec<String>` which taps again, opens the slot, reads the public objects again for the grants, keeps each circle as a card circle, fetches each generation key, publishes the badge's card into each circle's chat; `forget_card()`; `tap_for_keys(circle)` used by the chat screen; `CircleSummary` gains `kind: "phone" | "card" | "mixed"`; `history`, `poll`, `send_chat`, `hello_words` work for card circles; `publish_pending(circle)` posts any revocation or bump found in the public object at a tap)
- Test: `crates/phone/tests/card_circle.rs` (new) using `FakeNfc` over a `SoftChip` that a `FakeBoard` badge has kept a circle in and paired slot 0 for (reuse Task 2's badge test helpers through a dev-dependency on `cahoots-badge`)

**Interfaces:**
- Consumes Tasks 2 and 3, plan 2's `ChatSession::from_generation`, `install_generation`, `NeedsGeneration`, `MessageType::Card`.
- Produces the `Phone` methods above and `ChatMessage.kind == "needs_tap"` sentinel or a `CircleSummary.needs_tap: bool`, builder's choice recorded in `lib.rs`.

- [ ] **Step 1: Write the failing tests.** Pairing end to end against the software badge: `pair_card_start` returns the badge's name and four words equal to the badge's; after the badge accepts, `pair_card_finish` keeps one card circle whose `CircleSummary.kind == "card"`, whose record has no root and no pad file, and whose chat reads a message another member posted; the badge's card appears in `circle_members`; a bump posted by a member makes `poll` report `needs_tap`, and `tap_for_keys` clears it; after the badge unpairs, `tap_for_keys` fails with "This card no longer knows this phone." and `forget_card` removes the card circles; hello words from a tap equal the other member's words.
- [ ] **Step 2: Run and see them fail.** Run: `cargo test -p cahoots-phone --test card_circle`.
- [ ] **Step 3: Implement.** A card circle's `circle.cbor` carries the manifest and the seat (the badge's) so `circle_members` and faces work unchanged; `pad.bin` is not written; `Circle::load` tolerates an empty root when `card` is set.
- [ ] **Step 4: Run the phone tests**

Run: `cargo test -p cahoots-phone`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/phone
git commit -m "phone: card circles: pair by tap and press, chat under a generation key from a tap, hello words by tap"
```

### Task 5: The bench, with the real chip

**Files:**
- Create: `scripts/pair-bench.sh` (flash, keep a circle on the board through the existing bench ceremony, then print the steps for the person: open the app, Pair a card, hold the phone, press both on the badge (the script presses through the bench feature when `--press` is given), and wait for the app's words; the script watches the badge log for the pairing lines and the chip for the slot)
- Modify: whatever Task 3's EC key session needs after meeting the real chip; if it cannot be made to work within this task, switch the slot session to the AES-key fallback as the Global Constraints allow, record the reason in `crates/phone/src/nfc.rs`'s doc and in the spec, and keep the EC key path behind a feature for later
- Test: the bench run, recorded in `ios/README.md` and `docs/status.md`

- [ ] **Step 1: Build and flash** the firmware from this branch and run the existing bench ceremony so the board holds a circle kept with Task 1's policies.
- [ ] **Step 2: Drive the badge side** with `scripts/pair-bench.sh --press` and, since no phone is available to the agent, exercise the phone side with the Rust `nfc.rs` client over a real NFC reader if one is attached to the laptop, else record exactly which steps await a person. The person's steps are the ones the summary at the end of this plan lists.
- [ ] **Step 3: Record** what ran and what awaits the person in `ios/README.md` and `docs/status.md`.
- [ ] **Step 4: Commit**

```bash
git add scripts/pair-bench.sh ios/README.md docs/status.md crates/phone
git commit -m "bench: pairing on the board, with the steps that need a person"
```

### Task 6: The app's screens

**Files:**
- Create: `ios/Cahoots/PairCardView.swift` (name field, "hold the phone to the card", the four words and "press both buttons on your badge, then hold the phone to it again", the circles it brought)
- Modify: `ios/Cahoots/CirclesView.swift` (card circles with the card's face, "Pair a card" when the phone has none), `ios/Cahoots/SettingsView.swift` ("Pair a card", "Forget this card"), `ios/Cahoots/ChatView.swift` ("Tap your card to catch up" replaces the composer when `needs_tap`), `ios/Cahoots/PeopleView.swift` (device kinds), `ios/Cahoots/AppModel.swift`, `ios/Cahoots/Core.swift`
- Test: `ios/CahootsTests` for the view models where the project has them; the UI is checked on the device by the person

- [ ] **Step 1: Implement the screens** to the spec's section 9 and the sentences in Tasks 3 and 4. The pairing screen says in one line what a paired phone can do: "This phone can read and post in this badge's circles and manage their channels until you unpair it on the badge."
- [ ] **Step 2: Build**

Run: `scripts/ios-build.sh`
Expected: `** BUILD SUCCEEDED **`.

- [ ] **Step 3: Commit**

```bash
git add ios
git commit -m "ios: pair a card by tap and press, card circles that tap for their keys"
```

### Task 7: Docs, status and the TestFlight build

**Files:**
- Modify: `docs/status.md`, `docs/plan.md` (E1 row rewritten per the spec's corrections, M4j split, product row 3), `docs/protocol.md` (the device grant detail, the public object, the chip operations), `docs/superpowers/specs/2026-10-31-badge-and-phone-design.md` (any correction Task 5 forced), `ios/README.md` (the person's bench steps)
- The TestFlight upload is done by the controller with `scripts/ios-archive.sh` after the merge, not by this task.

- [ ] **Step 1: Write the docs** in plain sentences, naming what ran on the bench and what the person tests: pair, read a chat by tap, unpair.
- [ ] **Step 2: Commit**

```bash
git add docs ios/README.md
git commit -m "docs: pairing by tap, the device grant and the public object, and what the bench showed"
```

## The person's steps after the TestFlight build

1. Badge on USB, antenna switch on, flashed from this branch, holding a circle made after it.
2. Install the build. Settings, "Pair a card", type a name, hold the phone to the chip.
3. On the badge, open "pair a phone" and press both. Check the four words match the phone's. Press both again.
4. Hold the phone to the chip again. The circle appears as a card circle with the badge's face.
5. Open its chat. Post from the browser or the other phone and read it on the card circle. Tap when the app asks.
6. On the badge, "phones", press both on the phone's name. The next tap says the card no longer knows this phone.
