# Badge home screen and the phone's card pocket: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A badge whose resting screen is a home page with a one-press tap window and a two-level menu, and a phone that fills a pocket of keys and hello words in one tap and needs the badge only when the pocket runs low.

**Architecture:** The badge's `Menu` (which runs the custodian's link while a circle is open) gains a `Home` page as its resting page and an `Actions` page per circle, replacing the five-page ring. The tap window is a deadline on the menu that cuts the chip's power through the board's `tap_begin`/`tap_end` and closes by itself. On the phone, the card record keeps a pocket of hello word MACs per seat and counter, filled at the pairing's second tap and topped up by one `tap_card` that also fetches keys and posts bumps. The app gets one Card screen and one verb, "tap the card".

**Tech Stack:** Rust 1.94 workspace (`cahoots-badge`, `cahoots-phone`, `cahoots-host`, the `rp2350` firmware built from a scratch copy), minicbor records, UniFFI to Swift, SwiftUI iOS 17, the bench scripts.

**Spec:** `docs/superpowers/specs/2026-11-01-badge-and-app-flows-design.md`, which builds on `docs/superpowers/specs/2026-10-31-badge-and-phone-design.md`.

## Global Constraints

- Copy: plain words, no em dashes, no semicolons in anything a person reads, no coined names. The app says "tap the card" for every visit to the badge. The badge says "hold the phone to the chip". Circles are circles, the hardware member is the badge or the card.
- `CARGO_TARGET_DIR=/Users/connerswann/code/cahoots/target`. The firmware crate `crates/rp2350` is outside the workspace and does not build inside a git worktree: copy the tree to a scratch directory and build there with `env -u CARGO_TARGET_DIR cargo build --release --features bench` and without the feature, as `scripts/pair-bench.sh` does.
- Tests first, then the code. Every task commits with a lowercase prefix and a sentence.
- The chip's power is the tap gate: a chip the badge powers is deaf to a phone. `Board::tap_begin` cuts it, `Board::tap_end` restores it, every conversation of the badge's own restores it first (`chip_begin`). Nothing in this plan changes that.
- The e-paper is slow: a screen is redrawn only when something a person sees changes. The tap window's countdown redraws at open, at one minute left, and at close, not every second.
- UniFFI records and errors are the app's contract: new fields are added, none renamed. After changing `crates/phone` the Swift bindings come from `scripts/ios-framework.sh`, which `scripts/ios-build.sh` runs.

## Review Focus

1. The tap window closes by itself while a phone is mid-tap: the phone loses the tag, the app must say the card closed and to press its tap button again, and the badge must be back at "closed to taps". Test in Task 5 (the app's sentence) and Task 1 (the badge's state).
2. A laptop's request arrives while the window is open: the custodian's conversation restores the chip's power, so the window is over and the home screen must say so without a press. Test in Task 1.
3. The pocket runs out in the middle of a check: `next_hello` past the last stored pair must not move the counter and must ask for a tap. Test in Task 3.
4. A chat reset while the pocket holds words: hello words do not depend on the chat generation, so the pocket stays valid. Test in Task 3.
5. The words prompt during pairing must never time out like a note does: the owner may take minutes to compare. Test in Task 2.

---

### Task 1: The home page and the tap window

**Files:**
- Modify: `crates/badge/src/board.rs` (the `Board` trait, after `tap_end`)
- Modify: `crates/badge/src/badge.rs` (`Page`, `Menu`, `enter_menu`, `menu_step`, `show_page`, `Idle::Note`, `chooser_names`, `choose_step`)
- Modify: `crates/badge/src/screens.rs` (new `home`)
- Modify: `crates/badge/src/test_support.rs` (`FakeBoard::tap_open`)
- Modify: `crates/rp2350/src/board.rs` (`PicoBoard::tap_open`)
- Test: `crates/badge/tests/home.rs` (new), `crates/badge/tests/circles.rs` (the boot and list tests)

