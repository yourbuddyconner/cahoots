//! [`Badge`]: the boot screen, the pile and the ceremony, and the idle menu.

use crate::board::{Board, Circle, Press};
use crate::chip::{self, ChipSeat, SlotEntry};
use crate::screens::{self, Screen};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use cahoots_chip::{Curve, KeyId, KeyKind};
use cahoots_core::ceremony::CeremonyResult;
use cahoots_core::seat::SeatPublic;
use cahoots_core::frame::{self, Frame, FrameError};
use cahoots_core::pad::Pad;
use cahoots_core::params::CircleParams;
use cahoots_custodian::wire::{
    ErrorCode, Request, Response, KIND_REQUEST, KIND_RESPONSE, MAX_REQUEST_LEN,
};
use cahoots_custodian::{
    serve, CircleId, CircleInfo, Consent, Custodian, PadError, PskHandle, PSK_LEN,
};
use cahoots_hal::ledger::PersistentLedger;
use cahoots_hal::pile::{Pile, PileError};
use cahoots_hal::ring::{RingCeremony, Step as RingStep};
use cahoots_hal::store::StorePad;
use cahoots_hal::{Display, Entropy, Link};
use zeroize::Zeroizing;

/// What one [`Badge::step`] did, or where the pass stands after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// No pass is under way.
    Idle,
    /// In the pile, waiting for a seat and the count.
    InPile,
    /// The ceremony is running.
    InCeremony,
    /// The ceremony is done and the badge asks: Both keeps the circle. Left or Right asks
    /// again, "discard this circle?", where Both discards and Left or Right goes back.
    Prompt,
    /// The circle is saved. The pass is over.
    Kept,
    /// The result is dropped. The pass is over.
    Discarded,
    /// The pass failed with this message. The next step starts a new pass.
    Failed(String),
}

/// Where the pass is between two calls to [`Badge::step`].
enum State<L: Link> {
    Idle,
    Pile {
        pile: Pile<L>,
        shown: Option<u8>,
    },
    Ceremony {
        /// Boxed: the ceremony is a few kilobytes, and the state moves on every step.
        ring: Box<RingCeremony<L>>,
        /// Commitments, reveals and attestations held when the screen was last drawn.
        last: (u8, u8, u8),
        params: CircleParams,
        seat: u8,
    },
    /// The ceremony is complete. Push the last forwarded frames out before the prompt.
    Flush {
        links: Vec<L>,
        result: CeremonyResult,
        seat: u8,
    },
    Prompt {
        result: CeremonyResult,
        seat: u8,
    },
    /// A single press on the prompt: the words are a circle's whole life, so discarding
    /// takes a second question answered with Both.
    ConfirmDiscard {
        result: CeremonyResult,
        seat: u8,
    },
}

/// The custodian a badge serves: its stored pad and its ledger, reporting the board's tier.
type Custody<B> = Tiered<Pad<StorePad<<B as Board>::Pad>, PersistentLedger<<B as Board>::Records>>>;

/// A custodian that reports `tier` for every circle it holds. Everything else is `inner`'s.
struct Tiered<C> {
    inner: C,
    tier: u8,
}

impl<C: Custodian> Custodian for Tiered<C> {
    fn circles(&self) -> Vec<CircleInfo> {
        let mut circles = self.inner.circles();
        for c in &mut circles {
            c.tier = self.tier;
        }
        circles
    }

    fn psk(
        &mut self,
        circle: &CircleId,
        context: &[u8],
    ) -> Result<(PskHandle, Zeroizing<[u8; 32]>), PadError> {
        self.inner.psk(circle, context)
    }

    fn psk_for(
        &mut self,
        handle: &PskHandle,
        context: &[u8],
    ) -> Result<Zeroizing<[u8; 32]>, PadError> {
        self.inner.psk_for(handle, context)
    }

