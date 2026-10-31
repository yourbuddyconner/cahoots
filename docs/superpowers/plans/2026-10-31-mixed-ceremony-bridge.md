# Mixed Ceremony Bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A ceremony hosted on a phone includes badges stacked in a pile on a laptop, with the laptop carrying frames between the pile and the relay and holding no seat.

**Architecture:** The pile's `Seat` frame grows a bridged flag so a laptop can sit at the bottom of a pile without being a card, hand the badges the host's parameters and count, and let them accept the count from below. The offer code carries the number of badges so joining phones skip their seats. A new `cahoots-host bridge` command joins the relay channel from the offer, forwards every ceremony frame both ways, and prints each badge's join code for the host to scan. Nothing in the ceremony's rounds changes.

**Tech Stack:** Rust 1.94 workspace (`crates/core`, `crates/hal`, `crates/host`, `crates/phone`, `crates/wasm`, `crates/relay`), SwiftUI app in `ios/`, the browser app in `web/app`, `MemRelay` for tests.

**Spec:** `docs/superpowers/specs/2026-10-31-badge-and-phone-design.md`, section 8 (the ceremony: roles and the bridge) and the "Seats in a mixed circle" paragraph. This plan is the first of three for that design; chat generations with cards and faces, and the chip with pairing, follow in their own plans.

## Global Constraints

- The ceremony's round payloads do not change: `Params`, `Commit`, `Reveal`, `Attest` keep their bytes on the wire and on the relay. The channel message version stays 3, so the live relay needs no redeploy.
- An unbridged pile is byte-for-byte what it was: every existing test in `crates/hal/tests/pile.rs` and `ring.rs` passes unchanged.
- The `Seat` payload keeps its length. Byte 5, today `agent u8`, becomes a flags byte: bit 0 agent, bit 1 bridged.
- The bridge holds no seat, is not counted, makes no key, and never decrypts a reveal.
- The host is a member at seat 0. Badges are seats 1 to `m`. Phones are seats above `m`.
- Copy is plain words: no em dashes, no semicolons in user-facing text, no coined names. The first board is "V1", the hardware member is a "badge" or a "card", the laptop program is "the bridge".
- iOS 17 floor, XcodeGen project in `ios/project.yml`, UniFFI proc-macros in `crates/phone`; a changed `Phone` method signature means `scripts/ios-framework.sh` regenerates the Swift bindings.
- Commit after every task with a message in the repository's style: a lowercase area prefix and a sentence, for example `pile: a bridged Seat carries the host's parameters and takes the count from below`.

## Review Focus

- A phone on today's build scanning a new offer sees "This code is from a newer version of Cahoots. Update the app." and a new phone scanning an old offer sees the mirror of it. Test in Task 2 (`from_offer` on a v1 prefix) and Task 3 (the phone's error text).
- A host entering more badges than members minus one is refused before any code is shown. Test in Task 3.
- A pile with a different number of badges than the offer says is refused by the bridge with a message naming both numbers, before any frame reaches the relay. Test in Task 5.
- The top badge in a bridged pile pressing both buttons does nothing and the screen says the host starts it. Test in Task 1 (`start()` returns `PileError::Bridged`).
- A relay message for a badge's seat, which only a misbehaving phone could post, is never forwarded into the pile. Test in Task 5.

---

### Task 1: The bridged pile

**Files:**
- Modify: `crates/hal/src/pile.rs`
- Modify: `crates/badge/src/badge.rs:715-745` (the pile state's button handling and status line)
- Test: `crates/hal/tests/pile.rs`

**Interfaces:**
- Produces: `Proposal { name, pad_len, agent, bridged: bool }`; `Ready { params, seat, bridged: bool }`; `PileError::Bridged`; `pub fn bridged_seat_frame(first_seat_below: u8, params: &CircleParams) -> Vec<u8>` and `pub fn count_frame(n: u8) -> Vec<u8>` in `pile.rs`, each a whole encoded frame (`cahoots_core::frame::encode`) of kind 17 and 18 respectively, as a bridge sends them up its link.

- [ ] **Step 1: Write the failing tests** in `crates/hal/tests/pile.rs`, using the file's `stack`, `settle` and `End` helpers and a bare `End` link standing in for the bridge:

```rust
#[test]
fn a_bridged_seat_from_below_sets_the_seat_the_params_and_the_flag() {
    // bridge end sends bridged_seat_frame(0, &params) up to card 0; after settle,
    // card 0 has seat 1, card 1 has seat 2, both proposals have bridged == true,
    // and both cards forwarded a Seat with bit 1 set.
}
#[test]
fn a_bridged_pile_takes_the_count_from_below_and_refuses_the_top_cards_start() {
    // bridge end sends count_frame(4); settle; both cards are Ready { seat: 1 / 2, bridged: true }
    // with params.n == 4; cards[1].start() == Err(PileError::Bridged).
}
#[test]
fn an_unbridged_pile_ignores_a_count_from_below() {
    // today's behaviour: a Count arriving from DOWN on an unbridged card is dropped.
}
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test -p cahoots-hal --test pile`
Expected: FAIL, `bridged_seat_frame` and `PileError::Bridged` not found.

- [ ] **Step 3: Implement in `crates/hal/src/pile.rs`**: decode byte 5 as flags in `handle` for `(DOWN, KIND_SEAT)`; encode flags in `send_seat_up`; accept `(DOWN, KIND_COUNT)` when `self.proposal.bridged` and forward it up; `start()` returns `Err(PileError::Bridged)` when bridged; add `bridged` to `Ready`; add the two frame builders. Update the module doc table for `Seat`.

- [ ] **Step 4: Update the badge** in `crates/badge/src/badge.rs`: in the pile state, when `ready.bridged` the status line reads "waiting for the host to start" and a `PileError::Bridged` from `start()` shows "the host starts this one". Pass `bridged` through wherever `Ready` is matched.

- [ ] **Step 5: Run the hal and badge tests**

Run: `cargo test -p cahoots-hal -p cahoots-badge`
Expected: PASS, including every pre-existing pile and ring test.

- [ ] **Step 6: Commit**

```bash
git add crates/hal/src/pile.rs crates/hal/tests/pile.rs crates/badge/src/badge.rs
git commit -m "pile: a bridged Seat carries the host's parameters, takes the count from below, and the host starts it"
```

### Task 2: The offer carries the badge count

**Files:**
- Modify: `crates/core/src/channel.rs:49-230` (`OFFER_PREFIX`, `Offer`, `offer`, `from_offer`, `ChannelError`)
- Test: `crates/core/tests/channel.rs` (create if absent; the crate's channel tests may live in `channel.rs` under `#[cfg(test)]`, in which case add there)

