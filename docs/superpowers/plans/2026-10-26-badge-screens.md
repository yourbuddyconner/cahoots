# Badge screens: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Two-button hardware-wallet controls and framebuffer screens drawn by the badge crate, identical on the simulator and the Nano.

**Architecture:** `cahoots_hal::Frame` and a one-method `Display`; the badge crate gains a `screens` module rendering with `embedded-graphics`; `Board::press()` replaces the two button methods; the menu becomes a page ring with a chord to select. Boards blit frames and detect the chord.

**Tech Stack:** `embedded-graphics` 0.8 (no_std) in the badge crate; existing crates.

**Spec:** `docs/superpowers/specs/2026-10-26-badge-screens.md`

## Global Constraints

- `cahoots-hal` and `cahoots-badge` stay `#![no_std]`, `#![forbid(unsafe_code)]`, and build for `thumbv7em-none-eabi` with `--no-default-features`.
- Names exactly: `cahoots_hal::Frame { pub bits: [u8; 5000] }` with `Frame::WIDTH = 200`, `Frame::HEIGHT = 200`, `Frame::ROW_BYTES = 25`, `Frame::blank()`, `Frame::get(x, y) -> bool`, `Frame::set(x, y, black: bool)`; `Display::draw(&mut self, frame: &Frame)`; `cahoots_badge::Press { None, Left, Right, Both }`; `Board::press(&mut self) -> Press`; `Board::note_screen(&mut self, lines: &[String])` with a default empty body; `cahoots_badge::screens::Screen { lines, frame }`.
- The chord window is 80 ms; the debounce is 30 ms.
- Screen text in the plain voice; no em dashes anywhere.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-features --all-targets -- -D warnings`, `cargo test --workspace --all-features`, the no_std build, and `cargo build --release` in `crates/esp32s3` (with `source ~/.cargo/esp-env.sh`) pass after each task.

## Review Focus

1. A Left or Right press must not also count as Both when the second button arrives after the window; and Both must not also deliver a Left or Right. Test in Task 3 (fake) and by reading in Task 5 (Nano).
2. The consent prompt is answered only by a gesture seen on a call after the prompt was drawn. Test in Task 3.
3. A frame drawn by the badge for the words screen has the six words at the same pixel positions on the fake, the host and the Nano, because one renderer draws it. Test in Task 2 (renderer) and Task 4 (host PNG matches the frame).
4. The ring driver's progress still reaches the screen during the ceremony. Test in Task 1 (host simulator ceremony still writes status).
5. The self-test screen stays long enough to read and the panic screen waits for Both. Task 5 by reading.

---

### Task 1: `Frame`, one-method `Display`, ring `run` without a display

**Files:**
- Create: `crates/hal/src/frame.rs`
- Modify: `crates/hal/src/lib.rs` (`pub mod frame; pub use frame::Frame;`, the `Display` trait), `crates/hal/src/ring.rs:336-370` (`run(self, status: impl FnMut(&str), idle: impl FnMut())`), `crates/host/src/display.rs` (`PngDisplay` implements `draw`: scale the frame 1:1 into the PNG; keep `new(path, name)`), `crates/host/src/device.rs` (`Device::ceremony` passes a status closure that logs the line and draws a plain text frame through a small `status_frame(lines) -> Frame` helper in `crates/host/src/display.rs` for now), `crates/hal/tests/ring.rs` (the `run` test's display becomes a closure), `crates/badge/tests/common/mod.rs` and `crates/badge/src/badge.rs` only enough to compile (temporary: `draw` records frames, the badge keeps calling nothing new yet; it is rewritten in Task 2)
- Test: `crates/hal/tests/frame.rs`

**Interfaces:**
- Produces: `Frame` as in Global Constraints; `Display::draw`; `RingCeremony::run(self, status, idle)`.

- [ ] **Step 1: Failing tests** in `crates/hal/tests/frame.rs`: `blank_is_white` (every `get` false), `set_and_get_round_trip` (set (3, 7) black, `get(3, 7)` true, `bits[7 * 25 + 0] & 0x10 != 0` for an MSB-first row layout: pixel x is bit `7 - (x % 8)` of byte `x / 8`), `out_of_range_set_is_ignored`.
- [ ] **Step 2: Run, see them fail.** `cargo test -p cahoots-hal --test frame`
- [ ] **Step 3: Implement** `frame.rs`; change `Display`; change `run`; fix every caller so the workspace compiles. The badge's `show_words`/`show_qr`/`show_status` calls become `draw` of frames built by a temporary `text_frame(&[&str]) -> Frame` in the badge crate (Task 2 replaces it).
- [ ] **Step 4: Run** `cargo test --workspace --all-features` and the no_std build. Expected: PASS.
- [ ] **Step 5: Commit** `hal: a 200x200 Frame; Display draws a frame; the ring reports status through a closure`.

---

### Task 2: The badge renders its screens

**Files:**
- Create: `crates/badge/src/screens.rs`
- Modify: `crates/badge/Cargo.toml` (`embedded-graphics = { version = "0.8", default-features = false }`), `crates/badge/src/lib.rs`, `crates/badge/src/badge.rs` (every place that drew: boot, the pile, the ceremony counter, the words, the keep prompt, errors; each builds a `Screen` via `screens::*`, calls `board.note_screen(&screen.lines)` then `board.display().draw(&screen.frame)`), `crates/badge/src/board.rs` (`note_screen` default method)
- Test: `crates/badge/tests/screens.rs`

**Interfaces:**
- Produces: `screens::Screen { pub lines: Vec<String>, pub frame: Frame }` and constructors: `self_test(lines: &[String]) -> Screen`, `panic(msg: &str) -> Screen`, `pile(seat: Option<u8>) -> Screen`, `ceremony(commits: u8, reveals: u8, n: u8) -> Screen`, `words(circle: &str, tier: &str, words: &[&str; 6], note: &str) -> Screen`, `keep_prompt(words: &[&str; 6]) -> Screen`, `menu_page(circle, tier, page: usize, pages: usize, title: &str, body: &[&str], left: bool, right: bool) -> Screen`, `code(bytes: &[u8]) -> Screen`, `consent(len: u32, circle: &str) -> Screen`, `status(lines: &[&str]) -> Screen`. `lines` holds the text the screen shows, top to bottom, so tests and the host can read it.
- Rendering: `embedded_graphics` `DrawTarget` implemented for `Frame` in `screens.rs` (a private wrapper type), fonts `FONT_10X20` for content and `FONT_8X13` for secondary lines, header rule at y 26, arrows `<` at x 2 and `>` at x 190 on the middle row when `left`/`right`, inverted boxes via a filled black rectangle with white text.

- [ ] **Step 1: Failing tests** in `crates/badge/tests/screens.rs`:
  - `words_screen_lists_the_words_in_order`: `words("bench", "TRNG T1", &W, "say them aloud").lines` contains the six words in order and the note.
  - `menu_page_marks_its_neighbours`: `menu_page(.., 2, 3, "key as code", &[..], true, true).frame` has black pixels inside the left arrow box (x 2..10, y 95..105) and the right one; with `left: false` the left box is all white.
  - `code_screen_shows_32_bytes_as_four_rows`: `code(&[0xAB; 32]).lines` has four rows of eight `ab` pairs.
  - `a_frame_is_deterministic`: the same call twice yields equal `bits`.
- [ ] **Step 2: Run, see them fail.**
- [ ] **Step 3: Implement** `screens.rs` and wire the badge to it.
- [ ] **Step 4: Run** the badge tests, workspace tests, the no_std build. Expected: PASS.
- [ ] **Step 5: Commit** `badge: screens rendered into a frame; boards only draw`.

---

### Task 3: `Press`, the page ring, the chord to select

**Files:**
- Modify: `crates/badge/src/board.rs` (`Press`; `press()` replaces `a_pressed`/`b_pressed`; `a_down()` stays for the boot wipe), `crates/badge/src/badge.rs` (the pile step: Both starts the count on the top card; the keep prompt: Both keeps, Left or Right discards; the menu: Left/Right page, Both selects; the Key page: Both spends and shows `code`, any press returns to the Key page; the Serve page: Both starts serving, the consent prompt: Both allows, Left or Right refuses; first gesture after a prompt is drawn is discarded as before), `crates/badge/tests/common/mod.rs` (scripted `VecDeque<Press>`; `note_screen` records lines), `crates/badge/tests/boot.rs`, `ceremony.rs`, `menu.rs` (rewritten for gestures and screen lines)
- Test: the three test files

**Interfaces:**
- Produces: `cahoots_badge::Press`; `Board::press(&mut self) -> Press`.

- [ ] **Step 1: Failing tests** (edit the three files): `both_on_the_key_page_spends_32_bytes_and_shows_the_code`, `left_and_right_walk_the_page_ring` (Words, Key, Serve, Words), `a_single_press_refuses_consent_and_both_allows` (two runs), `a_gesture_before_the_prompt_is_drawn_is_not_an_answer`, `both_keeps_the_circle_and_a_single_press_discards`, plus the existing boot and ceremony tests updated for `press()`.
- [ ] **Step 2: Run, see them fail.** **Step 3: Implement.** **Step 4: Run** badge tests, workspace, no_std.
- [ ] **Step 5: Commit** `badge: two buttons, three gestures; pages and a chord to select`.

---

### Task 4: The host board: gestures from a file, frames to PNG

**Files:**
- Modify: `crates/host/src/board.rs` (`buttons.txt` lines `L`, `R`, `B`; `press()`; `note_screen` writes `screen.txt` with the lines, replacing `status.txt`), `crates/host/src/display.rs` (`PngDisplay::draw` writes the frame 1:1, black on white), `crates/host/tests/badge.rs`, `crates/host/tests/badge_serve.rs` (drive with `L`/`R`/`B` and read `screen.txt`), `crates/host/README.md`
- Test: those two test files

- [ ] **Step 1: Update the tests** to the new files and gestures; run, see them fail.
- [ ] **Step 2: Implement.** **Step 3: Run** `cargo test -p cahoots-host`. Expected: PASS.
- [ ] **Step 4: Commit** `host: gestures from buttons.txt, screens to PNG and screen.txt`.

---

### Task 5: The Nano: chord detection and a frame blit

**Files:**
- Modify: `crates/esp32s3/src/buttons.rs` (`Buttons::press() -> Press` with the 80 ms chord window on top of the 30 ms debounce: a stable Left or Right is held for up to 80 ms waiting for the other; if it arrives, `Both`; otherwise the single press), `crates/esp32s3/src/display.rs` (`Epd::draw(&Frame)` copies the 5000 bytes into `Display1in54`'s buffer, respecting its bit polarity, then one full refresh; the `screen`/`footer`/`show_*` drawing code is deleted), `crates/esp32s3/src/board.rs` (`press`, `note_screen` traces the lines), `crates/esp32s3/src/main.rs`, `crates/esp32s3/README.md`
- Verify: `source ~/.cargo/esp-env.sh && cargo build --release` in `crates/esp32s3`; flashing and the on-board check are the controller's.

- [ ] **Step 1: Baseline build.** **Step 2: Implement.** **Step 3: Build.** **Step 4: Commit** `esp32s3: two gestures and a chord; the panel draws the badge's frame`.

---

### Task 6: Docs

**Files:** `docs/ceremony-guide.md` (the badge section: stack, Both to start, say the words, Both to keep), `docs/diy-badge.md` (the two buttons paragraph), `crates/esp32s3/README.md` (what the board does), `docs/plan.md` (M4e evidence: screens and gestures).

- [ ] **Step 1: Edit** in the plain voice. **Step 2: Commit** `docs: the badge's two-button controls and screens`.