    fn spend(
        &mut self,
        circle: &CircleId,
        len: u32,
    ) -> Result<(PskHandle, Zeroizing<Vec<u8>>), PadError> {
        self.inner.spend(circle, len)
    }

    fn spend_for(&mut self, handle: &PskHandle, len: u32) -> Result<Zeroizing<Vec<u8>>, PadError> {
        self.inner.spend_for(handle, len)
    }

    fn remaining(&self, circle: &CircleId) -> u64 {
        self.inner.remaining(circle)
    }

    fn low_pad(&self, circle: &CircleId) -> bool {
        self.inner.low_pad(circle)
    }
}

/// How long the self-test screen stays unless Both is pressed, and how often boot polls.
const SELF_TEST_MS: u32 = 2500;
const POLL_MS: u32 = 10;

/// The menu's pages, a ring: Right goes to the next, Left to the previous, and the last
/// page's next is the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Words,
    Key,
    Serve,
}

impl Page {
    const ALL: [Page; 3] = [Page::Words, Page::Key, Page::Serve];

    /// The page's place in the ring, from 0.
    fn index(self) -> usize {
        match self {
            Page::Words => 0,
            Page::Key => 1,
            Page::Serve => 2,
        }
    }

    fn next(self) -> Page {
        Page::ALL[(self.index() + 1) % Page::ALL.len()]
    }

    fn previous(self) -> Page {
        Page::ALL[(self.index() + Page::ALL.len() - 1) % Page::ALL.len()]
    }

    fn title(self) -> &'static str {
        match self {
            Page::Words => "words",
            Page::Key => "key as code",
            Page::Serve => "serve a laptop",
        }
    }
}

/// A spending request waiting for an answer: Both allows, Left or Right refuses.
struct Pending {
    seq: u32,
    request: Vec<u8>,
}

/// The idle menu, built once when it is entered and kept for the idle loop's life.
struct Menu<B: Board> {
    pad: Custody<B>,
    down: B::Link,
    rx: Vec<u8>,
    page: Page,
    /// Both was pressed on the Serve page: requests on the down link are answered.
    serving: bool,
    pending: Option<Pending>,
    /// Whether the down link reported a peer on the last step, so a change is logged once.
    peer_seen: bool,
    /// Another screen (the code, or an error) covers the page; any gesture goes back to it.
    covered: bool,
}

enum Idle<B: Board> {
    /// Not entered yet.
    Off,
    /// The pad or the ledger would not open. Shown once.
    Broken,
    /// Boxed: the menu holds the custodian and the link, and lives beside the pass state.
    On(Box<Menu<B>>),
}

/// The answer the holder gave with the buttons, handed to `serve`.
struct Answer(bool);

impl Consent for Answer {
    fn allow(&mut self, _: &Request) -> bool {
        self.0
    }
}

pub struct Badge<B: Board> {
    board: B,
    circle: Option<Circle>,
    state: State<B::Link>,
    idle: Idle<B>,
    /// The chip slot of the seat key this pass made, until the circle is kept. A pass that
    /// fails or is discarded deletes the slot's objects.
    chip_slot: Option<u8>,
}

impl<B: Board> Badge<B> {
    pub fn new(board: B) -> Self {
        Self {
            board,
            circle: None,
            state: State::Idle,
            idle: Idle::Off,
            chip_slot: None,
        }
    }

    pub fn board(&mut self) -> &mut B {
        &mut self.board
    }

    pub fn circle(&self) -> Option<&Circle> {
        self.circle.as_ref()
    }