**Interfaces:**
- Produces: `Board::tap_open(&self) -> bool` (default `false`); `Page::Home`; `Menu { tap_until: Option<u64>, tap_minute_shown: bool, .. }`; `screens::home(circles: &[String], status: &str) -> Screen`; `const TAP_WINDOW_SECS: u64 = 120`; `const NOTE_SECS: u64 = 5`; `Idle::Note { index, until: u64, open_tap: bool }`; the chooser's last entry `BACK: &str = "back"`.
- Consumes: `Board::tap_begin`, `Board::tap_end`, `Board::uptime_secs`, `FakeBoard.uptime`, `FakeBoard.tap_open`.

- [ ] **Step 1: Write the failing tests** in `crates/badge/tests/home.rs`, using the helpers the pairing tests use (`kept_pair`, `press`, `b.board().shows`, `b.board().last`, `b.board().uptime`):

```rust
#[test]
fn the_badge_rests_on_a_home_page_that_lists_its_circles() {
    let (mut b1, _b2) = kept_pair((true, false));
    // Boot ends on home, whatever the circle count.
    assert!(b1.board().shows("closed to taps"), "{:?}", b1.board().last());
    assert!(b1.board().shows(&circle_name(&b1)));
    assert!(b1.board().last().iter().any(|l| l.contains("tap") && l.contains("A+B  menu")));
}

#[test]
fn the_left_button_opens_a_two_minute_tap_window_that_closes_by_itself() {
    let (mut b1, _b2) = kept_pair((true, false));
    press(&mut b1, Press::Left);
    assert!(b1.board().tap_open);
    assert!(b1.board().shows("open for a tap, 2 min"), "{:?}", b1.board().last());
    b1.board().uptime += 61;
    step(&mut b1);
    assert!(b1.board().shows("open for a tap, 1 min"));
    b1.board().uptime += 60;
    step(&mut b1);
    assert!(!b1.board().tap_open);
    assert!(b1.board().shows("closed to taps"));
    // A second press while open closes it at once.
    press(&mut b1, Press::Left);
    assert!(b1.board().tap_open);
    press(&mut b1, Press::Left);
    assert!(!b1.board().tap_open);
}

#[test]
fn a_conversation_of_the_badges_own_ends_the_window_and_home_says_so() {
    let (mut b1, _b2) = kept_pair((true, false));
    press(&mut b1, Press::Left);
    // The board's chip_begin restores the power, as a laptop's request would.
    b1.board().tap_open = false;
    step(&mut b1);
    assert!(b1.board().shows("closed to taps"), "{:?}", b1.board().last());
}

#[test]
fn the_menu_is_one_list_that_ends_with_back_to_home() {
    let (mut b1, _b2) = kept_pair((true, false));
    press(&mut b1, Press::Both);
    assert!(b1.board().shows(&format!("> {}", circle_name(&b1))), "{:?}", b1.board().last());
    assert!(b1.board().shows("  new circle"));
    assert!(b1.board().shows("  pair a phone"));
    assert!(b1.board().shows("  back"));
    to_entry(&mut b1, "back");
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("closed to taps"));
}

#[test]
fn a_note_returns_home_after_five_seconds_without_a_press() {
    let (mut b1, _b2) = kept_pair((true, false));
    press(&mut b1, Press::Both);
    to_entry(&mut b1, "pair a phone");
    press(&mut b1, Press::Both); // the pairing page
    press(&mut b1, Press::Both); // "no phone has tapped yet" is a note on the page: still there
    assert!(b1.board().shows("no phone has tapped yet"));
    press(&mut b1, Press::Left); // back closes the pairing: a plain note is not shown here
    assert!(b1.board().shows("closed to taps") || b1.board().shows("open for a tap"));
}
```

