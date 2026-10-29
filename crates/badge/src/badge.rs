//! [`Badge`]: the boot screen, the pile and the ceremony, and the idle menu.

use crate::apps::{BadgeApps, Served};
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
use cahoots_core::frame::{self, Frame, FrameError};
use cahoots_core::pad::Pad;
use cahoots_core::params::CircleParams;
use cahoots_core::seat::SeatPublic;
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

/// One circle's stored pad and its ledger.
type CirclePad<B> = Pad<StorePad<<B as Board>::Pad>, PersistentLedger<<B as Board>::Records>>;

/// The custodian a badge serves: the pad and ledger of every circle it holds, each request
/// sent to the circle it names, reporting the board's tier. A circle it does not hold is
/// `UnknownCircle`.
struct Pads<C> {
    held: Vec<(CircleId, C)>,
    tier: u8,
}

impl<C: Custodian> Pads<C> {
    fn get(&self, circle: &CircleId) -> Option<&C> {
        self.held.iter().find(|(id, _)| id == circle).map(|(_, c)| c)
    }

    fn get_mut(&mut self, circle: &CircleId) -> Result<&mut C, PadError> {
        self.held
            .iter_mut()
            .find(|(id, _)| id == circle)
            .map(|(_, c)| c)
            .ok_or(PadError::UnknownCircle)
    }
}

impl<C: Custodian> Custodian for Pads<C> {
    fn circles(&self) -> Vec<CircleInfo> {
        let mut circles: Vec<CircleInfo> = self.held.iter().flat_map(|(_, c)| c.circles()).collect();
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
        self.get_mut(circle)?.psk(circle, context)
    }

    fn psk_for(
        &mut self,
        handle: &PskHandle,
        context: &[u8],
    ) -> Result<Zeroizing<[u8; 32]>, PadError> {
        self.get_mut(&handle.circle_id)?.psk_for(handle, context)
    }

    fn spend(
        &mut self,
        circle: &CircleId,
        len: u32,
    ) -> Result<(PskHandle, Zeroizing<Vec<u8>>), PadError> {
        self.get_mut(circle)?.spend(circle, len)
    }

    fn spend_for(&mut self, handle: &PskHandle, len: u32) -> Result<Zeroizing<Vec<u8>>, PadError> {
        self.get_mut(&handle.circle_id)?.spend_for(handle, len)
    }

    fn remaining(&self, circle: &CircleId) -> u64 {
        self.get(circle).map_or(0, |c| c.remaining(circle))
    }

    fn low_pad(&self, circle: &CircleId) -> bool {
        self.get(circle).is_some_and(|c| c.low_pad(circle))
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
    /// Both shows the list of circles, where another circle opens or a new one starts.
    Circles,
    /// Both asks, then forgets this circle and nothing else.
    Forget,
}

impl Page {
    const ALL: [Page; 5] = [Page::Words, Page::Key, Page::Serve, Page::Circles, Page::Forget];