    /// The self-test screen for 2.5 s or until Both, the A-held wipe, and the words of a
    /// stored circle.
    pub fn boot(&mut self) {
        let mut lines: Vec<String> = Vec::new();
        let mut sample = vec![0u8; 4096];
        lines.push(match self.board.entropy().fill(&mut sample) {
            Ok(()) => "trng: health tests pass".into(),
            Err(e) => format!("trng: FAILED {e:?}"),
        });
        lines.extend(self.board.selftest());
        let circle = self.board.load_circle();
        lines.push(match &circle {
            Some(c) => format!(
                "circle: {} seat {} of {}",
                c.params.name, c.seat, c.params.n
            ),
            None => "circle: none yet".into(),
        });
        for l in &lines {
            self.board.log(l);
        }
        self.reconcile_chip();
        // A must already be held at power-on to mean "forget the circle". Sample it before
        // the wait: a Both that ends the wait holds A down too, and must not wipe.
        let held_at_power_on = self.board.a_down();
        show(&mut self.board, screens::self_test(&lines));
        let mut skipped = false;
        for _ in 0..SELF_TEST_MS / POLL_MS {
            if self.board.press() == Press::Both {
                skipped = true;
                break;
            }
            self.board.delay_ms(POLL_MS);
        }

        // A held from power-on through the whole self-test forgets the circle: record,
        // ledger and pad. A Both that ended the wait early never does.
        let wipe = held_at_power_on && !skipped && self.board.a_down();
        self.circle = if circle.is_some() && wipe {
            let pad_len = circle.as_ref().map_or(0, |c| c.params.pad_len);
            match self.board.wipe(pad_len) {
                Ok(()) => {
                    self.forget_chip();
                    self.board.log("A held at boot: circle forgotten");
                    show(
                        &mut self.board,
                        screens::status(&["circle forgotten", "release A"]),
                    );
                    while self.board.a_down() {
                        self.board.delay_ms(10);
                    }
                    None
                }
                Err(_) => {
                    self.board.log("wipe failed");
                    circle
                }
            }
        } else {
            circle
        };

        if let Some(c) = &self.circle {
            let words = c.check_words();
            show_words(&mut self.board, c, &words);
            self.board
                .log(&format!("words from the record: {}", words.join(" ")));
        }
    }

    /// One pass: find the pile, run the ceremony, keep or discard the result. Steps until
    /// the pass ends, with a 1 ms delay between steps.
    pub fn run_once(&mut self) -> Result<(), String> {
        loop {
            match self.step() {
                Step::Kept | Step::Discarded => return Ok(()),
                Step::Failed(e) => return Err(e),
                _ => self.board.delay_ms(1),
            }
        }
    }

    /// Advance the pass by one poll. Never blocks, so two badges can run on one thread.
    pub fn step(&mut self) -> Step {
        let state = core::mem::replace(&mut self.state, State::Idle);
        match self.advance(state) {
            Ok((next, step)) => {
                self.state = next;
                step
            }
            Err(e) => {
                self.drop_chip_seat();
                Step::Failed(e)
            }
        }
    }

    /// At boot: delete every chip object in a slot the chip record does not know, and log it.
    fn reconcile_chip(&mut self) {
        if self.board.chip().is_none() {
            return;
        }
        let stored = self.board.chip_record_load();
        let mut record = stored.clone().unwrap_or_default();
        let Some(chip) = self.board.chip() else {
            return;
        };
        let lines = record.reconcile(chip);
        for l in &lines {
            self.board.log(l);
        }
        if stored.is_some_and(|s| s != record) && self.board.chip_record_store(&record).is_err() {
            self.board.log("chip record: write failed");
        }
    }

    /// The wipe at boot: every chip object and the chip record go with the circle.
    fn forget_chip(&mut self) {
        let Some(chip) = self.board.chip() else {
            return;
        };
        if let Err(e) = chip::delete_all(chip) {
            self.board.log(&format!("chip: wipe failed: {e:?}"));
        }
        let empty = chip::ChipRecord::default();
        if self.board.chip_record_store(&empty).is_err() {
            self.board.log("chip record: write failed");
        }
    }