Add `circle_name`, `step` (one idle step with no press) and `to_entry` (press Right until the entry is marked) as helpers at the top of the file, next to the ones copied from `pairing.rs`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p cahoots-badge --test home`
Expected: compile errors for `tap_open` and the helpers, or failures on "closed to taps".

- [ ] **Step 3: `Board::tap_open`**

In `crates/badge/src/board.rs` add `fn tap_open(&self) -> bool { false }` with a doc line "True while `tap_begin` has cut the chip's power and no conversation has restored it." Implement it on `FakeBoard` (`self.tap_open`) and on `PicoBoard` (`self.tap_open`). The host's simulator board keeps the default.

- [ ] **Step 4: The home page**

In `screens.rs` add `pub fn home(circles: &[String], status: &str) -> Screen`: title "badge", up to six circle names as rows in `FONT_8X13` from y 30 (more than six: five rows and "+N more"), `status` centered near the bottom in `FONT_8X13`, footer `Some("tap"), Some("A+B  menu"), Some("menu")`. Every row and the status go into `s.lines`.

In `badge.rs`: add `Page::Home` to `Page` and `Page::ALL` (first), `name()` "home". `Menu` gains `tap_until: Option<u64>` and `tap_minute_shown: bool`. `enter_menu` opens on `Page::Home` (the circle's own pages come in Task 2). `show_page` for `Home` draws `screens::home(&names, status)` where `names` are the circles' names and `status` is `tap_status(board, m)`: `"closed to taps"` when `m.tap_until` is `None`, `"open for a tap, 2 min"` when more than 60 s remain, `"open for a tap, 1 min"` otherwise.

In `menu_step`, before the gesture is read, when `m.tap_until` is `Some(t)`: if `!board.tap_open()` or `board.uptime_secs() >= t`, call `board.tap_end()`, set `m.tap_until = None` and redraw home when the page is `Home`. Else if less than 60 s remain and `!m.tap_minute_shown`, set it and redraw home. Gestures on `Home`: `Left` toggles the window (`tap_begin` and `tap_until = Some(uptime + TAP_WINDOW_SECS)`, `tap_minute_shown = false`, or `tap_end` and `None`), then redraw; `Right` and `Both` return `MenuAction::Choose`.

`chooser_names` appends `BACK` last. `choose_step` on `Both` at `BACK` returns to home: `self.enter_menu()`. `Idle::Note` carries `until: board.uptime_secs() + NOTE_SECS` and `open_tap`; the step returns home (`enter_menu`) on any press or once `uptime_secs() >= until`, and when `open_tap` is set the new menu starts with `tap_until = Some(uptime + TAP_WINDOW_SECS)` (the board's power was already cut by the caller). `note()` takes `open_tap: bool`. Every existing `note(..)` call passes `false` except the two "paired with" notes, which pass `true` (Task 2 wires the pairing flow fully).

The boot path `Idle::Off => enter_menu()` is unchanged, so a badge with one circle now rests on home as well. Update `one_circle_opens_its_menu_without_the_list` in `circles.rs` to expect home.

- [ ] **Step 5: Run the badge tests**

Run: `cargo test -p cahoots-badge`
Expected: all pass, including `home` and the updated `circles` tests.

- [ ] **Step 6: Build the firmware** from a scratch copy, both variants, and run clippy on the host target for `cahoots-badge`.

- [ ] **Step 7: Commit**

```bash
git add crates/badge crates/rp2350/src/board.rs
git commit -m "badge: a home page with a one-press tap window, a menu that ends with back, and notes that return home"
```

---

### Task 2: One page per circle in place of the ring

**Files:**
- Modify: `crates/badge/src/badge.rs` (`Page`, `Menu`, `menu_step`, `show_page`, `choose_step`, `pair_start`, `pair_wait_step`, `pair_ask_step`, the phone page entry)
- Modify: `crates/badge/src/screens.rs` (`menu_page` loses the counter and arrows; `phone_page` words)
- Test: `crates/badge/tests/home.rs`, `crates/badge/tests/pairing.rs`, `crates/badge/tests/circles.rs`, `crates/badge/tests/grants.rs`

**Interfaces:**
- Produces: `Page::Actions`; `Menu.action_index: usize`; `const ACTIONS: [&str; 5] = ["hello words", "key as code", "grants", "forget this circle", "back"]`; `screens::action_page(circle: &str, tier: &str, title: &str, body: &[&str]) -> Screen`.
- Consumes: Task 1's `Page::Home`, `enter_menu`, `Idle::Note { open_tap }`.

- [ ] **Step 1: Write the failing tests**

In `home.rs`:

```rust
#[test]
fn a_circle_is_one_page_of_actions_two_levels_down_and_back_comes_home() {
    let (mut b1, _b2) = kept_pair((true, false));
    press(&mut b1, Press::Both);               // the menu
    press(&mut b1, Press::Both);               // the first circle
    for a in ["hello words", "key as code", "grants", "forget this circle", "back"] {
        assert!(b1.board().shows(a), "{a} in {:?}", b1.board().last());
    }
    press(&mut b1, Press::Both);               // hello words
    assert!(b1.board().shows("hello words"));
    assert!(!b1.board().last().iter().any(|l| l.contains("/5")));
    press(&mut b1, Press::Left);               // back to the actions, not another page
    assert!(b1.board().shows("> hello words"));
    to_entry(&mut b1, "back");
    press(&mut b1, Press::Both);
    assert!(b1.board().shows("closed to taps"));
}
```

In `pairing.rs`, in `pairing_from_the_list_shows_the_name_and_the_six_words`: the words prompt stays for ten minutes (`uptime += 600; step; shows("pair with Conner's phone?")`), after the second `Both` the badge shows "paired with Conner's phone" and five seconds later is home with "open for a tap" and `tap_open`, and the "phone" page no longer sets `tap_open` (home does).

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p cahoots-badge --test home --test pairing`
Expected: failures on the actions list and the page counter.

