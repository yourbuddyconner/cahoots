# Chat Generations, Cards and Faces Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The chat's keys come in generations so a device without the root can chat and be revoked, and every member has one name and one face carried by a signed card that every screen shows.

**Architecture:** A generation key derives from the root and a random generation id; epoch keys derive from the generation key, and everything below stays as it is. A bump is a signed control message in the chat that names the next id. The contact card gains a face seed and a device kind and travels inside the chat as a signed message, so the relay carries it unchanged. The phone, the browser and the vectors move together with a chat message version and state version bump.

**Tech Stack:** Rust 1.94 workspace (`crates/core`, `crates/chat`, `crates/objects`, `crates/phone`, `crates/wasm`, `crates/vectors`), SwiftUI app in `ios/`, browser app in `web/app`.

**Spec:** `docs/superpowers/specs/2026-10-31-badge-and-phone-design.md`, sections 6 (chat by generation) and 7 (cards and faces). Plan 2 of 3. A ruling made while planning: cards travel as chat messages, not as station objects or sealed envelopes, because the live relay cannot be redeployed from this session and already carries chat messages unchanged. Section 7's "published on the circle's topic as a station envelope" reads as "published in the circle's chat as a card message" from here on, and Task 6 records that in the spec.

## Global Constraints

- The relay changes nothing. Chat messages keep the wire version byte `2` that the station accepts. The header's own `version` field inside the message (today `CHAT_VERSION = 2`, read only by clients) becomes `3` for generation messages.
- Existing circles keep working: a state exported before this change imports as generation 0, whose id is derived, so no member has to do anything at the upgrade.
- Generation 0's id is `SHA-256("cahoots/chat/gen0" || circle_id)`. A generation key is `HMAC-SHA256(root, "chat/gen" || circle_id || generation_id)`. An epoch key is `HMAC-SHA256(generation_key, "chat/epoch" || n as u32 big-endian)`. The epoch id in the header is 5 bytes: slice byte `0xFE`, then a 2-byte tag that is the first two bytes of `SHA-256(generation_id)`, then `n` as u16 little-endian. Root epochs (`0xFF || n u32`) stay decodable for old history and are never produced again.
- A bump's payload is `generation_id (32) || COSE_Sign1` where the signature is a detached seat signature over `"cahoots/chat/bump/v1" || circle_id || generation_id` with kid `[seat]`, verified against the manifest. A device signs with its device key under a chain (plan 3); this plan verifies seat signatures only and leaves a hook for chains.
- A card message's payload is the seat-signed card bytes (`crates/objects/src/card.rs` COSE form), verified against the manifest on receipt. A card without a face seed is still a valid card.
- Device kinds in a card: `0` phone, `1` badge, `2` browser, `3` laptop.
- Faces: 24 by 24 one-bit, drawn by `cahoots_core::faces::pixels(seed)`. A device's face seed is 32 random bytes drawn once and kept.
- Copy is plain words: no em dashes, no semicolons in user-facing text, no coined names. Faces are "faces", the pixel portrait is a "cahoot" only in the faces doc, a member's record is a "card".
- Commit per task, lowercase area prefix and a sentence.

## Review Focus

- A member on the new build reading a circle's old history sees it: root-epoch messages still open. Test in Task 1 (an exported v5 state with root-epoch history imports and reads).
- A bump signed by a key that is not a seat of the manifest is ignored, and the chat continues in the old generation. Test in Task 1.
- A card claiming a seat other than the signer's is refused. Test in Task 4.
- A circle whose member has not published a card still shows a face and a name: the interim face from the seat key and the local seat name. Test in Task 4 (`circle_members` without cards).
- The browser and the phone show the same name and face for one member after that member changes their name. Checked in Task 5's end-to-end run and Task 6's two-simulator walk.

---

### Task 1: Generations in the chat crate