    /// Make this pass's seat key in the chip, in the chip record's lowest free slot. Anything
    /// left in that slot is deleted first: the record does not know it, so it is not reused.
    fn make_chip_seat(&mut self) -> Result<(u8, SeatPublic), String> {
        let record = self.board.chip_record_load().unwrap_or_default();
        let slot = record.next_free_slot().ok_or("chip: every slot is in use")?;
        let chip = self.board.chip().ok_or("chip: not found")?;
        chip::delete_slot(chip, slot).map_err(|e| format!("chip: {e:?}"))?;
        let public = chip
            .generate(KeyId::new(KeyKind::Seat, slot), Curve::P256)
            .map_err(|e| format!("chip: key not made: {e:?}"))?;
        // Set before the parse, so a failure below still deletes the key.
        self.chip_slot = Some(slot);
        let public = SeatPublic::parse(&public).map_err(|_| "chip: bad public key")?;
        self.board
            .log(&format!("seat key made in the chip, slot {slot}"));
        Ok((slot, public))
    }

    /// At the keep: the root and the epoch counter go into the chip beside the seat key, and
    /// the chip record learns the slot.
    fn keep_in_chip(&mut self, slot: u8, circle_id: &CircleId, root: &[u8; 32]) -> Result<(), String> {
        let chip = self.board.chip().ok_or("chip: not found")?;
        chip.put_hmac_key(KeyId::new(KeyKind::Root, slot), root)
            .map_err(|e| format!("chip: root not stored: {e:?}"))?;
        chip.put_counter(KeyId::new(KeyKind::Epoch, slot), 0)
            .map_err(|e| format!("chip: counter not stored: {e:?}"))?;
        let mut record = self.board.chip_record_load().unwrap_or_default();
        record.insert(SlotEntry {
            slot,
            circle_id: *circle_id,
            has_seat: true,
            has_root: true,
            has_epoch: true,
        });
        self.board
            .chip_record_store(&record)
            .map_err(|_| String::from("flash: chip record failed"))
    }

    /// A pass that made a chip key ended without keeping it: delete the slot's objects and
    /// any record entry for it.
    fn drop_chip_seat(&mut self) {
        let Some(slot) = self.chip_slot.take() else {
            return;
        };
        if let Some(chip) = self.board.chip() {
            if let Err(e) = chip::delete_slot(chip, slot) {
                self.board
                    .log(&format!("chip: slot {slot} not deleted: {e:?}"));
            }
        }
        if let Some(mut record) = self.board.chip_record_load() {
            if record.entry(slot).is_some() {
                record.remove(slot);
                if self.board.chip_record_store(&record).is_err() {
                    self.board.log("chip record: write failed");
                }
            }
        }
        self.board.log(&format!("chip: slot {slot} deleted"));
    }