- [ ] **Step 3: The actions page and the leaf pages**

`Page` gains `Actions` and loses `Circles`, `next()` and `previous()`. `Menu` gains `action_index`. Choosing a circle in the chooser sets `self.current` and enters the menu on `Page::Actions` with `action_index = 0`. `show_page` for `Actions` draws `screens::chooser_titled(&c.params.name, &ACTIONS, m.action_index)`. Gestures on `Actions`: `Left`/`Right` move the caret, `Both` opens `Words`, `Key`, `Grants` (reading the grants first) or `Forget`, and at "back" returns `MenuAction::Choose` so the person lands on the menu list, where "back" again goes home.

On `Words`, `Key`, `Grants` and `Forget`: `Left`/`Right` go back to `Actions` (on `Grants` the caret moves first and the ends go back), `Both` does what it does today. `menu_page` becomes `action_page(circle, tier, title, body)` without the "{page}/{pages}" line and arrows, footer `Some("back"), Some("A+B  choose"), Some("back")`. The `Forget` confirmation and `MenuAction::Forget` are unchanged.

Pairing: `pair_start` keeps `tap_begin` and the pairing page. `pair_wait_step` on `Left`/`Right` closes the pairing and goes home (`enter_menu`), not to the chooser. `pair_ask_step` never times out. After `pair_accept` the two "paired with" notes pass `open_tap: true` (Task 1), so home opens with the window running. The phone page entry no longer calls `tap_begin`. `phone_page`'s two middle lines go back to "reads and posts in" and "this badge's circles" since taps belong to home.

- [ ] **Step 4: Run every badge test**

Run: `cargo test -p cahoots-badge`
Expected: all pass. Fix the tests in `circles.rs` and `grants.rs` that walked the ring ("4/5", "circles" page) to walk the actions page instead.

- [ ] **Step 5: Firmware build and clippy, then commit**

```bash
git add crates/badge
git commit -m "badge: one page of actions per circle, back everywhere, and the words prompt waits"
```

---

### Task 3: The pocket of hello words on the phone

**Files:**
- Modify: `crates/phone/src/store.rs` (`CardCircle`)
- Modify: `crates/phone/src/card.rs` (`card_hello` goes, `pocket_fill` comes, the second tap fills it)
- Modify: `crates/phone/src/lib.rs` (`hello_words`, `next_hello`, `HelloWords`, `PhoneError`)
- Test: `crates/phone/tests/card_circle.rs`

**Interfaces:**
- Produces: `PocketWords { #[n(0)] other: u8, #[n(1)] number: u32, #[n(2)] mine: ByteVec, #[n(3)] theirs: ByteVec }` (32 bytes each); `CardCircle.hello_pocket: Option<Vec<PocketWords>>` at `#[n(7)]`; `const POCKET_WORDS: u32 = 8`; `pub(crate) fn pocket_fill(c: &mut Circle, s: &mut Session<'_, '_>) -> Result<u32, TapError>` (the number of pairs added); `pub(crate) fn pocket_left(c: &Circle, other: u8) -> u32`; `HelloWords.card_left: Option<u32>`; `PhoneError::NeedsTap` with the text "Tap the card first."
- Consumes: `Session::hmac_hello`, `hello_label`, `hello_words`, `card_hello_seat`, `CircleRecord.pair_counters`.