    /// The page's place in the ring, from 0.
    fn index(self) -> usize {
        match self {
            Page::Words => 0,
            Page::Key => 1,
            Page::Serve => 2,
            Page::Circles => 3,
            Page::Forget => 4,
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
            Page::Circles => "circles",
            Page::Forget => "forget this circle",
        }
    }
}

/// A request waiting for an answer, a spend or a signature: Both allows, Left or Right
/// refuses.
struct Pending {
    seq: u32,
    request: Vec<u8>,
    /// A `Chip` request: the log says "signature", not "spend".
    chip: bool,
}

/// The idle menu of one circle, built when it is entered. It holds every circle's pad, since
/// Serve answers for any circle the badge holds.
struct Menu<B: Board> {
    pads: Pads<CirclePad<B>>,
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
    /// Both on the Forget page asked "forget this circle?": Both forgets, Left or Right goes
    /// back.
    confirm_forget: bool,
}

/// What the menu asks of the badge after a step.
enum MenuAction {
    Stay,
    /// Show the list of circles.
    Choose,
    /// Forget the circle whose menu this is.
    Forget,
}

enum Idle<B: Board> {
    /// Not entered yet. With one circle, or right after a circle is kept, the next step opens
    /// its menu; with more, the list of circles.
    Off,
    /// The list of circles, `index` the entry marked: Left and Right move, Both opens that
    /// circle's menu. The last entry, "new circle", is there while the board has room.
    Choose { index: usize },
    /// Boxed: the menu holds the custodian and the link, and lives beside the pass state.
    On(Box<Menu<B>>),
    /// "new circle" was chosen: the pass runs from the idle loop until it ends.
    NewCircle,
}

/// The chooser's last entry, which starts a pass for one more circle.
const NEW_CIRCLE: &str = "new circle";

/// The answer the holder gave with the buttons, handed to `serve`.
struct Answer(bool);

impl Consent for Answer {
    fn allow(&mut self, _: &Request) -> bool {
        self.0
    }
}

pub struct Badge<B: Board> {
    board: B,
    /// Every circle the badge holds, in the board's order. Nothing is shared between them
    /// but the badge: each has its own record, pad, ledger, seat key, root and chip slot.
    circles: Vec<Circle>,
    /// The circle whose menu is open, or was last.
    current: usize,
    /// The next idle step opens the menu of `current`, not the list: a circle was just kept.
    open_current: bool,
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
            circles: Vec::new(),
            current: 0,
            open_current: false,
            state: State::Idle,
            idle: Idle::Off,
            chip_slot: None,
        }
    }

    pub fn board(&mut self) -> &mut B {
        &mut self.board
    }

    /// The circle whose menu is open or was last, or `None` if the badge holds none.
    pub fn circle(&self) -> Option<&Circle> {
        self.circles.get(self.current).or(self.circles.first())
    }

    /// Every circle the badge holds.
    pub fn circles(&self) -> &[Circle] {
        &self.circles
    }

    /// The chip slots the held circles name.
    fn held_slots(&self) -> Vec<u8> {
        self.circles.iter().filter_map(|c| c.chip_slot).collect()
    }

    /// The self-test screen for 2.5 s or until Both, the A-held wipe, and the words of a
    /// stored circle when there is one.
    pub fn boot(&mut self) {
        let mut lines: Vec<String> = Vec::new();
        let mut sample = vec![0u8; 4096];
        lines.push(match self.board.entropy().fill(&mut sample) {
            Ok(()) => "trng: health tests pass".into(),
            Err(e) => format!("trng: FAILED {e:?}"),
        });
        lines.extend(self.board.selftest());
        let circles = self.board.load_circles();
        if circles.is_empty() {
            lines.push("circle: none yet".into());
        }
        for c in &circles {
            lines.push(format!(
                "circle: {} seat {} of {}",
                c.params.name, c.seat, c.params.n
            ));
        }
        for l in &lines {
            self.board.log(l);
        }
        self.reconcile_chip(circles.iter().filter_map(|c| c.chip_slot).collect());
        // A must already be held at power-on to mean "forget every circle". Sample it before
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

        // A held from power-on through the whole self-test forgets every circle: records,
        // ledgers, pads, chip objects and the chip record. A Both that ended the wait early
        // never does.
        let wipe = held_at_power_on && !skipped && self.board.a_down();
        self.circles = if !circles.is_empty() && wipe {
            self.forget_all(&circles)
        } else {
            circles
        };
        self.current = 0;
        self.open_current = false;
        self.idle = Idle::Off;

        // With more than one circle the list comes first, in the idle loop.
        if let [c] = self.circles.as_slice() {
            let c = c.clone();
            let words = c.check_words();
            show_words(&mut self.board, &c, &words);
            self.board
                .log(&format!("words from the record: {}", words.join(" ")));
        }
    }

    /// The wipe at boot: every circle's pad and ledger, the list of records, every chip
    /// object and the chip record. Returns the circles left: none, or all of them if a
    /// write failed.
    ///
    /// Each circle's record goes before its pad, inside [`Board::forget`]; every circle is
    /// tried even if one fails, and the chip is wiped either way: a circle whose record is
    /// gone is never served, so its keys have nothing left to answer for.
    fn forget_all(&mut self, circles: &[Circle]) -> Vec<Circle> {
        let mut ok = true;
        for c in circles {
            ok &= self.board.forget(&c.circle_id, c.params.pad_len).is_ok();
        }
        ok &= self.board.store_circles(&[]).is_ok();
        let left = self.board.load_circles();
        let left_slots: Vec<u8> = left.iter().filter_map(|c| c.chip_slot).collect();
        if !ok {
            self.board.log("wipe failed");
            if !left_slots.is_empty() {
                // A circle whose record could not be erased keeps its chip keys.
                return left;
            }
        }
        self.forget_chip();
        if !ok {
            return left;
        }
        self.board.log("A held at boot: every circle forgotten");
        let what = if circles.len() == 1 {
            "circle forgotten"
        } else {
            "circles forgotten"
        };
        show(&mut self.board, screens::status(&[what, "release A"]));
        while self.board.a_down() {
            self.board.delay_ms(10);
        }
        Vec::new()
    }

    /// Forget one circle and nothing else: its record first (inside [`Board::forget`]), then
    /// its pad and ledger, then its chip objects and chip record entry. A record never stays
    /// over a blank pad: a failure after the record is gone leaves only bytes in a free place,
    /// or chip objects that a slot no circle names, which are not served.
    fn forget_circle(&mut self, c: &Circle) -> Result<(), String> {
        let forgot = self
            .board
            .forget(&c.circle_id, c.params.pad_len)
            .map_err(|_| String::from("flash: forgetting failed"));
        let rest: Vec<Circle> = self
            .circles
            .iter()
            .filter(|x| x.circle_id != c.circle_id)
            .cloned()
            .collect();
        let stored = self
            .board
            .store_circles(&rest)
            .map_err(|_| String::from("flash: circle records failed"));
        if self.board.load_circles().iter().any(|x| x.circle_id == c.circle_id) {
            // The record is still there, so is everything behind it.
            return forgot.and(stored).and(Err(String::from("flash: record not erased")));
        }
        if let Some(slot) = c.chip_slot {
            if let Some(chip) = self.board.chip() {
                chip::delete_slot(chip, slot).map_err(|e| format!("chip: {e:?}"))?;
            }
            match self.board.chip_record_load() {
                Ok(Some(mut record)) if record.entry(slot).is_some() => {
                    record.remove(slot);
                    self.board
                        .chip_record_store(&record)
                        .map_err(|_| String::from("flash: chip record failed"))?;
                }
                Ok(_) => {}
                Err(_) => self
                    .board
                    .log("chip record: unreadable, left as it is"),
            }
        }
        forgot.and(stored)
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
    /// A record that cannot be read deletes nothing: it is not the same as no record. A slot
    /// a kept circle names is never deleted, whatever the record says.
    fn reconcile_chip(&mut self, keep: Vec<u8>) {
        if self.board.chip().is_none() {
            return;
        }
        let stored = match self.board.chip_record_load() {
            Ok(stored) => stored,
            Err(_) => {
                self.board
                    .log("chip record: unreadable, chip objects left as they are");
                return;
            }
        };
        let mut record = stored.clone().unwrap_or_default();
        let Some(chip) = self.board.chip() else {
            return;
        };
        let lines = record.reconcile_keeping(chip, &keep);
        for l in &lines {
            self.board.log(l);
        }
        if stored.is_some_and(|s| s != record) && self.board.chip_record_store(&record).is_err() {
            self.board.log("chip record: write failed");
        }
    }

    /// The wipe at boot: every chip object and the chip record go with the circles.
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

    /// Make this pass's seat key in the chip, in the chip record's lowest free slot that no
    /// held circle names. Anything left in that slot is deleted first: the record does not
    /// know it, so it is not reused. A slot a circle holds is never chosen.
    fn make_chip_seat(&mut self) -> Result<(u8, SeatPublic), String> {
        let record = self.load_chip_record()?;
        let kept = self.held_slots();
        let slot = (0..=u8::MAX)
            .find(|&s| record.entry(s).is_none() && !kept.contains(&s))
            .ok_or("chip: every slot is in use")?;
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
    fn keep_in_chip(
        &mut self,
        slot: u8,
        circle_id: &CircleId,
        root: &[u8; 32],
    ) -> Result<(), String> {
        let chip = self.board.chip().ok_or("chip: not found")?;
        chip.put_hmac_key(KeyId::new(KeyKind::Root, slot), root)
            .map_err(|e| format!("chip: root not stored: {e:?}"))?;
        chip.put_counter(KeyId::new(KeyKind::Epoch, slot), 0)
            .map_err(|e| format!("chip: counter not stored: {e:?}"))?;
        let mut record = self.load_chip_record()?;
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

    /// The stored chip record, empty if there is none. An unreadable one is an error, logged:
    /// writing over it, or choosing a slot from nothing, could lose a kept circle's keys.
    fn load_chip_record(&mut self) -> Result<chip::ChipRecord, String> {
        match self.board.chip_record_load() {
            Ok(r) => Ok(r.unwrap_or_default()),
            Err(_) => {
                let e = "chip record: unreadable, no key made or kept";
                self.board.log(e);
                Err(String::from(e))
            }
        }
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
        if let Ok(Some(mut record)) = self.board.chip_record_load() {
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
                let screen = self.pile_screen(None);
                show(&mut self.board, screen);
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
                        let screen = self.pile_screen(Some(s));
                        show(&mut self.board, screen);
                    }
                }
                // Both on the top card starts the count. On a badge that holds circles, Left
                // or Right leaves the pile and goes back to the list; otherwise they do
                // nothing here.
                let gesture = self.board.press();
                if matches!(gesture, Press::Left | Press::Right) && !self.circles.is_empty() {
                    drop(pile);
                    self.board.log("left the pile");
                    return Ok((State::Idle, Step::Discarded));
                }
                if gesture == Press::Both {
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
                    let id = result.circle_id;
                    if self.circles.iter().any(|c| c.circle_id == id) {
                        return Err(String::from("circle: already held"));
                    }
                    self.board
                        .write_pad(&id, &result.pad)
                        .map_err(|_| "flash: pad write failed")?;
                    self.board
                        .records(&id, true)
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
                    let mut all = self.circles.clone();
                    all.push(circle);
                    self.board
                        .store_circles(&all)
                        .map_err(|_| "flash: circle record failed")?;
                    // Kept: the slot belongs to the circle now.
                    self.chip_slot = None;
                    self.board
                        .log(&format!("saved: {} seat {seat}", result.params.name));
                    show_status(&mut self.board, "saved. read the words aloud");
                    // The board's order, which is the order the next boot lists them in.
                    let stored = self.board.load_circles();
                    self.circles = if stored.iter().any(|c| c.circle_id == id) {
                        stored
                    } else {
                        all
                    };
                    self.current = self
                        .circles
                        .iter()
                        .position(|c| c.circle_id == id)
                        .unwrap_or(0);
                    self.open_current = true;
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

    /// The pile screen: with a way back to the list when the badge holds circles.
    fn pile_screen(&self, seat: Option<u8>) -> Screen {
        if self.circles.is_empty() {
            screens::pile(seat)
        } else {
            screens::pile_or_back(seat)
        }
    }

    /// One poll of the idle loop, for a badge that holds a circle. Call it in a loop. Does
    /// nothing until a circle is stored. Ends with a 1 ms delay.
    ///
    /// With one circle, its menu opens; with more, the list of circles comes first: Left and
    /// Right move, Both opens that circle's menu, and the last entry, "new circle", starts a
    /// pass for one more while the board has room. The menu is a ring of pages, Words, Key,
    /// Serve, Circles and Forget: Left and Right turn the page, Both selects. Both on Key
    /// spends 32 bytes and shows them as a code; Both on Serve answers custodian requests
    /// from the down link for any circle the badge holds, each spend or signature allowed by
    /// Both or refused by Left or Right; Both on Circles goes back to the list; Both on
    /// Forget asks, and Both again forgets this circle and nothing else.
    pub fn idle_step(&mut self) {
        if self.circles.is_empty() {
            return;
        }
        let idle = core::mem::replace(&mut self.idle, Idle::Off);
        self.idle = match idle {
            Idle::Off => {
                if self.circles.len() == 1 || self.open_current {
                    self.open_current = false;
                    if self.current >= self.circles.len() {
                        self.current = 0;
                    }
                    self.enter_menu()
                } else {
                    self.current = self.current.min(self.circles.len() - 1);
                    self.show_chooser(self.current);
                    Idle::Choose {
                        index: self.current,
                    }
                }
            }
            Idle::Choose { index } => self.choose_step(index),
            Idle::NewCircle => self.new_circle_step(),
            Idle::On(mut menu) => {
                match Self::menu_step(&mut self.board, &self.circles, self.current, &mut menu) {
                    MenuAction::Stay => Idle::On(menu),
                    MenuAction::Choose => {
                        drop(menu);
                        self.show_chooser(self.current);
                        Idle::Choose {
                            index: self.current,
                        }
                    }
                    MenuAction::Forget => {
                        drop(menu);
                        self.forget_current();
                        Idle::Off
                    }
                }
            }
        };
        self.board.delay_ms(1);
    }

    /// The chooser's entries: each circle's name, a number after a name two circles share,
    /// and "new circle" while the board has room for one more.
    fn chooser_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .circles
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let name = &c.params.name;
                if self.circles.iter().filter(|d| d.params.name == *name).count() > 1 {
                    format!("{name} {}", i + 1)
                } else {
                    name.clone()
                }
            })
            .collect();
        if self.circles.len() < self.board.max_circles() {
            names.push(String::from(NEW_CIRCLE));
        }
        names
    }

    fn show_chooser(&mut self, index: usize) {
        let names = self.chooser_names();
        show(&mut self.board, screens::chooser(&names, index));
    }

    fn choose_step(&mut self, index: usize) -> Idle<B> {
        let entries = self.chooser_names().len();
        let index = index.min(entries - 1);
        match self.board.press() {
            Press::None => Idle::Choose { index },
            Press::Left => {
                let index = (index + entries - 1) % entries;
                self.show_chooser(index);
                Idle::Choose { index }
            }
            Press::Right => {
                let index = (index + 1) % entries;
                self.show_chooser(index);
                Idle::Choose { index }
            }
            Press::Both if index < self.circles.len() => {
                self.current = index;
                self.enter_menu()
            }
            Press::Both => {
                self.board.log("new circle: looking for the pile");
                self.state = State::Idle;
                Idle::NewCircle
            }
        }
    }

    /// Run the pass for a new circle by one step. Kept opens the new circle's menu; anything
    /// else that ends the pass goes back to the list.
    fn new_circle_step(&mut self) -> Idle<B> {
        match self.step() {
            Step::Kept => Idle::Off,
            Step::Discarded => {
                self.show_chooser(self.current);
                Idle::Choose {
                    index: self.current,
                }
            }
            Step::Failed(e) => {
                self.board.log(&format!("new circle: {e}"));
                self.show_chooser(self.current);
                Idle::Choose {
                    index: self.current,
                }
            }
            _ => Idle::NewCircle,
        }
    }

    /// Forget the circle whose menu was open, and say so.
    fn forget_current(&mut self) {
        let Some(c) = self.circles.get(self.current).cloned() else {
            return;
        };
        match self.forget_circle(&c) {
            Ok(()) => {
                self.board
                    .log(&format!("forgotten: {} seat {}", c.params.name, c.seat));
                show_status(&mut self.board, "circle forgotten");
            }
            Err(e) => {
                self.board.log(&format!("forget: {e}"));
                show_status(&mut self.board, &e);
            }
        }
        self.circles = self.board.load_circles();
        self.current = 0;
        self.open_current = false;
    }

    /// Open circle `c`'s pad and ledger.
    fn open_circle_pad(&mut self, c: &Circle) -> Result<CirclePad<B>, String> {
        let store = self
            .board
            .open_pad(&c.circle_id, c.params.pad_len)
            .map_err(|_| String::from("flash: pad read failed"))?;
        let records = self
            .board
            .records(&c.circle_id, false)
            .map_err(|_| String::from("flash: ledger read failed"))?;
        let ledger = PersistentLedger::open(records, c.params.slices())
            .map_err(|e| format!("ledger: {e:?}"))?;
        Pad::with_circle_id(c.params.clone(), c.seat, c.circle_id, StorePad::new(store), ledger)
            .map_err(|e| format!("pad: {e:?}"))
    }

    /// The menu of circle `current`, with every circle's pad open for Serve. A pad that will
    /// not open is logged and left out: requests for that circle are `UnknownCircle`, and if
    /// it is this circle's, the error covers the first page.
    fn enter_menu(&mut self) -> Idle<B> {
        let tier = self.board.tier();
        let mut held = Vec::new();
        let mut broken = None;
        for (i, c) in self.circles.clone().iter().enumerate() {
            match self.open_circle_pad(c) {
                Ok(pad) => held.push((c.circle_id, pad)),
                Err(e) => {
                    self.board.log(&format!("{}: {e}", c.params.name));
                    if i == self.current {
                        broken = Some(e);
                    }
                }
            }
        }
        let (down, _up) = self.board.links();
        let mut menu = Menu {
            pads: Pads { held, tier },
            down,
            rx: Vec::new(),
            page: Page::Words,
            serving: false,
            pending: None,
            peer_seen: false,
            covered: false,
            confirm_forget: false,
        };
        let c = &self.circles[self.current];
        show_page(&mut self.board, c, &menu);
        if let Some(e) = broken {
            show_status(&mut self.board, &e);
            menu.covered = true;
        }
        Idle::On(Box::new(menu))
    }

    fn menu_step(board: &mut B, circles: &[Circle], current: usize, m: &mut Menu<B>) -> MenuAction {
        let c = &circles[current];
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
            redraw = m.page == Page::Serve && !m.covered && !m.confirm_forget;
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
            Self::menu_decode(board, circles, m);
        }

        // A gesture made before the prompt was on the screen is not an answer to it: on the
        // Nano the prompt's refresh blocks for hundreds of milliseconds. Drop the gesture
        // seen on this call and count only gestures seen on a later call.
        if !asked && m.pending.is_some() {
            let _ = board.press();
            return MenuAction::Stay;
        }

        let gesture = board.press();
        if let Some(p) = &m.pending {
            let yes = match gesture {
                Press::None => return MenuAction::Stay,
                Press::Both => true,
                Press::Left | Press::Right => false,
            };
            let mut served = Served {
                custody: &mut m.pads,
                apps: BadgeApps::new(circles, board.chip()),
            };
            let resp = serve(&mut served, &mut Answer(yes), &p.request);
            respond(&mut m.down, p.seq, &resp);
            board.log(match (p.chip, yes) {
                (false, true) => "spend allowed",
                (false, false) => "spend refused",
                (true, true) => "signature allowed",
                (true, false) => "signature refused",
            });
            m.pending = None;
            // The prompt replaced whatever was on the screen; the page is back now.
            m.covered = false;
            show_page(board, c, m);
            return MenuAction::Stay;
        }

        if m.covered {
            if gesture != Press::None {
                m.covered = false;
                show_page(board, c, m);
            }
            return MenuAction::Stay;
        }

        if m.confirm_forget {
            match gesture {
                Press::None => {}
                Press::Both => return MenuAction::Forget,
                Press::Left | Press::Right => {
                    m.confirm_forget = false;
                    show_page(board, c, m);
                }
            }
            return MenuAction::Stay;
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
                    match m.pads.spend(&c.circle_id, PSK_LEN) {
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
                Page::Circles => return MenuAction::Choose,
                Page::Forget => {
                    m.confirm_forget = true;
                    show(board, screens::forget_prompt(&c.params.name));
                }
            },
        }
        MenuAction::Stay
    }

    /// Answer or queue every whole request frame in `rx`. Bytes that are not a frame, or a
    /// header longer than any request, are dropped one at a time to resync. A partial header
    /// left by garbage can claim a length that is never filled, so while the front is
    /// incomplete, a whole frame further on wins and the bytes before it are dropped.
    fn menu_decode(board: &mut B, circles: &[Circle], m: &mut Menu<B>) {
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
            let req = Request::decode(&payload).ok();
            let ask = match &req {
                Some(Request::Chip { circle, op }) => {
                    BadgeApps::new(circles, board.chip()).prompt(circle, op)
                }
                Some(r) => match spends(r) {
                    // A spend for a circle the badge does not hold is refused before asking.
                    Some((id, n)) => circles
                        .iter()
                        .find(|c| c.circle_id == id)
                        .map(|c| Some(screens::consent(n, &c.params.name)))
                        .ok_or(ErrorCode::UnknownCircle),
                    None => Ok(None),
                },
                None => Ok(None),
            };
            match ask {
                Ok(Some(screen)) => {
                    show(board, screen);
                    m.pending = Some(Pending {
                        seq,
                        request: payload,
                        chip: matches!(req, Some(Request::Chip { .. })),
                    });
                }
                // Identify, Remaining, Describe, Attest and anything malformed: answered now,
                // never asked.
                Ok(None) => {
                    let mut served = Served {
                        custody: &mut m.pads,
                        apps: BadgeApps::new(circles, board.chip()),
                    };
                    let resp = serve(&mut served, &mut Answer(false), &payload);
                    respond(&mut m.down, seq, &resp);
                }
                // Refused before anyone is asked: a circle the badge does not hold, a request
                // to sign that it cannot describe, one it cannot do yet.
                Err(code) => {
                    board.log(&format!("request refused: {code:?}"));
                    respond(&mut m.down, seq, &Response::Error { code }.encode());
                }
            }
        }
    }
}

/// The circle a request would spend from and the bytes it would spend, or `None` if it
/// spends nothing. A chip request spends no pad; [`BadgeApps::prompt`] decides whether it
/// asks.
fn spends(req: &Request) -> Option<(CircleId, u32)> {
    match req {
        Request::Identify | Request::Remaining { .. } | Request::Chip { .. } => None,
        Request::Psk { circle, .. } => Some((*circle, PSK_LEN)),
        Request::PskFor { handle, .. } => Some((handle.circle_id, PSK_LEN)),
        Request::Spend { circle, len } => Some((*circle, *len)),
        Request::SpendFor { handle, len } => Some((handle.circle_id, *len)),
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
        Page::Circles => vec!["Both to list the circles"],
        Page::Forget => vec!["Both to forget this circle", "the others stay"],
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