    fn advance(&mut self, state: State<B::Link>) -> Result<(State<B::Link>, Step), String> {
        match state {
            State::Idle => {
                show(&mut self.board, screens::pile(None));
                let (down, up) = self.board.links();
                let (name, pad_len) = self.board.proposal();
                let pile = Pile::new(down, up, &name, pad_len, false);
                Ok((State::Pile { pile, shown: None }, Step::InPile))
            }
            State::Pile {
                mut pile,
                mut shown,
            } => {
                if let Some(ready) = pile.poll().map_err(|e| format!("pile: {e:?}"))? {
                    let params = ready.params.clone();
                    let seat = ready.seat;
                    self.board.log(&format!(
                        "pile counted: seat {seat} of {}, circle {}",
                        params.n, params.name
                    ));
                    show_status(
                        &mut self.board,
                        &format!("seat {seat} of {}: {}", params.n, params.name),
                    );
                    let tier = self.board.tier();
                    // The commitment binds the seat key, so it is made now: in the chip if
                    // the board has one, otherwise by the ceremony, in software.
                    let in_chip = self.board.chip().is_some();
                    show(&mut self.board, screens::making_key(in_chip));
                    let ring = if in_chip {
                        let (_, public) = self.make_chip_seat()?;
                        pile.into_ceremony_with_seat(ready, self.board.entropy(), public, tier)
                    } else {
                        pile.into_ceremony(ready, self.board.entropy(), tier)
                    };
                    let mut ring = ring.map_err(|e| format!("ceremony: {e:?}"))?;
                    show_status(&mut self.board, "ceremony: waiting for the others");
                    ring.start();
                    let state = State::Ceremony {
                        ring: Box::new(ring),
                        last: (0, 0, 0),
                        params,
                        seat,
                    };
                    return Ok((state, Step::InCeremony));
                }
                if pile.seat() != shown {
                    shown = pile.seat();
                    if let Some(s) = shown {
                        self.board.log(&format!("seat {s}"));
                        show(&mut self.board, screens::pile(Some(s)));
                    }
                }
                // Both on the top card starts the count. Left and Right do nothing here.
                if self.board.press() == Press::Both {
                    match pile.start() {
                        Ok(()) => {
                            self.board.log("Both: counting the pile");
                            show_status(&mut self.board, "counting the pile...");
                        }
                        Err(PileError::NoSeat) => show_status(&mut self.board, "nothing below yet"),
                        Err(e) => self.board.log(&format!("start: {e:?}")),
                    }
                }
                Ok((State::Pile { pile, shown }, Step::InPile))
            }
            State::Ceremony {
                mut ring,
                mut last,
                params,
                seat,
            } => {
                // No gesture means anything during the ceremony; drop them so none is
                // taken as an answer to the keep prompt.
                let _ = self.board.press();
                let step = match self.chip_slot {
                    Some(slot) => {
                        let chip = self.board.chip().ok_or("chip: not found")?;
                        let mut seat = ChipSeat::new(chip, KeyId::new(KeyKind::Seat, slot));
                        let step = ring.poll_signing(&mut seat);
                        if let Some(e) = seat.error {
                            self.board.log(&format!("chip: signing failed: {e:?}"));
                        }
                        step
                    }
                    None => ring.poll(),
                };
                let step = step.map_err(|e| {
                    show_status(&mut self.board, "ceremony: ABORTED");
                    format!("ceremony: {e:?}")
                })?;
                let now = (
                    ring.commitments_held(),
                    ring.reveals_held(),
                    ring.attestations_held(),
                );
                if now != last {
                    last = now;
                    show(
                        &mut self.board,
                        screens::ceremony(now.0, now.1, now.2, params.n),
                    );
                }
                if step != RingStep::Complete {
                    let state = State::Ceremony {
                        ring,
                        last,
                        params,
                        seat,
                    };
                    return Ok((state, Step::InCeremony));
                }
                let (result, links) = (*ring).finish().map_err(|e| format!("ceremony: {e:?}"))?;
                Ok((
                    State::Flush {
                        links,
                        result,
                        seat,
                    },
                    Step::InCeremony,
                ))
            }
            State::Flush {
                mut links,
                result,
                seat,
            } => {
                // Our neighbours may still need the frames we forwarded last. Keep reading
                // (and dropping: the ceremony is over) while we flush, so a neighbour that
                // is also flushing toward us is never blocked.
                let _ = self.board.press();
                let mut done = true;
                let mut buf = [0u8; 256];
                for link in links.iter_mut() {
                    if !link.flush() {
                        done = false;
                    }
                    while link.recv(&mut buf) > 0 {}
                }
                if !done {
                    let state = State::Flush {
                        links,
                        result,
                        seat,
                    };
                    return Ok((state, Step::InCeremony));
                }
                drop(links);
                show(&mut self.board, screens::keep_prompt(&result.check_words));
                self.board
                    .log(&format!("words: {}", result.check_words.join(" ")));
                // A gesture seen on the call that drew the prompt is not an answer to it.
                let _ = self.board.press();
                Ok((State::Prompt { result, seat }, Step::Prompt))
            }
            State::Prompt { result, seat } => {
                let gesture = self.board.press();
                if gesture == Press::Both {
                    show_status(&mut self.board, "saving...");
                    self.board
                        .write_pad(&result.pad)
                        .map_err(|_| "flash: pad write failed")?;
                    self.board
                        .records(true)
                        .map_err(|_| "flash: ledger erase failed")?;
                    let words = crate::board::word_indices(&result.check_words)
                        .ok_or("words: not in the word list")?;
                    let root = result.root.as_ref().ok_or("ceremony: no circle root")?;
                    // A chip keeps the seat key and the root; the record keeps only the slot.
                    // Without a chip the record keeps both.
                    let (seat_key, chip_slot, root) = match self.chip_slot {
                        Some(slot) => {
                            self.keep_in_chip(slot, &result.circle_id, root)?;
                            ([0u8; 32], Some(slot), None)
                        }
                        None => (
                            // This ceremony was built from a seed, so it holds the key.
                            *result
                                .seat_key
                                .as_ref()
                                .ok_or("ceremony: no seat key")?
                                .to_bytes(),
                            None,
                            Some(**root),
                        ),
                    };
                    let circle = Circle {
                        params: result.params.clone(),
                        seat,
                        circle_id: result.circle_id,
                        words,
                        seat_key,
                        chip_slot,
                        root,
                        tier: result.tier,
                        manifest: result.manifest.encode(),
                    };
                    self.board
                        .save_circle(&circle)
                        .map_err(|_| "flash: circle record failed")?;
                    // Kept: the slot belongs to the circle now.
                    self.chip_slot = None;
                    self.board
                        .log(&format!("saved: {} seat {seat}", circle.params.name));
                    show_status(&mut self.board, "saved. read the words aloud");
                    self.circle = Some(circle);
                    self.idle = Idle::Off;
                    return Ok((State::Idle, Step::Kept));
                }
                if matches!(gesture, Press::Left | Press::Right) {
                    show(&mut self.board, screens::discard_prompt());
                    return Ok((State::ConfirmDiscard { result, seat }, Step::Prompt));
                }
                Ok((State::Prompt { result, seat }, Step::Prompt))
            }
            State::ConfirmDiscard { result, seat } => match self.board.press() {
                Press::Both => {
                    drop(result);
                    self.drop_chip_seat();
                    self.board.log("discarded");
                    show_status(&mut self.board, "discarded");
                    Ok((State::Idle, Step::Discarded))
                }
                Press::Left | Press::Right => {
                    show(&mut self.board, screens::keep_prompt(&result.check_words));
                    Ok((State::Prompt { result, seat }, Step::Prompt))
                }
                Press::None => Ok((State::ConfirmDiscard { result, seat }, Step::Prompt)),
            },
        }
    }