**Interfaces:**
- Produces: `OFFER_PREFIX = "cahoots:ceremony:v2:"`; `pub const OLD_OFFER_PREFIX = "cahoots:ceremony:v1:"`; `Offer { channel, params, relay, host, badges: u8 }`; `Channel::offer(&self, params, relay, host, badges: u8) -> Result<String, ChannelError>`; `ChannelError::OlderOffer` (a v1 code) and `ChannelError::NewerOffer` (a prefix `cahoots:ceremony:v` with a number above 2); `Offer::is_badge_seat(&self, seat: u8) -> bool` true for `1..=badges`.
- Encoding: `id (16) || key (32) || host (32) || badges (1) || relay_len (1) || relay || params`. `badges` must be below `params.n`, else `Malformed`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn an_offer_round_trips_its_badge_count() { /* badges 2 of n 4; from_offer(...).badges == 2; is_badge_seat(1) && is_badge_seat(2) && !is_badge_seat(3) */ }
#[test] fn a_v1_offer_is_refused_as_older() { /* "cahoots:ceremony:v1:AAAA" -> Err(OlderOffer) */ }
#[test] fn a_v3_offer_is_refused_as_newer() { /* "cahoots:ceremony:v3:AAAA" -> Err(NewerOffer) */ }
#[test] fn a_badge_count_at_or_above_n_is_malformed() { /* offer(params n 3, badges 3) -> Err(Malformed) */ }
```

- [ ] **Step 2: Run and see them fail**

Run: `cargo test -p cahoots-core channel`
Expected: FAIL on the new names.

- [ ] **Step 3: Implement** the encoding and the two prefix errors in `channel.rs`. Keep `CEREMONY_VERSION` at 3. Update the doc comment on `offer`.

- [ ] **Step 4: Fix every caller** of `Channel::offer` (`crates/phone/src/ceremony.rs` around line 472, `crates/wasm/src/lib.rs:292`) to pass `0` for now, and `OFFER_PREFIX` uses in `web/app/app.js:67,539,541` to the v2 text. Run `cargo test --workspace` and `cargo build -p cahoots-wasm --target wasm32-unknown-unknown` (or the project's wasm build script) to confirm.

- [ ] **Step 5: Run the tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/channel.rs crates/core/tests crates/phone/src/ceremony.rs crates/wasm/src/lib.rs web/app/app.js
git commit -m "ceremony: the offer carries the badge count, version 2, and a code from another version says so"
```