**Files:**
- Modify: `crates/core/src/root.rs` (add `chat_gen_key`, `gen0_id`)
- Modify: `crates/chat/src/primitives.rs` (`CHAT_VERSION`, `EpochId`, `epoch_key`, `MessageType::Bump = 2`, `MessageType::Card = 3`)
- Modify: `crates/chat/src/session.rs` (generation state, `bump`, `from_generation`, `ChatError::NeedsGeneration`, state version)
- Create: `crates/chat/src/bump.rs` (`sign_bump`, `verify_bump`)
- Modify: `crates/vectors/src/main.rs` and `crates/vectors/chat.json`, `crates/vectors/root.json`
- Modify: `docs/chat.md`
- Test: `crates/chat/tests/` (the crate's existing test files; add `generations.rs`)

**Interfaces:**
- Produces in `cahoots_core::root`: `pub fn gen0_id(circle_id: &CircleId) -> [u8; 32]`; `pub fn chat_gen_key(root: &[u8; 32], circle_id: &CircleId, generation_id: &[u8; 32]) -> [u8; 32]`. `chat_key` stays for old history.
- Produces in `cahoots_chat::primitives`: `EpochId::generation(tag: u16, n: u16) -> EpochId` and `EpochId::generation_parts(&self) -> Option<(u16, u16)>`; `pub fn generation_tag(generation_id: &[u8; 32]) -> u16`; `pub fn epoch_key_in(generation_key: &[u8; 32], n: u16) -> Key`; `MessageType::{Text = 0, Control = 1, Bump = 2, Card = 3}`; `CHAT_VERSION = 3`.
- Produces in `cahoots_chat::session`: `ChatSession::new(root, circle_id, members, my_index)` starts in generation 0; `ChatSession::from_generation(generation_id: [u8; 32], generation_key: [u8; 32], circle_id, members, my_index)` for a device with no root; `pub fn generation_id(&self) -> [u8; 32]`; `pub fn bump(&mut self, generation_id: [u8; 32]) -> Result<(), ChatError>` moves to the named generation and starts its epoch 1 (derives the key when the root is held, else `Err(ChatError::NeedsGeneration(generation_id))`); `pub fn install_generation(&mut self, generation_id, generation_key)` for a device that fetched the key; `pub fn send_bump(&mut self, generation_id, signature: &[u8]) -> Result<Vec<u8>, ChatError>` sends a `Bump` message under the current epoch; `receive` returns `(Header, Vec<u8>)` as today, with `header.msg_type == Bump` and the payload for the caller to verify, then the caller calls `bump`. A message whose epoch names an unknown generation returns `Err(ChatError::NeedsGeneration(_))` only when the tag matches no held generation; the session keeps the last two generations' keys so stragglers open.
- Produces in `cahoots_chat::bump`: `pub fn sign_bump<S: SeatSigner>(signer: S, seat: u8, circle_id: &CircleId, generation_id: &[u8; 32]) -> Result<Vec<u8>, ChatError>` returning the payload `generation_id || COSE_Sign1`; `pub fn verify_bump(payload: &[u8], circle_id: &CircleId, manifest: &Manifest) -> Result<([u8; 32], u8), ChatError>` returning the id and the signing seat.
- State: `STATE_MAGIC` version byte goes from 5 to 6; `import_state` of a 5 reads as generation 0 with the root held.

- [ ] **Step 1: Write the failing tests** in `crates/chat/tests/generations.rs`:

```rust
#[test] fn three_sessions_chat_in_generation_zero_and_rotate_for_free() { /* new(..) x3, send/receive, start_epoch twice, all open; current_epoch().generation_parts() == Some((tag(gen0), 3)) */ }
#[test] fn a_bump_moves_every_root_holder_and_locks_out_a_device_without_the_key() { /* a: new; b: new; d: from_generation(gen0, key0); a.send_bump(id1, sig) ; b.receive -> Bump; b.bump(id1); d.receive -> Bump; d.bump(id1) == Err(NeedsGeneration(id1)); a.send text -> b opens, d.receive == Err(NeedsGeneration); d.install_generation(id1, key1); d opens */ }
#[test] fn a_bump_from_a_key_that_is_not_a_seat_is_refused() { /* verify_bump with a signature by a random key -> Err; manifest from a test ceremony */ }
#[test] fn a_v5_state_with_root_epoch_history_imports_as_generation_zero() { /* build with the pre-change derivation using chat_key and EpochId::root, export v5 bytes (fixture in tests/fixtures/chat-v5.bin made before the change, see Step 2), import, read a stored root-epoch message, then send in generation 0 */ }
#[test] fn the_last_two_generations_stay_open() { /* after bump, a straggler message from the old generation still opens; after a second bump, one from two back is NeedsGeneration */ }
```

- [ ] **Step 2: Make the v5 fixture before changing the crate.** Write a small test (or a one-off in the vectors binary) at the current commit that builds a session, sends two root-epoch messages, exports the state and the wires, and writes them to `crates/chat/tests/fixtures/chat-v5.bin` and `chat-v5-wires.bin`. Commit the fixtures with the tests from Step 1 (which fail to compile until Step 3).

- [ ] **Step 3: Run the tests and see them fail**

Run: `cargo test -p cahoots-chat --test generations`
Expected: FAIL, the new names do not exist.

- [ ] **Step 4: Implement** the interfaces above. `EpochId` for generations: slice `0xFE`, `offset` packs tag in the high 16 bits and `n` in the low 16; `encode`/`decode` unchanged in shape. The per-sender chain derivation is unchanged below the epoch key. `bump` keeps the previous generation's epoch states so `receive` can still open them, and drops anything older. Update `docs/chat.md` sections 1, 3 and 8 and the message type table.

- [ ] **Step 5: Regenerate the vectors.** In `crates/vectors/src/main.rs` add to `root.json` the gen0 id and a generation key for the test circle, and to `chat.json` a generation-epoch message set and a bump vector; keep the existing root-epoch cases marked as the old form. Run `cargo run -p cahoots-vectors` as the crate's README says, then `cargo test --workspace`.

- [ ] **Step 6: Run the chat and core tests**

Run: `cargo test -p cahoots-chat -p cahoots-core -p cahoots-vectors`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/root.rs crates/chat crates/vectors docs/chat.md
git commit -m "chat: generations, a random id each, with a signed bump and the last two kept open"
```

### Task 2: Generations in the browser

**Files:**
- Modify: `crates/wasm/src/lib.rs` (`Circle` chat methods, `export`/`import` state version, new `bump_generation`, bump handling in `chat_receive`)
- Modify: `web/app/app.js` (receive path handles a `Bump` result, settings gains "Reset keys for everyone", state version)
- Test: `crates/wasm/tests/`, `web/e2e.mjs`

**Interfaces:**
- Consumes Task 1.
- Produces: `Circle::bump_generation(&mut self) -> Result<String, String>` (signs with the browser's seat key, sends, moves) and `ChatReceived { kind: "text" | "control" | "bump" | "card", ... }` with `bump` already applied when the signature verifies against the circle's manifest and ignored otherwise (the result says `ignored: true`). `Circle::import` reads state version 5 and 6.

- [ ] **Step 1: Write the failing wasm test**: two `Circle`s from one ceremony chat, one bumps, the other receives a `bump` result and then opens a later text; a bump payload with a bad signature yields `ignored: true` and the chat continues.
- [ ] **Step 2: Run and see it fail.** Run: `cargo test -p cahoots-wasm`.
- [ ] **Step 3: Implement** in the wasm crate and `app.js`. The settings sheet's existing "Renew encryption keys" stays as the epoch rotation; add "Reset keys for everyone" beneath it with the footnote "Every member moves to new keys. A device that cannot reach its card reads nothing new until it taps." (plan 3 gives that sentence its meaning; keep it).
- [ ] **Step 4: Run the wasm tests, build the wasm, run the browser check**

Run: `cargo test -p cahoots-wasm`, then `cd crates/wasm && wasm-pack build --target web --release --out-dir ../../web/app/pkg --no-typescript`, then `cargo build -p cahoots-station && node web/e2e.mjs`
Expected: PASS, and the e2e ends "e2e ok".

- [ ] **Step 5: Commit**

```bash
git add crates/wasm web/app/app.js web/e2e.mjs
git commit -m "web: chat generations, with a reset for everyone in the circle's settings"
```

### Task 3: Generations on the phone

**Files:**
- Modify: `crates/phone/src/chat.rs` (session construction, bump on receive, `bump_generation`), `crates/phone/src/lib.rs` (`Phone::bump_chat_generation(circle: String)`), `crates/phone/src/store.rs` (chat state version)
- Modify: `ios/Cahoots/ChatView.swift` or the circle settings screen (a "Reset keys for everyone" action with the same footnote as the web)
- Test: `crates/phone/tests/phone.rs`

**Interfaces:**
- Consumes Task 1.
- Produces: `Phone::bump_chat_generation(circle: String) -> Result<(), PhoneError>`; `ChatMessage.kind` gains `"bump"` entries shown in the transcript as "Keys were reset by seat N." in the app's status style.

- [ ] **Step 1: Write the failing tests**: two phones chat; one bumps; the other reads the next message; a phone with a chat state saved before the change (fixture from Task 1's v5 bytes placed in a circle directory by the test) opens and reads old history.
- [ ] **Step 2: Run and see them fail.** Run: `cargo test -p cahoots-phone --test phone`.
- [ ] **Step 3: Implement**, regenerate bindings with `scripts/ios-framework.sh`, add the action to the app.
- [ ] **Step 4: Run the phone tests and build the app**

Run: `cargo test -p cahoots-phone` and `scripts/ios-build.sh`
Expected: PASS and `** BUILD SUCCEEDED **`.

- [ ] **Step 5: Commit**

```bash
git add crates/phone ios/Cahoots
git commit -m "phone: chat generations, with a reset for everyone"
```

### Task 4: The card carries a face and a device, and travels in the chat

**Files:**
- Modify: `crates/objects/src/card.rs` (fields `face: Option<[u8; 32]>` key 6, `device: Option<u8>` key 7; encode, decode, vectors)
- Modify: `crates/phone/src/store.rs` (device record: `name: String`, `face_seed: [u8; 32]`; per-circle `cards.cbor` map seat to signed card bytes; per-circle optional `face_override: [u8; 32]`)
- Modify: `crates/phone/src/chat.rs` (on receive of `Card`: verify against the manifest, refuse a card whose seat is not the signer's kid, store; `publish_card`)
- Modify: `crates/phone/src/lib.rs` (`my_name`, `set_my_name`, `my_face_seed` from the stored seed, `redraw_face`, `circle_members` and `seat_face_seed` prefer cards, `Member` gains `has_card: bool`, `device: Option<u8>`; publish the phone's card when a ceremony completes and after `set_my_name` or `redraw_face`; `set_circle_face(circle, fresh: bool)`)
- Modify: `crates/vectors/src/main.rs` (objects.json card cases gain face and device)
- Modify: `docs/protocol.md` (the card's fields)
- Test: `crates/objects/tests/`, `crates/phone/tests/phone.rs`

**Interfaces:**
- Consumes Task 1's `MessageType::Card` and Task 3's phone session.
- Produces: `Card { circle_id, seat, name, handles, endpoints, log, face: Option<[u8; 32]>, device: Option<u8> }`; `Phone::my_name() -> String` (default: the empty string, and the app asks for one on the welcome screen and in Settings); `Phone::set_my_name(name: String) -> Result<(), PhoneError>` (1 to 64 bytes); `Phone::redraw_face() -> Result<(), PhoneError>`; `Phone::set_circle_face(circle: String, fresh: bool) -> Result<(), PhoneError>`; `Member { seat, name, face_seed, has_card, device, tier, ... }` where `name` is the card's name when a card exists, else the local seat name, else "Seat N".

- [ ] **Step 1: Write the failing tests.** Objects: a card with face and device round-trips and verifies; the old vector (no face) still decodes; `verify` refuses a card whose `seat` differs from the signer's kid. Phone: after a ceremony both phones publish cards and each sees the other's name and face in `circle_members`; `set_my_name` republishes and the other side updates; without cards `circle_members` falls back to the interim face and the local name; `set_circle_face(circle, true)` changes that circle's face only.
- [ ] **Step 2: Run and see them fail.** Run: `cargo test -p cahoots-objects -p cahoots-phone`.
- [ ] **Step 3: Implement.** The phone's device face seed is drawn with the system RNG on first `open` and stored under protection until first unlock. Cards are published through the chat session as `MessageType::Card` with the signed card as payload, by `Circle::publish_card(http)` called from the places named above. Received cards replace an older card for the same seat only when their `log` field is not lower (use the relay sequence the card was read at, stored beside it).
- [ ] **Step 4: Regenerate the objects vectors and run the tests**

Run: `cargo run -p cahoots-vectors` then `cargo test -p cahoots-objects -p cahoots-phone -p cahoots-vectors`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/objects crates/phone crates/vectors docs/protocol.md
git commit -m "cards: a face and a device on every card, published in the chat and shown for every seat"
```

### Task 5: Names and faces on the phone's screens

**Files:**
- Modify: `ios/Cahoots/WelcomeView.swift` (a name field on first visit), `ios/Cahoots/SettingsView.swift` ("Your name" field, "Draw a new face" button beside the face), `ios/Cahoots/PeopleView.swift`, `ios/Cahoots/ChatView.swift`, `ios/Cahoots/CirclesView.swift` (card names and faces through `Member`), the circle settings (a "New face for this circle" toggle)
- Modify: `ios/Cahoots/AppModel.swift` (`seatName` prefers the card's name)
- Test: `ios/CahootsTests` and `ios/CahootsUITests/RelayWalkUITests.swift` (the walk sets a name on each side and asserts the other side's People screen shows it)

**Interfaces:**
- Consumes Task 4's `Member`, `my_name`, `set_my_name`, `redraw_face`, `set_circle_face`.

- [ ] **Step 1: Extend the walk test**: `testHost` and `testJoiner` each set a name in Settings before the ceremony ("Ada" and "Grace"), and after the chat each asserts the other's name appears in People and in the chat header. Run `scripts/ios-walk.sh` and see it fail on the missing field.
- [ ] **Step 2: Implement the screens.** The welcome screen asks "What should people call you?" with the device's name as the placeholder and lets it be skipped. Settings shows the field under the face with "People in your circles see this name and this face." The face row gains "Draw a new face". People rows show the card's face and name, and the device kind in words ("phone", "badge", "browser", "laptop") after the tier. A member without a card shows the interim face and "Seat N" as today.
- [ ] **Step 3: Build and run the walk**

Run: `scripts/ios-build.sh` then `scripts/ios-walk.sh`
Expected: `** BUILD SUCCEEDED **` and both UI tests pass with the names asserted.

- [ ] **Step 4: Commit**

```bash
git add ios
git commit -m "ios: your name and your face on every circle, and the cards people publish on theirs"
```

### Task 6: Names and faces in the browser, and the docs

**Files:**
- Modify: `crates/wasm/src/lib.rs` (`face_pixels(seed)`, `Circle::publish_card(name, face, device)`, `my_face_seed` storage helpers as the app needs, card handling in `chat_receive` as a `card` result with the seat and the decoded fields)
- Modify: `web/app/app.js` (a face canvas component drawing 24 by 24 from pixels; "Your name" and "Draw a new face" in settings; rosters, the chat header and the dashboard members show card names and faces; the browser publishes its card after a ceremony and on a name change; device kind 2)
- Modify: `web/e2e.mjs` (both contexts set names and assert the other's name on the people screen)
- Modify: `docs/superpowers/specs/2026-10-31-badge-and-phone-design.md` section 7 (cards travel in the chat, the ruling above), `docs/cahoots.md` ("What works today"), `docs/status.md`, `docs/plan.md` (M4i's face in the card; the chat row)
- Test: `crates/wasm/tests/`, `web/e2e.mjs`

**Interfaces:**
- Consumes Tasks 1, 2 and 4.

- [ ] **Step 1: Write the failing wasm test**: a `Circle` publishes a card, the other receives a `card` result with the name and face, and `face_pixels` returns 576 bytes for any seed.
- [ ] **Step 2: Run and see it fail.** Run: `cargo test -p cahoots-wasm`.
- [ ] **Step 3: Implement** in the wasm crate and `app.js`. The face canvas replaces `circleMark` wherever a member is shown; `circleMark` stays for circles and channels.
- [ ] **Step 4: Update the docs** named above in plain sentences.
- [ ] **Step 5: Run the wasm tests, build the wasm, run the browser check**

Run: `cargo test -p cahoots-wasm`, the wasm-pack build, then `cargo build -p cahoots-station && node web/e2e.mjs`
Expected: PASS and "e2e ok" with the name assertions.

- [ ] **Step 6: Commit**

```bash
git add crates/wasm web docs
git commit -m "web: names and faces from cards, and the docs say cards travel in the chat"
```