    /// One poll of the idle menu, for a badge with a stored circle. Call it in a loop. The
    /// menu is a ring of pages, Words, Key and Serve: Left and Right turn the page, Both
    /// selects. Both on Key spends 32 bytes and shows them as a code; Both on Serve answers
    /// custodian requests from the down link, each spend allowed by Both or refused by Left
    /// or Right. Ends with a 1 ms delay. Does nothing until a circle is stored.
    pub fn idle_step(&mut self) {
        let Some(circle) = self.circle.clone() else {
            return;
        };
        if matches!(self.idle, Idle::Off) {
            self.idle = self.enter_menu(&circle);
        }
        if let Idle::On(menu) = &mut self.idle {
            Self::menu_step(&mut self.board, &circle, menu);
        }
        self.board.delay_ms(1);
    }

    fn enter_menu(&mut self, c: &Circle) -> Idle<B> {
        let len = c.params.pad_len;
        let tier = self.board.tier();
        let store = self.board.open_pad(len);
        let pad = self
            .board
            .records(false)
            .map_err(|_| String::from("flash: ledger read failed"))
            .and_then(|records| {
                PersistentLedger::open(records, c.params.slices())
                    .map_err(|e| format!("ledger: {e:?}"))
            })
            .and_then(|ledger| {
                Pad::with_circle_id(
                    c.params.clone(),
                    c.seat,
                    c.circle_id,
                    StorePad::new(store),
                    ledger,
                )
                .map(|pad| Tiered { inner: pad, tier })
                .map_err(|e| format!("pad: {e:?}"))
            });
        match pad {
            Ok(pad) => {
                let (down, _up) = self.board.links();
                let menu = Menu {
                    pad,
                    down,
                    rx: Vec::new(),
                    page: Page::Words,
                    serving: false,
                    pending: None,
                    peer_seen: false,
                    covered: false,
                };
                show_page(&mut self.board, c, &menu);
                Idle::On(Box::new(menu))
            }
            Err(e) => {
                self.board.log(&e);
                show_status(&mut self.board, &e);
                Idle::Broken
            }
        }
    }