### Task 3: The phone hosts with badges and joiners skip their seats

**Files:**
- Modify: `crates/phone/src/lib.rs:545-700` (`start_ceremony`, `join_offer`, `join_scan`, `check_seat`, the `JoinOffer` and `CeremonyState` records)
- Modify: `crates/phone/src/ceremony.rs` (where the offer is built and seats are listed)
- Modify: `ios/Cahoots/NewCircleView.swift`, `ios/Cahoots/JoinView.swift`, `ios/Cahoots/CeremonyView.swift`, `ios/Cahoots/AppModel.swift` (the `startCeremony` call)
- Test: `crates/phone/tests/phone.rs`; `ios/CahootsTests` as the project's pattern allows

**Interfaces:**
- Consumes: Task 2's `Offer.badges`, `is_badge_seat`, `OlderOffer`, `NewerOffer`.
- Produces: `Phone::start_ceremony(name: String, members: u8, badges: u8, pad_len: u32, through_relay: bool) -> Result<String, PhoneError>`; `JoinOffer.badges: u8`; `CeremonyState.badges: u8`; `check_seat(params, badges, seat)` refusing `1..=badges` with "Seats 2 to {badges + 1} are cards in the pile. Pick a seat above them."; `PhoneError::Ceremony` messages "This code is from an older version of Cahoots. Ask the host to update." and "This code is from a newer version of Cahoots. Update the app." for the two prefix errors.
- `start_ceremony` refuses `badges >= members` with "A circle of {members} can have at most {members - 1} cards." and `badges > 0 && !through_relay` with "Cards in a pile join through the relay. Turn the relay on."

- [ ] **Step 1: Write the failing tests** in `crates/phone/tests/phone.rs` with the file's `device` and `relay` helpers:

```rust
#[test] fn a_host_with_two_badges_makes_an_offer_that_says_so() { /* start_ceremony("pile", 4, 2, 16384, true); Channel::from_offer(offer_code).badges == 2; ceremony_state(id).badges == 2 */ }
#[test] fn a_joiner_cannot_take_a_badge_seat() { /* join_offer(code, 1) -> Err whose message contains "cards in the pile"; join_offer(code, 3) -> Ok */ }
#[test] fn too_many_badges_is_refused_before_a_code_exists() { /* start_ceremony("x", 3, 3, ...) -> Err "at most 2 cards" */ }
#[test] fn an_old_offer_is_explained() { /* join_scan("cahoots:ceremony:v1:AAAA") -> Err message "older version" */ }
```

- [ ] **Step 2: Run and see them fail**

Run: `cargo test -p cahoots-phone --test phone`
Expected: FAIL, wrong arity on `start_ceremony`.

- [ ] **Step 3: Implement** in the phone crate. Store `badges` with the pending ceremony so `offer()` and the state carry it. Map the two `ChannelError`s to the messages above wherever offers are parsed (`join_offer`, `join_scan`).

- [ ] **Step 4: Run the phone tests**

Run: `cargo test -p cahoots-phone`
Expected: PASS.

- [ ] **Step 5: Regenerate the bindings and update the app.** Run `scripts/ios-framework.sh`. In `NewCircleView.swift` add a `PeopleStepper` "Cards in the pile" with range `0...(members - 1)`, shown only when the relay toggle is on, with the footnote "Badges stacked on a laptop running the bridge. They take the seats right after yours." In `JoinView.swift` render seats `1...badges` as rows titled "Seat N, a card" that cannot be chosen. In `CeremonyView.swift` the host's seat rows for badge seats read "card, through the bridge" in the waiting state. Pass `badges` through `AppModel.startCeremony`.

- [ ] **Step 6: Build the app**

Run: `scripts/ios-build.sh`
Expected: `** BUILD SUCCEEDED **`.

- [ ] **Step 7: Commit**

```bash
git add crates/phone ios/Cahoots
git commit -m "phone: a host names how many cards are in the pile, and joiners skip their seats"
```

### Task 4: The browser app reads version 2 offers