- [ ] **Step 1: Write the failing tests** in `card_circle.rs`, beside `hello_words_from_a_tap_equal_the_other_members`:

```rust
#[test]
fn the_second_tap_fills_a_pocket_of_eight_hellos_per_seat_and_no_tap_follows() {
    // After pair(), with the fake NFC taken away, the words for numbers 0 to 7 are served
    // and equal the other members' (as the existing test computes them), next_hello moves
    // through them, card_left counts down from 8 to 1, and the ninth asks for a tap.
    // ... expect Err(PhoneError::NeedsTap) from next_hello at the end, and the counter still 7.
}

#[test]
fn a_reset_does_not_empty_the_pocket() {
    // Pair, let a root holder reset the chat, poll: hello_words still answer without a tap.
}
```

Write them fully against the test file's helpers (`pair`, `tap`, `device`, the fake NFC's `Arc<Mutex<Option<SoftChip>>>` set to `None` to take the card away).

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p cahoots-phone --test card_circle`
Expected: `NeedsTap` and `card_left` do not exist.

- [ ] **Step 3: Store, fill and serve**

Add `PocketWords` and the `hello_pocket` field (an older record decodes as `None`, treated as empty). `pocket_fill` walks every other seat below `c.params.n`, and for numbers from `pair_counters[other]` to that plus `POCKET_WORDS - 1` not already held asks `hmac_hello` for both speakers' labels (`hello_label(me, n)` and `hello_label(other, n)`), pushes the pair, drops pairs with a number below the counter, and returns how many were added. `pair_card_finish` calls it for every circle kept, inside the same session, after the keys. `hello_words` for a card circle finds the pair at the counter and answers with `card_left: Some(pocket_left(..))`, else `Err(NeedsTap)`. `next_hello` for a card circle needs the pair at counter + 1 in the pocket: if it is there the counter moves and it answers, else `Err(NeedsTap)` with the counter unchanged. `card_hello` is deleted. `HelloWords.card_left` is `None` for a software circle.

- [ ] **Step 4: Run the phone tests**

Run: `cargo test -p cahoots-phone`
Expected: all pass, including the renamed hello test, which no longer taps per hello.

- [ ] **Step 5: Commit**

```bash
git add crates/phone
git commit -m "phone: a card circle keeps a pocket of eight hellos per seat from the pairing tap"
```

---

### Task 4: One tap fills everything, and a status that says what it would do

**Files:**
- Modify: `crates/phone/src/card.rs` (`tap_card`, `card_status`, `tap_for_keys` goes)
- Modify: `crates/phone/src/chat.rs` (`forward_bumps` becomes `pub(crate)`)
- Modify: `crates/phone/src/store.rs` (`PhoneRecord.card_last_tap: Option<u64>` at the next free `#[n]`)
- Modify: `crates/phone/src/lib.rs` (the records below)
- Test: `crates/phone/tests/card_circle.rs`

**Interfaces:**
- Produces: `#[derive(uniffi::Record)] pub struct TapReport { pub keys: u32, pub bumps: u32, pub words: u32 }`; `#[derive(uniffi::Record)] pub struct WordsLeft { pub circle: String, pub seat: u8, pub left: u32 }`; `#[derive(uniffi::Record)] pub struct CardStatus { pub paired: bool, pub badge: String, pub circles: u32, pub keys_wanted: u32, pub bumps_to_post: u32, pub words_low: Vec<WordsLeft>, pub last_tap: Option<u64> }`; `pub fn tap_card(&self) -> Result<TapReport, PhoneError>`; `pub fn card_status(&self) -> Result<CardStatus, PhoneError>`.
- Consumes: Task 3's `pocket_fill`, `pocket_left`; `wanted_generations`, `take_keys`, `forward_bumps`, `read_public`, `hmac_root`, `generation_label`, `slot_key`, `my_public`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn one_tap_fetches_the_waiting_key_posts_the_bumps_and_tops_up_the_pocket() {
    // Pair, use five hellos with one seat, let a root holder reset the chat, poll so the
    // card circle waits for a key (needs_tap). card_status: keys_wanted 1, words_low has
    // that seat with left 3. tap_card: TapReport { keys: 1, bumps: 0, words: 5 }. After it
    // needs_tap is false, card_status says keys_wanted 0 and words_low is empty, last_tap is
    // Some.
}