    fn menu_step(board: &mut B, c: &Circle, m: &mut Menu<B>) {
        let asked = m.pending.is_some();
        let present = m.down.peer_present();
        let mut redraw = false;
        if present != m.peer_seen {
            m.peer_seen = present;
            board.log(if present {
                "down link: a laptop is listening"
            } else {
                "down link: nothing plugged in"
            });
            redraw = m.page == Page::Serve;
        }
        // Read a chunk and decode after each, so `rx` never holds more than one request
        // frame and one chunk, whatever arrives.
        let mut buf = [0u8; 256];
        loop {
            let n = m.down.recv(&mut buf);
            if n == 0 {
                break;
            }
            m.rx.extend_from_slice(&buf[..n]);
            Self::menu_decode(board, c, m);
        }

        // A gesture made before the prompt was on the screen is not an answer to it: on the
        // Nano the prompt's refresh blocks for hundreds of milliseconds. Drop the gesture
        // seen on this call and count only gestures seen on a later call.
        if !asked && m.pending.is_some() {
            let _ = board.press();
            return;
        }

        let gesture = board.press();
        if let Some(p) = &m.pending {
            let yes = match gesture {
                Press::None => return,
                Press::Both => true,
                Press::Left | Press::Right => false,
            };
            let resp = serve(&mut m.pad, &mut Answer(yes), &p.request);
            respond(&mut m.down, p.seq, &resp);
            m.pending = None;
            board.log(if yes {
                "spend allowed"
            } else {
                "spend refused"
            });
            // The prompt replaced whatever was on the screen; the page is back now.
            m.covered = false;
            show_page(board, c, m);
            return;
        }

        if m.covered {
            if gesture != Press::None {
                m.covered = false;
                show_page(board, c, m);
            }
            return;
        }

        match gesture {
            Press::None => {
                if redraw {
                    show_page(board, c, m);
                }
            }
            Press::Left | Press::Right => {
                m.page = if gesture == Press::Left {
                    m.page.previous()
                } else {
                    m.page.next()
                };
                m.serving = false;
                show_page(board, c, m);
            }
            Press::Both => match m.page {
                Page::Words => show_page(board, c, m),
                Page::Key => {
                    match m.pad.spend(&c.circle_id, PSK_LEN) {
                        Ok((_, bytes)) => show(board, screens::code(&bytes)),
                        Err(e) => show_status(board, &format!("key: {e:?}")),
                    }
                    m.covered = true;
                }
                Page::Serve => {
                    m.serving = true;
                    board.log("serving the down link");
                    show_page(board, c, m);
                }
            },
        }
    }