**Files:**
- Modify: `crates/wasm/src/lib.rs:177-300` (`offer_params`, `from_offer`, `offer`)
- Modify: `web/app/app.js:380-390, 549-580` (offer parsing and the seat picker)
- Test: `crates/wasm/tests` (the crate's existing test pattern), `web/e2e.mjs` run unchanged

**Interfaces:**
- Consumes: Task 2.
- Produces: `offer_params(text)` returns the params bytes followed by one trailing `badges` byte, or the JS reads badges from a new `offer_badges(text) -> Result<u8, String>`; choose the latter. The browser host passes `0` badges. The joiner's seat picker marks seats `1..=badges` as "card" and unselectable, and an old or new offer shows the same two sentences as the phone.

- [ ] **Step 1: Write the failing test** in the wasm crate's tests: `offer_badges` on an offer made with badges 2 returns 2; on a v1 string returns an error containing "older".

- [ ] **Step 2: Run and see it fail**

Run: `cargo test -p cahoots-wasm`
Expected: FAIL, `offer_badges` not found.

- [ ] **Step 3: Implement** `offer_badges` and update `app.js` so `taken` includes badge seats with the label "card", and the error map at `app.js:40` gains the two version messages.

- [ ] **Step 4: Run the wasm tests and the browser end-to-end check**

Run: `cargo test -p cahoots-wasm` and the project's web check (`web/e2e.mjs`, see `web/README.md` for the command)
Expected: PASS, and the e2e run unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/wasm web/app/app.js
git commit -m "web: version 2 offers, with card seats shown as taken"
```

### Task 5: `cahoots-host bridge`

**Files:**
- Create: `crates/host/src/bridge.rs`
- Modify: `crates/host/src/main.rs` (dispatch at the `usage:` line and the command match), `crates/host/src/lib.rs` (export), `crates/host/Cargo.toml` (`cahoots-relay` dependency; `cahoots-phone` and `cahoots-relay` as dev-dependencies)
- Modify: `crates/host/src/pile.rs` (make a simulated card runnable with caller-supplied links: `pub fn spawn_card(down: BoxLink, up: BoxLink, name: &str, dir: PathBuf, seed: Option<u64>) -> JoinHandle<Result<Device, DeviceError>>`, refactored out of `run_pile`)
- Test: `crates/host/tests/bridge.rs`

**Interfaces:**
- Consumes: Task 1's `bridged_seat_frame`, `count_frame`, `PileError::Bridged`; Task 2's `Offer`, `is_badge_seat`, `commitment_hash`, `join_code`, `Channel::seal`, `parts`, `Mailbox::receive`; `cahoots_relay::{Relay, HttpRelay, MemRelay}`; `cahoots_hal::ring::Frame`; `cahoots_core::frame::{encode, decode}`.
- Produces: `pub struct Bridge` with `pub fn new(offer: Offer, badges_in_pile: u8) -> Result<Bridge, BridgeError>` (refuses `badges_in_pile != offer.badges` with `BridgeError::Count { pile, offer }`), `pub fn step(&mut self, link: &mut dyn Link, relay: &mut dyn Relay, on_join_code: &mut dyn FnMut(u8, &str)) -> Result<BridgeStep, BridgeError>` where `BridgeStep` is `Working | Done`, and `pub fn run(...)` looping `step` until `Done`. `BridgeError` variants: `Count`, `HostMismatch` (seat 0's commitment does not hash to the offer), `Params` (a badge's `Params` differs from the offer's), `Link`, `Relay`.
- Behaviour of `step`: when the link's peer appears or a `Hello` arrives, send `bridged_seat_frame(0, &params)` then `count_frame(params.n)`; send one synthesised `Params` frame per non-badge seat (`index || params.encode()`); for each ring frame from the link with a badge index, post it as that seat's channel message (sealed under `channel.seal`, split with `parts` when needed) and, for a `Commit`, call `on_join_code(seat, &join_code(&channel.id, seat, payload))`; drop frames from the link with a non-badge index; for each relay message, `Mailbox::receive` it, drop it if its seat is a badge seat, check seat 0's commitment against `offer.host`, and send it up the link as a ring frame; `Done` when an `Attest` has gone each way for every seat.
- The command: `cahoots-host bridge --serial PORT --offer TEXT [--badges N]` with `--badges` defaulting to the offer's count; `--offer-file PATH` reads the text from a file so a code pasted into a file works too. It prints each join code as a QR in the terminal (`qrcode` crate with its `unicode` renderer, added to `crates/host`) above the text form.

- [ ] **Step 1: Write the failing integration test** `crates/host/tests/bridge.rs`. Copy the `FakeHttp` over `MemRelay` and the software `Signer` from `crates/phone/tests/phone.rs` into `crates/host/tests/common/` (or `tests/bridge.rs` itself). The test:

```rust
#[test]
fn a_phone_hosts_two_cards_through_the_bridge_and_a_phone_joiner() {
    // relay = Arc<Mutex<MemRelay>>; host = Phone over FakeHttp; joiner = Phone over FakeHttp
    // host.start_ceremony("mixed", 4, 2, 16384, true); code = host.offer_code(id)
    // sockets: (bridge_end, card1_down), (card1_up, card2_down); card2_up = Absent
    // spawn_card x2 with tempdirs; Bridge::new(Channel::from_offer(&code)?, 2)
    // joiner.join_offer(code, 3); run bridge.step in a thread against relay (as &mut dyn Relay via a MemRelay adapter)
    // collect join codes from on_join_code; host.ceremony_scan(id, code) for seats 1, 2 and the joiner's join code
    // host.ceremony_continue(id); tick host and joiner every few ms until both ceremony_state(..).complete
    // assert: cards' Device::check_words == host words == joiner words; manifest from any party has 4 seats
}
#[test]
fn the_bridge_refuses_a_pile_that_does_not_match_the_offer() { /* Bridge::new(offer with badges 2, 3) -> Err(Count { pile: 3, offer: 2 }) */ }
#[test]
fn a_relay_message_for_a_badge_seat_never_enters_the_pile() { /* post a sealed Commit for seat 1 to the relay; after step, the link saw no frame with index 1 */ }
#[test]
fn a_host_commitment_that_does_not_match_the_offer_stops_the_bridge() { /* -> Err(HostMismatch) */ }
```

- [ ] **Step 2: Run and see them fail**

Run: `cargo test -p cahoots-host --test bridge`
Expected: FAIL, `cahoots_host::bridge` not found.

- [ ] **Step 3: Refactor `run_pile`** so `spawn_card` exists and `run_pile` calls it. Run `cargo test -p cahoots-host` to confirm nothing changed.

- [ ] **Step 4: Implement `bridge.rs`** to the interface above. The dedupe for relay messages is `Mailbox::receive`; the dedupe for link frames is a `HashSet<(u8 kind, u8 index, [u8; 32] body hash)>`. Posting a payload over `MAX_PART` goes through `parts` and posts each part in order.

- [ ] **Step 5: Add the command** in `main.rs`: parse the arguments, `HttpRelay::new(&offer.relay)`, `SerialLink::open(port, "pile")`, run the bridge, print join codes as QR and text, print "every seat has signed. The badges are keeping their circles." on `Done`. Add `bridge` to the `usage:` line.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p cahoots-host`
Expected: PASS.

- [ ] **Step 7: Document.** In `docs/ceremony-guide.md` add a section "With badges in a pile" that says: stack the badges on the laptop with the bottom one on USB, run the command with the host's code, scan the codes the laptop prints with the host's phone, press continue on the phone. In `docs/plan.md` mark the M4d row: the bridge built, over USB serial in `cahoots-host`, the browser's WebSerial later. Keep each sentence plain.

- [ ] **Step 8: Commit**

```bash
git add crates/host docs/ceremony-guide.md docs/plan.md
git commit -m "host: the bridge carries a pile into a ceremony the phone hosts, and holds no seat"
```

### Task 6: The bench run

**Files:**
- Create: `scripts/bridge-bench.sh`
- Modify: `ios/README.md` ("Tried in the simulator" section), `docs/status.md` (the iOS and hardware rows)

**Interfaces:**
- Consumes: Task 5's command; `scripts/bench.sh`'s port and flashing conventions; `scripts/ios-walk.sh`'s two-simulator setup.

- [ ] **Step 1: Write `scripts/bridge-bench.sh`** following `scripts/bench.sh`: flash the badge (or `--no-flash`), start a local station, boot one simulator as the host with `members 3, badges 1`, read the offer from the simulator's screen through `xcrun simctl` the way `ios-walk.sh` reads codes, run `cahoots-host bridge --serial $PORT --offer "$CODE"`, feed the printed join code and a second simulator's join code to the host through the walk's scanning hook, continue, and wait for all three to complete. Print the six words from each.

- [ ] **Step 2: Run it on the bench** with the Click board badge on `/dev/cu.usbmodempico21`

Run: `scripts/bridge-bench.sh`
Expected: the badge's screen shows "waiting for the host to start", then the rounds, then the six words; the two simulators show the same six words; the script prints "bridge bench: ok".

- [ ] **Step 3: Record it.** `ios/README.md` gains a paragraph under "Tried in the simulator" naming the mixed run. `docs/status.md`'s iOS row says a phone-hosted ceremony with a badge through the bridge has run on the bench.

- [ ] **Step 4: Commit**

```bash
git add scripts/bridge-bench.sh ios/README.md docs/status.md
git commit -m "bench: a phone hosts a badge through the bridge"
```