#[test]
fn a_tap_that_loses_the_card_keeps_nothing_half_done() {
    // The fake NFC fails on the third command: tap_card is Err, needs_tap unchanged, the
    // pocket as before.
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p cahoots-phone --test card_circle`

- [ ] **Step 3: `tap_card` and `card_status`**

`tap_card` opens one tap with the slot key (the keychain read happens before `Tap::begin`, so Face ID comes before the sheet), opens the slot, and for every card circle in turn reads its public object (refusing `NotPaired` as `tap_for_keys` did), fetches a key per wanted generation, and calls `pocket_fill`. Keys and pocket changes are applied to the circles only after the session closed without error. Then every card circle's `forward_bumps` runs, `card_last_tap` is written, and the report counts keys, bumps posted and pairs added. `card_status` reads every card circle: `keys_wanted` is the count of circles whose `wanted_generations` is not empty, `bumps_to_post` the sum of `forward.len()`, `words_low` every seat with `pocket_left` below 3, `badge` the name the pairing gave (the card record's badge name where it is kept, else "your badge"), `circles` the count. `tap_for_keys` is removed.

- [ ] **Step 4: Run the phone tests and clippy**

Run: `cargo test -p cahoots-phone && cargo clippy -p cahoots-phone`

- [ ] **Step 5: Commit**

```bash
git add crates/phone
git commit -m "phone: tap_card fetches keys, posts bumps and tops up the pocket in one tap, and card_status says what a tap would do"
```

---

### Task 5: The app's Card screen and one verb

**Files:**
- Create: `ios/Cahoots/CardView.swift`
- Modify: `ios/Cahoots/AppModel.swift` (`Route.card`, `tapCard`, `cardStatus`), `ios/Cahoots/SettingsView.swift` (the Card section), `ios/Cahoots/ChatView.swift` (the banner), `ios/Cahoots/PeopleView.swift` (`HelloView`), `ios/Cahoots/CirclesView.swift` (`meta`), `ios/Cahoots/ContentView.swift` or wherever `Route` is switched
- Test: `ios/CahootsTests/CardViewTests.swift` (new), the build

**Interfaces:**
- Consumes: `tapCard() -> TapReport`, `cardStatus() -> CardStatus`, `HelloWords.cardLeft`, `PhoneError.NeedsTap`.

- [ ] **Step 1: Regenerate the bindings** with `scripts/ios-framework.sh` and confirm `TapReport`, `CardStatus` and `NeedsTap` appear in the generated Swift.

- [ ] **Step 2: Write the failing test** `CardViewTests.swift`: a pure function `CardView.summary(_ s: CardStatus) -> [String]` gives `["up to date"]` for an empty status, and `["2 circles need a key", "1 bump to post", "hello words: 2 left for Ada"]` for a status with those counts and a name lookup closure; `CardView.result(_ r: TapReport) -> String` gives "Fetched 2 keys, posted 1 bump and 16 hello words." and "Nothing was waiting." for zeros.

- [ ] **Step 3: Build the Card screen**

`CardView`: the badge's name, "last tap: when" as a relative date or "never", the summary lines, one filled button "Tap the card" (busy state while tapping, then the result line), "Pair a card" when `paired` is false, and "Forget this card" with its confirmation moved here from Settings. `Route.card` opens it. In Settings the Card section becomes one row, "Card", with the first summary line as its subtitle, plus "Pair a card" when nothing is paired. In `ChatView` the banner reads "Tap the card" and calls `tapCard` then refreshes. In `HelloView`, a `NeedsTap` error shows a "Tap the card" button that taps and reloads, and when `cardLeft` is 3 or fewer a line "N hellos left from your card. Tap the card soon." In `CirclesView.meta`, "Tap your card to catch up." becomes "Tap the card." Every NFC error sentence that says the card moved away adds "Press tap on the badge and try again." in the Card screen only.

- [ ] **Step 4: Build and test**

Run: `scripts/ios-build.sh test`
Expected: `** TEST SUCCEEDED **`. Then `grep -rn "Tap your card\|catch up" ios/Cahoots` returns nothing.

- [ ] **Step 5: Commit**

```bash
git add ios
git commit -m "ios: one Card screen with one button, and the app says tap the card everywhere"
```

---

### Task 6: No empty id for a stranger to take

**Files:**
- Modify: `crates/badge/src/chip.rs` (`first_boot`, `remake_antenna`, `ANTENNA_IDS` or a new `SLOT_IDS`)
- Modify: `crates/chip/src/ops.rs` if a placeholder policy is missing
- Test: `crates/badge/tests/chip_seat.rs` or `crates/badge/tests/upgrade.rs`

**Interfaces:**
- Consumes: the chip-pairing branch's placeholder rule (a placeholder the badge owns, never adopted, deleted and remade when a circle takes the slot), `KeyId::new(kind, slot)` for `Public` and `Hello` ids of circle slots 0 to 15.
- Produces: `pub const CIRCLE_SLOTS: u8` (16, or 8 if the measurement in Step 4 says so).

- [ ] **Step 1: Write the failing test**: after `first_boot` on the software chip, every `Public` id of slots 0 to `CIRCLE_SLOTS - 1` and every `Hello` id of those slots holds an object no session may write or delete, a planted object at any of them before first boot refuses the chip like the other antenna ids, and keeping a circle in a slot deletes that slot's placeholders and makes the circle's objects.

- [ ] **Step 2: Run it to see it fail**: `cargo test -p cahoots-badge --test chip_seat`

- [ ] **Step 3: Make the placeholders** in `first_boot` and `remake_antenna`, under the policy the other placeholders use, and delete a slot's placeholders in the keep path before its objects are made (the slot's ids were empty before only because nothing had been there).

- [ ] **Step 4: Measure on the bench.** Flash the bench build, let the badge make its placeholders, then run `scripts/pair-bench.sh --keep` repeatedly or the bench's keep loop until the chip refuses a circle, and record how many fit. Six fit before this task. If fewer than five fit, set `CIRCLE_SLOTS` to 8 and measure again. Write the numbers into the task report.

- [ ] **Step 5: Run the badge and chip tests, build the firmware, commit**

```bash
git add crates/badge crates/chip
git commit -m "badge: a placeholder at every circle slot's ids from first boot, so a stranger's tap finds no empty id"
```

---

### Task 7: The bench, the person's steps and the record

**Files:**
- Modify: `crates/host/src/chip.rs` (`goto`), `scripts/pair-bench.sh` if its expectations name the chooser
- Modify: `ios/README.md`, `docs/status.md`, `docs/plan.md` (the E1 row), `docs/superpowers/specs/2026-11-01-badge-and-app-flows-design.md` (an "as built" note where a call changed)

- [ ] **Step 1: The bench walks the new screens.** `goto` reaches "pair a phone" from home: on the title "badge" it presses `Both` for the list, then moves to the entry as before. Run `scripts/pair-bench.sh` on the board (`/dev/cu.usbmodempico21`) and quote its last line, "pair bench: ok".

- [ ] **Step 2: The person's steps.** `ios/README.md`: the badge rests on home, the left button opens a two minute tap window, pairing from the menu, the Card screen and its one button, hello words without a badge until the pocket runs low, and that no page needs opening for a tap. `docs/status.md` and the E1 row say what ran on the bench and on the phone. The spec gets a short "as built" paragraph under section 11 for anything that differs.

- [ ] **Step 3: Copy check.** `grep -rn "—\|;" ios/README.md docs/status.md` shows nothing new in the lines touched.

- [ ] **Step 4: Commit**

```bash
git add crates/host scripts ios/README.md docs
git commit -m "docs: the home page, the tap window and the Card screen, as the bench and the phone ran them"
```