    /// Answer or queue every whole request frame in `rx`. Bytes that are not a frame, or a
    /// header longer than any request, are dropped one at a time to resync. A partial header
    /// left by garbage can claim a length that is never filled, so while the front is
    /// incomplete, a whole frame further on wins and the bytes before it are dropped.
    fn menu_decode(board: &mut B, c: &Circle, m: &mut Menu<B>) {
        loop {
            let (kind, seq, payload, used) = match frame::decode_max(&m.rx, MAX_REQUEST_LEN) {
                Ok((f, used)) => (f.kind, f.seq, f.payload.to_vec(), used),
                Err(FrameError::Incomplete) => {
                    let whole = |i: &usize| frame::decode_max(&m.rx[*i..], MAX_REQUEST_LEN).is_ok();
                    match (1..m.rx.len()).find(whole) {
                        Some(skip) => {
                            m.rx.drain(..skip);
                            continue;
                        }
                        None => break,
                    }
                }
                Err(FrameError::Crc) => {
                    m.rx.remove(0);
                    continue;
                }
            };
            m.rx.drain(..used);
            if kind != KIND_REQUEST {
                continue;
            }
            if !m.serving || m.pending.is_some() {
                let busy = Response::Error {
                    code: ErrorCode::Busy,
                };
                respond(&mut m.down, seq, &busy.encode());
                continue;
            }
            match Request::decode(&payload).ok().as_ref().and_then(spends) {
                Some(n) => {
                    show(board, screens::consent(n, &c.params.name));
                    m.pending = Some(Pending {
                        seq,
                        request: payload,
                    });
                }
                // Identify, Remaining and anything malformed: answered now, never asked.
                None => {
                    let resp = serve(&mut m.pad, &mut Answer(false), &payload);
                    respond(&mut m.down, seq, &resp);
                }
            }
        }
    }
}

/// The bytes a request would spend, or `None` if it spends nothing.
fn spends(req: &Request) -> Option<u32> {
    match req {
        Request::Identify | Request::Remaining { .. } => None,
        Request::Psk { .. } | Request::PskFor { .. } => Some(PSK_LEN),
        Request::Spend { len, .. } | Request::SpendFor { len, .. } => Some(*len),
    }
}

/// Send `response` back as a response frame echoing the request's `seq`.
fn respond<L: Link>(link: &mut L, seq: u32, response: &[u8]) {
    link.send(&frame::encode(&Frame {
        kind: KIND_RESPONSE,
        seq,
        payload: response,
    }));
}

/// Note `screen`'s lines on the board, then draw its frame.
fn show<B: Board>(board: &mut B, screen: Screen) {
    board.note_screen(&screen.lines, screen.secret);
    board.display().draw(&screen.frame);
}

/// A status screen of one line.
fn show_status<B: Board>(board: &mut B, s: &str) {
    show(board, screens::status(&[s]));
}

/// The menu's current page for the stored circle `c`.
fn show_page<B: Board>(board: &mut B, c: &Circle, m: &Menu<B>) {
    let tier = tier_label(board.tier());
    let body: Vec<&str> = match m.page {
        Page::Words => {
            let words = c.check_words();
            let page = screens::words_page(
                &c.params.name,
                &tier,
                m.page.index() + 1,
                Page::ALL.len(),
                &words,
                true,
                true,
            );
            show(board, page);
            return;
        }
        Page::Key => vec!["Both to spend 32 bytes"],
        Page::Serve => {
            let mut body = vec![if m.serving {
                "serving"
            } else {
                "Both to start serving"
            }];
            if m.peer_seen {
                body.push("a laptop is listening");
            }
            body
        }
    };
    let page = screens::menu_page(
        &c.params.name,
        &tier,
        m.page.index() + 1,
        Page::ALL.len(),
        m.page.title(),
        &body,
        true,
        true,
    );
    show(board, page);
}

/// The words of the stored circle `c`, under its header.
fn show_words<B: Board>(board: &mut B, c: &Circle, words: &[&str; 6]) {
    let tier = tier_label(board.tier());
    show(board, screens::words(&c.params.name, &tier, words));
}

/// The header's label for a board's tier.
fn tier_label(tier: u8) -> String {
    format!("tier {tier}")
}
