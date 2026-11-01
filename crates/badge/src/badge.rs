//! [`Badge`]: the boot screen, the pile and the ceremony, and the idle menu.

use crate::apps::{BadgeApps, Served};
use crate::board::{Board, Circle, Press};
use crate::chip::{self, ChipRecord, ChipSeat, PhoneSlot, SlotEntry};
use crate::grants::{self, Clock, GrantsRecord};
use crate::pairing::{self, Inbox};
use crate::public::PublicObject;
use crate::screens::{self, Screen};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use cahoots_chip::{ChipError, Curve, KeyId, KeyKind, ObjectPolicy};
use cahoots_core::ceremony::CeremonyResult;
use cahoots_core::frame::{self, Frame, FrameError};
use cahoots_core::pad::Pad;
use cahoots_core::params::CircleParams;
use cahoots_core::root::gen0_id;
use cahoots_core::seat::SeatPublic;
use cahoots_custodian::wire::{
    ChipRequest, ErrorCode, Request, Response, KIND_REQUEST, KIND_RESPONSE, MAX_REQUEST_LEN,
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

/// Why pairing or unpairing a phone failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeError {
    /// The badge has no chip it owns.
    NoChip,
    /// A phone is paired already: unpair it first.
    AlreadyPaired,
    /// No phone is paired.
    NotPaired,
    /// The phone's key in the inbox is not a P-256 key.
    BadInbox,
    /// The chip record could not be read or written.
    Record,
    /// The random source failed its health tests.
    Random,
    /// A chip call failed.
    Chip(ChipError),
    /// A circle's public object does not fit its place in the chip.
    TooLarge,
    /// No pairing is open: [`Badge::pair_open`] first.
    NotOpen,
    /// The inbox holds a request not signed for the open pairing: another phone's, an old
    /// one, or one a tap changed. It was emptied.
    NotForThisBadge,
    /// The chip left the phone slot's id empty and could not fill it: the badge holds the
    /// chip, pin high, until it is turned off and on ([`Board::chip_hold`]).
    ChipHeld,
    /// The bump could not be put together.
    Bump,
    /// The phone is unpaired and its taps end, but some circles' public objects were not
    /// written: they are written at the next boot.
    NotAllUpdated,
    /// An earlier unpair left public objects unwritten: no phone pairs until they are.
    UnpairUnfinished,
}

impl From<chip::PublicError> for BadgeError {
    fn from(e: chip::PublicError) -> Self {
        match e {
            chip::PublicError::Chip(e) => BadgeError::Chip(e),
            chip::PublicError::TooLarge => BadgeError::TooLarge,
        }
    }
}

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
    /// Lists the live grants of this circle; Both on one asks, then revokes it.
    Grants,
    /// Both shows the list of circles, where another circle opens or a new one starts.
    Circles,
    /// Both asks, then forgets this circle and nothing else.
    Forget,
}

impl Page {
    const ALL: [Page; 5] = [
        Page::Words,
        Page::Key,
        Page::Grants,
        Page::Circles,
        Page::Forget,
    ];

    /// The page's place in the ring, from 0.
    fn index(self) -> usize {
        match self {
            Page::Words => 0,
            Page::Key => 1,
            Page::Grants => 2,
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
            Page::Grants => "grants",
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
/// the badge answers the laptop for any circle it holds.
struct Menu<B: Board> {
    pads: Pads<CirclePad<B>>,
    down: B::Link,
    rx: Vec<u8>,
    page: Page,
    /// The laptop on the down link was allowed, with Both, since it was plugged in. The
    /// first request after a plug-in asks; unplugging forgets the answer.
    connected: bool,
    /// The request that arrived before the laptop was allowed, held while the badge asks
    /// whether to allow the laptop at all. Answered or refused with that answer.
    connect: Option<Pending>,
    pending: Option<Pending>,
    /// Whether the down link reported a peer on the last step, so a change is logged once.
    peer_seen: bool,
    /// Another screen (the code, or an error) covers the page; any gesture goes back to it.
    covered: bool,
    /// Both on the Forget page asked "forget this circle?": Both forgets, Left or Right goes
    /// back.
    confirm_forget: bool,
    /// The live grants of this circle, read when the Grants page is entered, after a revoke
    /// and when a cover over the page goes, and the one marked.
    grant_entries: Vec<grants::PageEntry>,
    grant_index: usize,
    /// Both on a grant asked "revoke this grant?": Both revokes, Left or Right goes back.
    confirm_revoke: bool,
    /// The nonce a proof under a grant must be made over: drawn when the menu opens, and
    /// drawn again after every signature under a grant.
    /// `None` while the random source fails: no nonce is given and no use accepted until a
    /// draw succeeds.
    nonce: Option<[u8; 16]>,
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
    /// A note covers the list of circles: any gesture goes back to it, entry `index` marked.
    Note { index: usize },
    /// A pairing is open, waiting for the phone's tap: Both reads the inbox, Left or Right
    /// closes the pairing.
    PairWait { index: usize },
    /// "pair with {name}?" and the four words: Both pairs the phone that wrote the inbox,
    /// Left or Right declines and empties the inbox.
    PairAsk { inbox: Inbox, index: usize },
    /// The paired phone's page: Both unpairs it, Left or Right goes back to the list.
    Phone { index: usize },
}

/// The chooser's entry that starts a pass for one more circle.
const NEW_CIRCLE: &str = "new circle";
/// The chooser's entry that pairs a phone, on a badge whose chip it owns.
const PAIR_PHONE: &str = "pair a phone";
/// The chooser's entry for the paired phone, while one is paired.
const PHONE: &str = "phone";

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
    /// The last contact card the holder signed for each circle, as signed. Kept in memory
    /// only, for a card page to show: a restart forgets it, and the laptop keeps its own copy.
    cards: Vec<(CircleId, Vec<u8>)>,
    /// The badge's best time, for grants: the stored time from boot, raised by what laptops
    /// say, counted on with the uptime.
    clock: Clock,
    /// The nonce and the ephemeral key of the open pairing, while `pair a phone` is open.
    /// Never stored: the key lives in RAM and is zeroized when the pairing closes.
    pairing: Option<([u8; 16], pairing::PairingKey)>,
    /// Whether the badge can pair a phone, and the paired phone's name: read from the chip
    /// record when first needed, and again after a boot, a keep, a pairing or an unpairing.
    phone_state: Option<(bool, Option<String>)>,
    /// The badge held its chip until a restart ([`Board::chip_hold`]).
    chip_held: bool,
}

/// What [`Badge::own_chip`] found at boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owned {
    /// Already this badge's, or nothing to do.
    Kept,
    /// Wiped and set up anew.
    Upgraded,
    /// Not this badge's to own, for good: left unused.
    Refused,
    /// Not usable this boot: left unused, tried again at the next.
    NotNow,
}

/// What the badge says when it will never use its chip: a tap touched it before the badge
/// owned it, or its setup failed after the owner object, which nobody may delete, was made.
const REFUSED: &str =
    "the chip was touched or could not be set up. This chip cannot be used. Circles on this badge keep software keys.";
/// What the badge says when it holds its chip until a restart.
const RESTART: &str = "the chip needs a restart. Turn the badge off and on.";
/// What the badge says when it cannot use its chip this time.
const NOT_NOW: &str =
    "the chip could not be set up. Try turning the badge off and on. Circles made now keep software keys.";

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
            cards: Vec::new(),
            clock: Clock::default(),
            pairing: None,
            phone_state: None,
            chip_held: false,
        }
    }

    /// The last contact card signed on this badge for `circle` since it started, if the badge
    /// still holds that circle.
    pub fn signed_card(&self, circle: &CircleId) -> Option<&[u8]> {
        if !self.circles.iter().any(|c| c.circle_id == *circle) {
            return None;
        }
        self.cards
            .iter()
            .find(|(id, _)| id == circle)
            .map(|(_, card)| card.as_slice())
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

    /// The chip record of a chip this badge owns, or why there is none to pair with.
    fn owned_record(&mut self) -> Result<ChipRecord, BadgeError> {
        if !self.board.has_chip() {
            return Err(BadgeError::NoChip);
        }
        match self.board.chip_record_load() {
            Ok(Some(r)) if r.is_current() => Ok(r),
            Ok(_) => Err(BadgeError::NoChip),
            Err(_) => Err(BadgeError::Record),
        }
    }

    /// Whether the badge can pair a phone, and the paired phone's name, from the cache.
    fn phone_state(&mut self) -> (bool, Option<String>) {
        if self.phone_state.is_none() {
            self.phone_state = Some(match self.owned_record() {
                Ok(r) => (true, r.phone.map(|p| p.name)),
                Err(_) => (false, None),
            });
        }
        self.phone_state.clone().unwrap_or((false, None))
    }

    /// The name of the phone paired to this badge, if one is.
    pub fn paired_phone(&mut self) -> Option<String> {
        self.phone_state().1
    }

    /// The circles whose keys are in the chip, with their slots: the ones a paired phone
    /// reaches.
    fn chip_circles(&self) -> Vec<(u8, Circle)> {
        self.circles
            .iter()
            .filter_map(|c| c.chip_slot.map(|s| (s, c.clone())))
            .collect()
    }

    /// Circle `c`'s public object as this badge writes it: the manifest, the last card signed
    /// here, and what is given.
    fn public_object(
        &self,
        record: &ChipRecord,
        c: &Circle,
        generation: [u8; 32],
        grant: Option<Vec<u8>>,
        pending_bumps: Vec<Vec<u8>>,
        phone_public: Option<[u8; 33]>,
    ) -> PublicObject {
        PublicObject {
            circle_id: c.circle_id,
            relay: String::new(),
            manifest: c.manifest.clone(),
            card: self
                .cards
                .iter()
                .find(|(id, _)| *id == c.circle_id)
                .map(|(_, card)| card.clone()),
            grant,
            generation,
            pending_bumps,
            badge_nonce: record.badge_nonce,
            phone_public,
        }
    }

    /// The circle's current chat generation, as the chip record keeps it.
    fn generation_of(record: &ChipRecord, slot: u8, c: &Circle) -> [u8; 32] {
        record
            .entry(slot)
            .map_or_else(|| gen0_id(&c.circle_id), SlotEntry::current_generation)
    }

    /// The chip left the phone slot's id empty: keep the pin high until the badge is turned
    /// off and on, so no tap can fill it, and let the next boot make it again.
    fn hold_chip(&mut self, e: ChipError) -> BadgeError {
        self.board.log(&format!(
            "chip: the phone slot could not be made ({e:?}), held until the next boot"
        ));
        // Zero means no pairing is open: say so while the chip still answers.
        self.close_pairing();
        self.board.chip_hold();
        self.chip_held = true;
        self.phone_state = None;
        BadgeError::ChipHeld
    }

    /// Open a pairing: draw a fresh nonce, empty the inbox, and write the nonce to the pairing
    /// object for the phone's tap to read. The nonce stays in memory until
    /// [`Badge::pair_close`], [`Badge::pair_accept`] or [`Badge::pair_decline`]. An inbox
    /// made by earlier firmware, too short for a signed request, is made again first, with the
    /// record flagged while its id is empty. `AlreadyPaired` while a phone is paired.
    pub fn pair_open(&mut self) -> Result<[u8; 16], BadgeError> {
        let r = self.open_pairing();
        self.board.chip_end();
        match &r {
            Ok(_) => self.board.log("pairing: open, waiting for a phone's tap"),
            Err(e) => self.board.log(&format!("pairing: not opened: {e:?}")),
        }
        r
    }

    fn open_pairing(&mut self) -> Result<[u8; 16], BadgeError> {
        let mut record = self.owned_record()?;
        if record.phone.is_some() {
            return Err(BadgeError::AlreadyPaired);
        }
        let nonce = self.draw::<16>()?;
        let key = loop {
            let scalar = Zeroizing::new(self.draw::<32>()?);
            if let Some(k) = pairing::PairingKey::from_scalar(&scalar) {
                break k;
            }
        };
        let chip = self.board.chip_begin().ok_or(BadgeError::NoChip)?;
        if chip::inbox_short(chip).map_err(BadgeError::Chip)? {
            record.generation = chip::REMAKE_GENERATION;
            self.board
                .chip_record_store(&record)
                .map_err(|_| BadgeError::Record)?;
            let chip = self.board.chip_begin().ok_or(BadgeError::NoChip)?;
            if let Err(e) = chip::remake_inbox(chip) {
                return Err(self.hold_chip(e));
            }
            record.generation = chip::GENERATION;
            if self.board.chip_record_store(&record).is_err() {
                self.board.log("chip record: write failed, the next boot makes the inbox again");
            }
            self.board.log("pairing: the inbox was made again at its new size");
        }
        let chip = self.board.chip_begin().ok_or(BadgeError::NoChip)?;
        // A request from before this pairing is not for it.
        chip::clear_inbox(chip).map_err(BadgeError::Chip)?;
        chip::write_pairing(chip, &nonce, &key.public()).map_err(BadgeError::Chip)?;
        self.pairing = Some((nonce, key));
        Ok(nonce)
    }

    /// Close the open pairing: forget the nonce and write a zero nonce over the pairing
    /// object.
    pub fn pair_close(&mut self) {
        self.close_pairing();
        self.board.chip_end();
    }

    fn close_pairing(&mut self) {
        if self.pairing.take().is_none() {
            return;
        }
        if let Some(chip) = self.board.chip_begin() {
            if let Err(e) = chip::close_pairing(chip) {
                self.board.log(&format!("pairing: the pairing object was not closed: {e:?}"));
            }
        }
    }

    /// What a phone wrote in the inbox for the open pairing: `Ok(None)` while the inbox is
    /// empty, `NotForThisBadge` for a request not signed by the phone's key over this
    /// pairing's nonce and key, its sealed AES key and its name, or whose seal does not open
    /// with this pairing's key; such a request is emptied. Read in the owner session, which
    /// alone may read the inbox.
    pub fn pair_read_inbox(&mut self) -> Result<Option<Inbox>, BadgeError> {
        let r = self.read_inbox();
        self.board.chip_end();
        r
    }

    fn read_inbox(&mut self) -> Result<Option<Inbox>, BadgeError> {
        if self.pairing.is_none() {
            return Err(BadgeError::NotOpen);
        }
        self.owned_record()?;
        let chip = self.board.chip_begin().ok_or(BadgeError::NoChip)?;
        let bytes = chip.read_binary(KeyId::INBOX).map_err(BadgeError::Chip)?;
        if bytes.iter().all(|&b| b == 0) {
            return Ok(None);
        }
        match Inbox::decode(&bytes).filter(|inbox| self.open_inbox(inbox).is_some()) {
            Some(inbox) => Ok(Some(inbox)),
            None => {
                let chip = self.board.chip_begin().ok_or(BadgeError::NoChip)?;
                let cleared = chip::clear_inbox(chip);
                self.board.log("pairing: the inbox holds a request not for this pairing");
                if let Err(e) = cleared {
                    self.board.log(&format!("pairing: the inbox was not emptied: {e:?}"));
                }
                Err(BadgeError::NotForThisBadge)
            }
        }
    }

    /// The phone's AES key from `inbox`, if it is signed for the open pairing and its seal
    /// opens with the pairing's key.
    fn open_inbox(&self, inbox: &Inbox) -> Option<Zeroizing<[u8; 16]>> {
        let (nonce, key) = self.pairing.as_ref()?;
        inbox.open(nonce, key)
    }

    /// The four words for `inbox` in the open pairing, as the phone that wrote it shows them.
    /// `None` if no pairing is open or the inbox is not for it.
    pub fn pair_words(&self, inbox: &Inbox) -> Option<[&'static str; 4]> {
        let (nonce, key) = self.pairing.as_ref()?;
        let aes = inbox.open(nonce, key)?;
        Some(pairing::words(nonce, &key.public(), &inbox.public, &aes))
    }

    /// The owner said no: empty the inbox and close the pairing.
    pub fn pair_decline(&mut self) {
        if let Some(chip) = self.board.chip_begin() {
            if let Err(e) = chip::clear_inbox(chip) {
                self.board.log(&format!("pairing: the inbox was not emptied: {e:?}"));
            }
        }
        self.close_pairing();
        self.board.chip_end();
        self.board.log("pairing: declined");
    }

    /// Pair the phone that wrote `inbox` for the open pairing, in one owner conversation:
    /// check its signature again, sign a device grant for its key with each circle's seat
    /// key, write each circle's public object with the grant, the phone's key and the current
    /// chat generation, put the phone's AES key in the phone slot, record the phone, empty the
    /// inbox and close the pairing. With no circles in the chip the phone pairs for the
    /// circles kept later, which carry a grant for it.
    ///
    /// The slot is swapped with the record flagged ([`chip::REMAKE_GENERATION`]) before the
    /// delete and set back ([`chip::GENERATION`]) once the phone's key is in, so a
    /// conversation cut off between them leaves the next boot to make the slot again as a
    /// placeholder. A failure before the slot holds the phone's key puts back the public
    /// objects it wrote; one that leaves the slot's id empty holds the chip (`ChipHeld`).
    /// `AlreadyPaired` while a phone is paired: one phone per badge.
    pub fn pair_accept(&mut self, inbox: Inbox) -> Result<(), BadgeError> {
        let r = match (&self.pairing, self.open_inbox(&inbox)) {
            (None, _) => Err(BadgeError::NotOpen),
            (Some(_), None) => Err(BadgeError::NotForThisBadge),
            (Some(_), Some(key)) => self.pair(&inbox, &key),
        };
        if r != Err(BadgeError::ChipHeld) {
            self.close_pairing();
        }
        self.board.chip_end();
        self.phone_state = None;
        match r {
            Ok(n) => self.board.log(&format!(
                "pairing: paired with {}, {n} circles",
                screens::phone_name(&inbox.name)
            )),
            Err(e) => self.board.log(&format!("pairing: failed: {e:?}")),
        }
        r.map(|_| ())
    }

    fn pair(&mut self, inbox: &Inbox, key: &[u8; 16]) -> Result<usize, BadgeError> {
        let mut record = self.owned_record()?;
        if record.phone.is_some() {
            return Err(BadgeError::AlreadyPaired);
        }
        let circles = self.chip_circles();
        if record.pending_unpair.is_some() {
            let all: Vec<Circle> = circles.iter().map(|(_, c)| c.clone()).collect();
            if self.finish_unpair(&mut record, &all) > 0 {
                return Err(BadgeError::UnpairUnfinished);
            }
        }
        let grantee = SeatPublic::parse(&inbox.public).map_err(|_| BadgeError::BadInbox)?;
        let mut nonces = Vec::new();
        for _ in &circles {
            nonces.push(self.draw::<16>()?);
        }
        // Everything is signed before anything is changed.
        let mut writes: Vec<(u8, Option<Vec<u8>>, PublicObject)> = Vec::new();
        for ((slot, c), nonce) in circles.iter().zip(nonces) {
            let claims =
                cahoots_objects::grant::issue_device_grant(&c.circle_id, c.seat, grantee, nonce);
            let chip = self.board.chip_begin().ok_or(BadgeError::NoChip)?;
            let mut signer = ChipSeat::new(chip, KeyId::new(KeyKind::Seat, *slot));
            let grant = cahoots_objects::grant::sign_device_grant(&claims, &mut signer, c.seat);
            if let Some(e) = signer.error {
                return Err(BadgeError::Chip(e));
            }
            let old = chip.read_binary(chip::public_id(*slot)).ok();
            // Bumps signed at earlier unpairs stay, in order, for this phone to post.
            let pending = old
                .as_deref()
                .and_then(|b| PublicObject::decode(b).ok())
                .map(|o| o.pending_bumps)
                .unwrap_or_default();
            let generation = Self::generation_of(&record, *slot, c);
            let object =
                self.public_object(&record, c, generation, Some(grant), pending, Some(inbox.public));
            writes.push((*slot, old, object));
        }
        // The public objects first: no tap reads them before this conversation ends.
        for (i, (slot, _, object)) in writes.iter().enumerate() {
            let chip = self.board.chip_begin().ok_or(BadgeError::NoChip)?;
            if let Err(e) = chip::write_public(chip, *slot, object) {
                self.restore_publics(&writes[..=i]);
                return Err(e.into());
            }
        }
        // The slot, flagged in the record while its id is empty.
        record.generation = chip::REMAKE_GENERATION;
        if self.board.chip_record_store(&record).is_err() {
            self.restore_publics(&writes);
            return Err(BadgeError::Record);
        }
        let chip = self.board.chip_begin().ok_or(BadgeError::NoChip)?;
        match chip::replace_slot(chip, Some(key)) {
            Ok(()) => {}
            Err(chip::SlotFailure::Empty(e)) => {
                self.restore_publics(&writes);
                return Err(self.hold_chip(e));
            }
            Err(chip::SlotFailure::Unchanged(e) | chip::SlotFailure::Placeholder(e)) => {
                // The slot holds a placeholder: nothing is paired.
                self.restore_publics(&writes);
                record.generation = chip::GENERATION;
                if self.board.chip_record_store(&record).is_err() {
                    self.board.log("chip record: write failed, the next boot makes the slot again");
                }
                return Err(BadgeError::Chip(e));
            }
        }
        record.generation = chip::GENERATION;
        // This phone may post the pending bumps: the next unpair must not replace them.
        for (slot, _) in &circles {
            record.set_paired_since_bump(*slot, true);
        }
        record.phone = Some(PhoneSlot {
            name: inbox.name.clone(),
            public: inbox.public,
        });
        if self.board.chip_record_store(&record).is_err() {
            // The stored record is still flagged, and the next boot makes the slot again: put
            // the placeholder back now, so the phone is not half paired until then.
            self.restore_publics(&writes);
            let chip = self.board.chip_begin().ok_or(BadgeError::NoChip)?;
            if let Err(chip::SlotFailure::Empty(e)) = chip::replace_slot(chip, None) {
                return Err(self.hold_chip(e));
            }
            return Err(BadgeError::Record);
        }
        if let Some(chip) = self.board.chip_begin() {
            if let Err(e) = chip::clear_inbox(chip) {
                self.board.log(&format!("pairing: the inbox was not emptied: {e:?}"));
            }
        }
        Ok(writes.len())
    }

    /// Put back public objects as they were before a pairing that failed: the old bytes, or
    /// no object where there was none.
    fn restore_publics(&mut self, writes: &[(u8, Option<Vec<u8>>, PublicObject)]) {
        for (slot, old, _) in writes {
            let Some(chip) = self.board.chip_begin() else {
                return;
            };
            let id = chip::public_id(*slot);
            let put = match old {
                Some(bytes) => chip.rewrite_binary(id, bytes),
                None => chip.delete(id),
            };
            if let Err(e) = put {
                self.board
                    .log(&format!("pairing: slot {slot} public object not put back: {e:?}"));
            }
        }
    }

    /// Unpair the phone, in one owner conversation. For each circle it draws a new chat
    /// generation id and signs a bump to it with the seat key, and stores the bumps in the
    /// chip record as an unpair under way ([`ChipRecord::pending_unpair`]) with the record
    /// flagged, before it touches anything. Then it puts a placeholder in the phone slot,
    /// which ends the phone's taps, and records no phone. Then it appends each bump to its
    /// circle's public object, with no grant and no phone key, and the chip record takes each
    /// new generation as its public object is written. A device posts the bumps in order the
    /// next time one taps; until then the circle chats in the old generation.
    ///
    /// A conversation cut off anywhere is finished at the next boot: it makes the slot again
    /// and writes the public objects from the stored bumps. `NotAllUpdated` when some public
    /// objects were not written: the phone is unpaired and the next boot writes them.
    pub fn unpair(&mut self) -> Result<(), BadgeError> {
        let r = self.unpair_phone();
        self.board.chip_end();
        self.phone_state = None;
        match &r {
            Ok(name) => self
                .board
                .log(&format!("pairing: unpaired {}", screens::phone_name(name))),
            Err(e) => self.board.log(&format!("pairing: unpairing failed: {e:?}")),
        }
        r.map(|_| ())
    }

    fn unpair_phone(&mut self) -> Result<String, BadgeError> {
        let mut record = self.owned_record()?;
        let phone = record.phone.clone().ok_or(BadgeError::NotPaired)?;
        let circles = self.chip_circles();
        let all: Vec<Circle> = circles.iter().map(|(_, c)| c.clone()).collect();
        if record.pending_unpair.is_some() {
            self.finish_unpair(&mut record, &all);
        }
        let mut ids = Vec::new();
        for _ in &circles {
            ids.push(self.draw::<32>()?);
        }
        let mut bumps = Vec::new();
        for ((slot, c), to) in circles.iter().zip(ids) {
            let from = Self::generation_of(&record, *slot, c);
            let chip = self.board.chip_begin().ok_or(BadgeError::NoChip)?;
            let mut list = chip::read_public(chip, *slot)
                .ok()
                .flatten()
                .map(|o| o.pending_bumps)
                .unwrap_or_default();
            match list.last().map(|b| bump_ends(b)) {
                // The last bump W to X moves to the generation the record has: replace it with
                // W to the new one if no phone was paired since, which nobody can have posted.
                Some(Some((w, x))) if x == from && !record.paired_since_bump(*slot) => {
                    list.pop();
                    list.push(self.sign_bump_in_chip(*slot, c, &w, &to)?);
                }
                // A phone paired since may have posted it: add X to the new one after it.
                Some(Some((_, x))) if x == from => {
                    list.push(self.sign_bump_in_chip(*slot, c, &from, &to)?);
                }
                // No bump, or one that does not end where the record is: start again.
                _ => {
                    list.clear();
                    list.push(self.sign_bump_in_chip(*slot, c, &from, &to)?);
                }
            }
            // At the cap the two oldest, A to B and B to C, become one, A to C. Members already
            // in B, who followed A to B and not B to C, are stranded by that: it needs four
            // cycles of pairing a phone that never taps and unpairing it, with nobody posting
            // in between. A reset by any member with the root (a bump of its own) recovers
            // them.
            while list.len() > crate::public::MAX_PENDING_BUMPS {
                let (a, _) = bump_ends(&list[0]).ok_or(BadgeError::Bump)?;
                let (_, cc) = bump_ends(&list[1]).ok_or(BadgeError::Bump)?;
                let joined = self.sign_bump_in_chip(*slot, c, &a, &cc)?;
                list.splice(0..2, [joined]);
            }
            record.set_paired_since_bump(*slot, false);
            bumps.push(chip::PendingBump {
                slot: *slot,
                to,
                bumps: list.into_iter().map(Into::into).collect(),
            });
        }
        // The unpair is stored before anything is touched, flagged for the slot.
        record.generation = chip::REMAKE_GENERATION;
        record.pending_unpair = (!bumps.is_empty()).then_some(bumps);
        self.board
            .chip_record_store(&record)
            .map_err(|_| BadgeError::Record)?;
        // The slot first: the phone's taps end here.
        let chip = self.board.chip_begin().ok_or(BadgeError::NoChip)?;
        match chip::replace_slot(chip, None) {
            Ok(()) => {}
            Err(chip::SlotFailure::Unchanged(e)) => {
                // The phone's key is still in the slot: nothing changed.
                record.generation = chip::GENERATION;
                record.pending_unpair = None;
                if self.board.chip_record_store(&record).is_err() {
                    self.board.log("chip record: write failed");
                }
                return Err(BadgeError::Chip(e));
            }
            Err(chip::SlotFailure::Empty(e) | chip::SlotFailure::Placeholder(e)) => {
                return Err(self.hold_chip(e));
            }
        }
        record.generation = chip::GENERATION;
        record.phone = None;
        let stored = self.board.chip_record_store(&record).is_ok();
        let left = self.finish_unpair(&mut record, &all);
        if !stored {
            return Err(BadgeError::Record);
        }
        if left > 0 {
            return Err(BadgeError::NotAllUpdated);
        }
        Ok(phone.name)
    }

    /// Write the bumps of an unpair under way into their circles' public objects: each bump
    /// appended to the object's list unless it is there already, the generation moved to the
    /// bump's, no grant and no phone key. A circle whose object is written takes the new
    /// generation in the record. The bumps not written stay in the record for the next boot;
    /// the record is stored. Returns how many stayed.
    fn finish_unpair(&mut self, record: &mut ChipRecord, circles: &[Circle]) -> usize {
        let Some(pending) = record.pending_unpair.take() else {
            return 0;
        };
        let mut left = Vec::new();
        for b in pending {
            let Some(chip) = self.board.chip_begin() else {
                left.push(b);
                continue;
            };
            let old = chip::read_public(chip, b.slot).ok().flatten();
            let circle = circles.iter().find(|c| c.chip_slot == Some(b.slot));
            let object = match (old, circle) {
                (Some(mut o), _) => {
                    o.pending_bumps = b.bumps.iter().map(|x| x.to_vec()).collect();
                    o.generation = b.to;
                    // A phone paired since keeps its grant and its key here.
                    if record.phone.is_none() {
                        o.grant = None;
                        o.phone_public = None;
                    }
                    o
                }
                (None, Some(c)) => {
                    let phone = record.phone.as_ref().map(|p| p.public);
                    let list = b.bumps.iter().map(|x| x.to_vec()).collect();
                    self.public_object(record, c, b.to, None, list, phone)
                }
                (None, None) => {
                    self.board.log(&format!(
                        "pairing: slot {} holds no circle and no public object, its bump is dropped",
                        b.slot
                    ));
                    continue;
                }
            };
            let Some(chip) = self.board.chip_begin() else {
                left.push(b);
                continue;
            };
            match chip::write_public(chip, b.slot, &object) {
                Ok(()) => {
                    if let Some(e) = record.entry_mut(b.slot) {
                        e.chat_generation = Some(b.to);
                    }
                }
                Err(e) => {
                    self.board.log(&format!(
                        "pairing: slot {} public object not written, kept for the next boot: {e:?}",
                        b.slot
                    ));
                    left.push(b);
                }
            }
        }
        let n = left.len();
        record.pending_unpair = (!left.is_empty()).then_some(left);
        if self.board.chip_record_store(record).is_err() {
            self.board.log("chip record: write failed");
        }
        n
    }

    /// A bump from `from` to `to` in circle `c`, signed with its seat key in slot `slot`.
    fn sign_bump_in_chip(
        &mut self,
        slot: u8,
        c: &Circle,
        from: &[u8; 32],
        to: &[u8; 32],
    ) -> Result<Vec<u8>, BadgeError> {
        let chip = self.board.chip_begin().ok_or(BadgeError::NoChip)?;
        let mut signer = ChipSeat::new(chip, KeyId::new(KeyKind::Seat, slot));
        let payload = cahoots_chat::sign_bump(&mut signer, c.seat, &c.circle_id, from, to);
        if let Some(e) = signer.error {
            return Err(BadgeError::Chip(e));
        }
        payload.map_err(|_| BadgeError::Bump)
    }

    /// At boot: finish an unpair a cut-off conversation left under way.
    fn finish_unpair_at_boot(&mut self, circles: &[Circle]) {
        let Ok(mut record) = self.owned_record() else {
            return;
        };
        if record.pending_unpair.is_none() {
            return;
        }
        self.board.log("pairing: finishing an unpair that was cut off");
        let left = self.finish_unpair(&mut record, circles);
        if left > 0 {
            self.board
                .log(&format!("pairing: {left} public objects still not written"));
        }
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
        let owned = self.own_chip();
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
        self.finish_unpair_at_boot(&circles);
        self.phone_state = None;
        self.pairing = None;
        self.chip_held = false;
        // No pairing is open after a boot: a nonce left from before goes.
        if let Some(chip) = self.board.chip_begin() {
            match chip::close_pairing(chip) {
                Ok(()) | Err(cahoots_chip::ChipError::NoSuchKey) => {}
                Err(e) => self.board.log(&format!("pairing: the pairing object was not closed: {e:?}")),
            }
        }
        let held: Vec<cahoots_core::circle_id::CircleId> = circles.iter().map(|c| c.circle_id).collect();
        for l in grants::sweep(&mut self.board, &held) {
            self.board.log(&l);
        }
        self.clock = grants::boot_clock(&mut self.board);
        // The chip rests through the self-test screen.
        self.board.chip_end();
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
        self.board.chip_end();
        let said = match owned {
            Owned::Upgraded => Some("updated for phones. Make a new circle."),
            Owned::Refused => Some(REFUSED),
            Owned::NotNow => Some(NOT_NOW),
            Owned::Kept => None,
        };
        if let Some(said) = said {
            show(&mut self.board, screens::status(&[said]));
            for _ in 0..SELF_TEST_MS / POLL_MS {
                if self.board.press() == Press::Both {
                    break;
                }
                self.board.delay_ms(POLL_MS);
            }
        }
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
        let stayed = self.forget_chip();
        // The grants went with the chip's counters and the circles that issued them.
        if self.board.grants_store(&GrantsRecord::default()).is_err() {
            self.board.log("grants record: write failed");
        }
        if !ok {
            return left;
        }
        self.board.log("A held at boot: every circle forgotten");
        let what = if circles.len() == 1 {
            "circle forgotten"
        } else {
            "circles forgotten"
        };
        if stayed == 0 {
            show(&mut self.board, screens::status(&[what, "release A"]));
        } else {
            let stayed = left_line(stayed);
            show(&mut self.board, screens::status(&[what, &stayed, "release A"]));
        }
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
            if let Some(chip) = self.board.chip_begin() {
                let left = chip::delete_slot(chip, slot).map_err(|e| format!("chip: {e:?}"))?;
                if !left.is_empty() {
                    // The record keeps the slot, marked stuck, so it is never chosen again
                    // and the next boot tries it again.
                    self.log_left(&left);
                    if let Ok(Some(mut record)) = self.board.chip_record_load() {
                        record.set_stuck(slot, true);
                        if self.board.chip_record_store(&record).is_err() {
                            self.board.log("chip record: write failed");
                        }
                    }
                    return Err(left_line(left.len()));
                }
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
        let step = match self.advance(state) {
            Ok((next, step)) => {
                self.state = next;
                step
            }
            Err(e) => {
                self.drop_chip_seat();
                Step::Failed(e)
            }
        };
        // The ceremony signs with the chip as it goes, so the chip stays up through it and
        // rests after; every other step ends its conversation.
        if !matches!(self.state, State::Ceremony { .. }) {
            self.board.chip_end();
        }
        step
    }

    /// At boot, before anything else touches the chip: make sure the badge owns it.
    ///
    /// A chip record of this firmware's generation, on a chip whose owner object opens with
    /// the board's secret, means the chip was set up by this badge: only a missing phone slot
    /// or inbox is made again. A refused record means an earlier boot found the chip touched:
    /// it is refused again at once, and nothing else is touched. Anything else means the
    /// chip's objects were made the old way, reachable by any session, or by a tap: every
    /// object but the owner object is deleted, the circles whose keys were in the chip and
    /// the grants with them, and the chip is set up anew in one owner conversation
    /// ([`chip::first_boot`]), and the badge says so once.
    ///
    /// Every way out but those two refuses the chip for this boot ([`Badge::refuse_chip`]):
    /// the board stops handing it out, so its pin rests low with nothing made that a tap could
    /// claim and nothing in it used. Where the chip is in a state no later boot can trust (an
    /// object a tap made stays at the phone slot, inbox or issue counter id, or the setup
    /// failed after the owner object was made) a refused record is stored, so the wipe runs
    /// once and later boots refuse at once. Otherwise no record is stored and the next boot
    /// tries again. Circles whose keys are in software are never touched here.
    ///
    /// An unreadable record changes nothing: it is not the same as no record.
    fn own_chip(&mut self) -> Owned {
        if !self.board.has_chip() {
            return Owned::Kept;
        }
        let stored = match self.board.chip_record_load() {
            Ok(r) => r,
            Err(_) => {
                self.board
                    .log("chip record: unreadable, chip objects left as they are");
                return Owned::Kept;
            }
        };
        if stored.as_ref().is_some_and(|r| r.is_refused()) {
            return self.refuse_chip("chip: refused at an earlier boot", true);
        }
        let Some(secret) = self.board.owner_secret() else {
            return self.refuse_chip("chip: the board gives no owner secret", false);
        };
        // The nonce before anything is deleted: a random source that fails leaves all as it is.
        let mut nonce = [0u8; 16];
        let drawn = self.board.entropy().fill(&mut nonce).is_ok();
        let Some(chip) = self.board.chip_begin() else {
            return self.refuse_chip("chip: no answer", false);
        };
        let has_owner = match chip.exists(KeyId::OWNER) {
            Ok(b) => b,
            Err(e) => {
                return self.refuse_chip(&format!("chip: could not look for the owner object: {e:?}"), false);
            }
        };
        let opens = has_owner && chip.open_owner(&secret).is_ok();
        if stored.as_ref().is_some_and(|r| r.is_current()) && opens {
            let remake = stored.as_ref().is_some_and(|r| r.needs_remake());
            return match chip::ensure_owned(chip, &secret, remake) {
                Ok(chip::Ensured::Kept) => Owned::Kept,
                Ok(chip::Ensured::Remade) => {
                    self.board
                        .log("chip: the phone slot, the inbox and the grant counter were made again");
                    if let Some(mut record) = stored {
                        // A remade slot holds no phone's key.
                        record.generation = chip::GENERATION;
                        record.phone = None;
                        if self.board.chip_record_store(&record).is_err() {
                            self.board.log("chip record: write failed");
                        }
                    }
                    Owned::Kept
                }
                Err(chip::EnsureError::Before(e)) => {
                    self.refuse_chip(&format!("chip: the owner session did not open: {e:?}"), false)
                }
                Err(chip::EnsureError::During(e)) => self.refuse_chip(
                    &format!("chip: the phone slot, inbox or grant counter not made again: {e:?}"),
                    true,
                ),
            };
        }
        if has_owner && !opens {
            return self.refuse_chip(
                "upgrade: the chip's owner object does not open with this badge's secret",
                false,
            );
        }
        if !drawn {
            return self.refuse_chip("upgrade: the random source failed, chip left as it is", false);
        }
        // Everything but the owner object goes, the phone slot, the inbox and the issue
        // counter included, whoever made them.
        let (n, left) = match chip::delete_for_upgrade(chip) {
            Ok(w) => w,
            Err(e) => {
                return self.refuse_chip(&format!("upgrade: the chip's objects could not be listed: {e:?}"), false);
            }
        };
        let stuck = left.iter().any(|(id, _)| chip::ANTENNA_IDS.contains(id));
        let made = if stuck { None } else { Some(chip::first_boot(chip, &secret, nonce)) };
        // A setup that failed before the owner object was made can be tried again; one that
        // failed after cannot, since nobody may delete the owner object.
        let owner_made = matches!(made, Some(Err(_))) && chip.exists(KeyId::OWNER) != Ok(false);
        self.board.log(&format!(
            "upgrade: {n} chip objects made before the badge owned its chip deleted"
        ));
        for (id, e) in &left {
            self.board
                .log(&format!("upgrade: chip object {:08x} not deleted: {e:?}", id.0));
        }
        // The circles whose keys were in the chip, and the grants, go: those keys were
        // reachable by any session. Circles with software keys stay.
        let circles = self.board.load_circles();
        let (gone, kept): (Vec<Circle>, Vec<Circle>) =
            circles.into_iter().partition(|c| c.chip_slot.is_some());
        for c in &gone {
            if self.board.forget(&c.circle_id, c.params.pad_len).is_err() {
                self.board.log("upgrade: a circle was not forgotten");
            }
        }
        if !gone.is_empty() && self.board.store_circles(&kept).is_err() {
            self.board.log("upgrade: circle records failed");
        }
        if self.board.grants_store(&GrantsRecord::default()).is_err() {
            self.board.log("grants record: write failed");
        }
        let record = match made {
            None => {
                return self.refuse_chip(
                    "upgrade: an object a tap made stays at the phone slot, inbox or grant counter id",
                    true,
                )
            }
            Some(Err(e)) => {
                return self.refuse_chip(&format!("upgrade: chip not set up: {e:?}"), owner_made);
            }
            Some(Ok(record)) => record,
        };
        if self.board.chip_record_store(&record).is_err() {
            return self.refuse_chip("chip record: write failed", false);
        }
        self.board.log(
            "upgrade: the chip has its owner, its phone slot, its inbox and its grant counter",
        );
        if !gone.is_empty() || n > 0 || stored.is_some() {
            Owned::Upgraded
        } else {
            Owned::Kept
        }
    }

    /// The chip is not this badge's to use: log why, end the conversation and stop the board
    /// handing the chip out for this boot. With `for_good`, store the refused record, so later
    /// boots refuse at once.
    fn refuse_chip(&mut self, why: &str, for_good: bool) -> Owned {
        self.board.log(why);
        self.board.chip_end();
        self.board.chip_unusable();
        if for_good {
            self.board.log("chip: not owned, never used");
            if self.board.chip_record_store(&chip::ChipRecord::refused()).is_err() {
                self.board.log("chip record: write failed");
            }
            Owned::Refused
        } else {
            self.board.log("chip: not used this time");
            Owned::NotNow
        }
    }

    /// At boot: delete every chip object in a slot the chip record does not know, and log it.
    /// A record that cannot be read deletes nothing: it is not the same as no record. A slot
    /// a kept circle names is never deleted, whatever the record says.
    fn reconcile_chip(&mut self, keep: Vec<u8>) {
        if !self.board.has_chip() {
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
        // Only a record of this firmware is reconciled: without one the chip was not set up,
        // and its objects are not the badge's to sort.
        let Some(mut record) = stored.clone().filter(|r| r.is_current()) else {
            return;
        };
        let Some(chip) = self.board.chip_begin() else {
            return;
        };
        let mut lines = record.reconcile_keeping(chip, &keep);
        // A slot a wipe marked stuck held an object that could not be deleted: try again, and
        // let the slot go once it is empty. A slot no loaded circle names but not marked stuck
        // is left: its circle's record may only have failed to read this time.
        let mut stuck: Vec<u8> = Vec::new();
        for e in &record.slots {
            if keep.contains(&e.slot) {
                continue;
            }
            if record.is_stuck(e.slot) {
                stuck.push(e.slot);
            } else {
                lines.push(format!("chip: slot {} names no circle that loaded, left as it is", e.slot));
            }
        }
        for slot in record.stuck.clone().unwrap_or_default() {
            if !keep.contains(&slot) && !stuck.contains(&slot) {
                stuck.push(slot);
            }
        }
        for slot in stuck {
            match chip::delete_slot(chip, slot) {
                Ok(left) if left.is_empty() => {
                    record.remove(slot);
                    record.set_stuck(slot, false);
                    lines.push(format!("chip: slot {slot} names no circle, emptied and let go"));
                }
                Ok(left) => lines.push(format!(
                    "chip: slot {slot} names no circle, {} objects in it still cannot be deleted",
                    left.len()
                )),
                Err(e) => lines.push(format!("chip: slot {slot} names no circle, not emptied: {e:?}")),
            }
        }
        for l in &lines {
            self.board.log(l);
        }
        if stored.is_some_and(|s| s != record) && self.board.chip_record_store(&record).is_err() {
            self.board.log("chip record: write failed");
        }
    }

    /// The wipe at boot: every chip object and the chip record go with the circles. Returns
    /// how many objects stayed; the record keeps the slots they are in, so those slots are
    /// never chosen again.
    fn forget_chip(&mut self) -> usize {
        let Some(chip) = self.board.chip_begin() else {
            return 0;
        };
        let left = match chip::delete_all(chip) {
            Ok(left) => left,
            Err(e) => {
                self.board.log(&format!("chip: wipe failed: {e:?}"));
                return 0;
            }
        };
        self.log_left(&left);
        let stayed: Vec<u8> = left.iter().filter_map(|(id, _)| id.circle_slot()).collect();
        // The circles go; the badge's own objects, its nonce and its phone stay.
        match self.board.chip_record_load() {
            Ok(Some(mut record)) => {
                record.slots.retain(|e| stayed.contains(&e.slot));
                for slot in &stayed {
                    record.set_stuck(*slot, true);
                }
                if self.board.chip_record_store(&record).is_err() {
                    self.board.log("chip record: write failed");
                }
            }
            Ok(None) => {}
            Err(_) => self.board.log("chip record: unreadable, left as it is"),
        }
        left.len()
    }

    /// Log each object a wipe could not delete.
    fn log_left(&mut self, left: &chip::Left) {
        for (id, e) in left {
            self.board
                .log(&format!("chip: object {:08x} could not be deleted: {e:?}", id.0));
        }
    }

    /// Make this pass's seat key in the chip, in the chip record's lowest free slot that no
    /// held circle names. Anything left in that slot is deleted first: the record does not
    /// know it, so it is not reused. A slot a circle holds is never chosen.
    fn make_chip_seat(&mut self) -> Result<(u8, SeatPublic), String> {
        let record = self.load_chip_record()?;
        let kept = self.held_slots();
        let free: Vec<u8> = (0..chip::CIRCLE_SLOTS)
            .filter(|&s| record.entry(s).is_none() && !kept.contains(&s))
            .collect();
        // Anything left in a free slot is deleted first; a slot holding an object that cannot
        // be deleted is passed over.
        let mut chosen = None;
        for s in free {
            let chip = self.board.chip_begin().ok_or("chip: not found")?;
            let left = chip::delete_slot(chip, s).map_err(|e| format!("chip: {e:?}"))?;
            if left.is_empty() {
                chosen = Some(s);
                break;
            }
            self.log_left(&left);
        }
        let slot = chosen.ok_or("chip: every slot is in use")?;
        let chip = self.board.chip_begin().ok_or("chip: not found")?;
        let public = chip
            .generate(KeyId::new(KeyKind::Seat, slot), Curve::P256, &ObjectPolicy::owner_only())
            .map_err(|e| format!("chip: key not made: {e:?}"))?;
        // Set before the parse, so a failure below still deletes the key.
        self.chip_slot = Some(slot);
        let public = SeatPublic::parse(&public).map_err(|_| "chip: bad public key")?;
        self.board
            .log(&format!("seat key made in the chip, slot {slot}"));
        Ok((slot, public))
    }

    /// At the keep: the root and the epoch counter go into the chip beside the seat key, a
    /// hello object for each other seat from the pair key the ceremony gave for it, and the
    /// circle's public object; the chip record learns the slot. With a phone paired, the
    /// public object carries a device grant for it, so the phone finds the new circle at its
    /// next tap.
    fn keep_in_chip(&mut self, slot: u8, result: &CeremonyResult, seat: u8) -> Result<(), String> {
        let root = result.root.as_ref().ok_or("ceremony: no circle root")?;
        let circle_id = result.circle_id;
        let mut record = self.load_chip_record()?;
        let phone = record.phone.clone();
        let nonce = match phone {
            Some(_) => Some(self.draw::<16>().map_err(|_| "trng: failed, no grant nonce")?),
            None => None,
        };
        let chip = self.board.chip_begin().ok_or("chip: not found")?;
        chip.put_hmac_key(KeyId::new(KeyKind::Root, slot), root, &ObjectPolicy::owner_and_slot_mac())
            .map_err(|e| format!("chip: root not stored: {e:?}"))?;
        chip.put_counter(KeyId::new(KeyKind::Epoch, slot), 0, &ObjectPolicy::owner_counter())
            .map_err(|e| format!("chip: counter not stored: {e:?}"))?;
        let mut hellos = 0;
        for (other, key) in result.pair_keys.iter().enumerate() {
            let Some(key) = key else { continue };
            // A circle has at most 8 seats, so `other` fits the hello id's three bits.
            let other = u8::try_from(other).ok().filter(|o| *o < 8).ok_or("ceremony: seat out of range")?;
            chip.put_hmac_key(
                chip::hello_id(slot, other),
                &chip::hello_value(key),
                &ObjectPolicy::owner_and_slot_mac(),
            )
            .map_err(|e| format!("chip: hello object not stored: {e:?}"))?;
            hellos += 1;
        }
        let grant = match (&phone, nonce) {
            (Some(p), Some(nonce)) => {
                let grantee = SeatPublic::parse(&p.public).map_err(|_| "chip record: bad phone key")?;
                let claims = cahoots_objects::grant::issue_device_grant(&circle_id, seat, grantee, nonce);
                let mut signer = ChipSeat::new(chip, KeyId::new(KeyKind::Seat, slot));
                let signed = cahoots_objects::grant::sign_device_grant(&claims, &mut signer, seat);
                if let Some(e) = signer.error {
                    return Err(format!("chip: device grant not signed: {e:?}"));
                }
                Some(signed)
            }
            _ => None,
        };
        let object = PublicObject {
            circle_id,
            relay: String::new(),
            manifest: result.manifest.encode(),
            card: None,
            grant,
            generation: gen0_id(&circle_id),
            pending_bumps: Vec::new(),
            badge_nonce: record.badge_nonce,
            phone_public: phone.map(|p| p.public),
        };
        let chip = self.board.chip_begin().ok_or("chip: not found")?;
        chip::clear_public(chip, slot)
            .map_err(|e| format!("chip: an object at the public object's id was not deleted: {e:?}"))?;
        let written = chip::write_public(chip, slot, &object);
        self.board.log(&format!(
            "chip: slot {slot} kept, seat key for the owner alone, root for the owner and the phone slot"
        ));
        self.board.log(&format!(
            "chip: slot {slot} has {hellos} hello objects for the owner and the phone slot"
        ));
        match written {
            Ok(()) => self.board.log(&format!("chip: slot {slot} public object written")),
            // The circle is kept without one: a phone does not see it, and nothing else needs it.
            Err(chip::PublicError::TooLarge) => self.board.log(&format!(
                "chip: slot {slot} public object too large, a phone will not see this circle"
            )),
            Err(chip::PublicError::Chip(e)) => {
                return Err(format!("chip: public object not written: {e:?}"))
            }
        }
        record.insert(SlotEntry {
            slot,
            circle_id,
            has_seat: true,
            has_root: true,
            has_epoch: true,
            chat_generation: None,
        });
        self.board
            .chip_record_store(&record)
            .map_err(|_| String::from("flash: chip record failed"))
    }

    /// `N` bytes from the board's random source.
    fn draw<const N: usize>(&mut self) -> Result<[u8; N], BadgeError> {
        let mut out = [0u8; N];
        self.board
            .entropy()
            .fill(&mut out)
            .map_err(|_| BadgeError::Random)?;
        Ok(out)
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
        if let Some(chip) = self.board.chip_begin() {
            match chip::delete_slot(chip, slot) {
                Ok(left) => self.log_left(&left),
                Err(e) => self
                    .board
                    .log(&format!("chip: slot {slot} not deleted: {e:?}")),
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
                    let in_chip = self.board.has_chip();
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
                        if pile.is_bridged() {
                            show_status(&mut self.board, "waiting for the host to start");
                        }
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
                        Err(PileError::Bridged) => {
                            show_status(&mut self.board, "the host starts this one")
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
                        let chip = self.board.chip_begin().ok_or("chip: not found")?;
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
                            self.keep_in_chip(slot, &result, seat)?;
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
    /// The current circle's menu opens: a ring of pages, Words, Key, Circles and Forget,
    /// Left and Right to turn the page, Both to select. Both on Key spends 32 bytes and
    /// shows them as a code; Both on Circles shows the list, where Left and Right move, Both
    /// opens that circle's menu, and the last entry, "new circle", starts a pass for one
    /// more while the board has room; Both on Forget asks, and Both again forgets this
    /// circle and nothing else.
    ///
    /// On every page, the badge answers custodian requests from the laptop on the down link
    /// for any circle it holds. The first request after the laptop was plugged in asks
    /// whether to allow that laptop at all, with Both, until it is unplugged; after that,
    /// each spend or signature asks on its own, allowed by Both or refused by Left or
    /// Right, and the rest are answered as they come.
    pub fn idle_step(&mut self) {
        if self.circles.is_empty() {
            return;
        }
        let idle = core::mem::replace(&mut self.idle, Idle::Off);
        self.idle = match idle {
            Idle::Off => {
                self.open_current = false;
                if self.current >= self.circles.len() {
                    self.current = 0;
                }
                self.enter_menu()
            }
            Idle::Choose { index } => self.choose_step(index),
            Idle::NewCircle => self.new_circle_step(),
            Idle::Note { index } => {
                if self.board.press() == Press::None {
                    Idle::Note { index }
                } else {
                    // The note may have changed the list: a phone paired or unpaired.
                    let index = index.min(self.chooser_names().len() - 1);
                    self.show_chooser(index);
                    Idle::Choose { index }
                }
            }
            Idle::PairWait { index } => self.pair_wait_step(index),
            Idle::PairAsk { inbox, index } => self.pair_ask_step(inbox, index),
            Idle::Phone { index } => self.phone_step(index),
            Idle::On(mut menu) => {
                match Self::menu_step(
                    &mut self.board,
                    &self.circles,
                    self.current,
                    &mut menu,
                    &mut self.cards,
                    &mut self.clock,
                ) {
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
        self.board.chip_end();
        self.board.delay_ms(1);
    }

    /// The chooser's entries: each circle's name, a number after a name two circles share,
    /// "new circle" while the board has room for one more, then on a badge that owns its
    /// chip "pair a phone", and "phone" while one is paired.
    fn chooser_names(&mut self) -> Vec<String> {
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
        let (can_pair, phone) = self.phone_state();
        if can_pair {
            names.push(String::from(PAIR_PHONE));
            if phone.is_some() {
                names.push(String::from(PHONE));
            }
        }
        names
    }

    /// A note over the list of circles, until the next gesture.
    fn note(&mut self, lines: &[&str], index: usize) -> Idle<B> {
        show(&mut self.board, screens::status(lines));
        Idle::Note { index }
    }

    /// "pair a phone" was chosen: open a pairing and wait for the phone's tap.
    fn pair_start(&mut self, index: usize) -> Idle<B> {
        match self.pair_open() {
            Ok(_) => {
                show(&mut self.board, screens::pair_wait(None));
                Idle::PairWait { index }
            }
            Err(BadgeError::AlreadyPaired) => self.note(&["unpair the current phone first"], index),
            Err(BadgeError::ChipHeld) => self.note(&[RESTART], index),
            Err(_) => self.note(&["the chip cannot pair a phone"], index),
        }
    }

    fn pair_wait_step(&mut self, index: usize) -> Idle<B> {
        match self.board.press() {
            Press::None => Idle::PairWait { index },
            Press::Left | Press::Right => {
                self.pair_close();
                self.show_chooser(index);
                Idle::Choose { index }
            }
            Press::Both => match self.pair_read_inbox() {
                Ok(Some(inbox)) => {
                    let Some(words) = self.pair_words(&inbox) else {
                        self.pair_close();
                        return self.note(&["pairing failed, try again"], index);
                    };
                    self.board.log(&format!(
                        "pairing: {} asks, words {}",
                        screens::phone_name(&inbox.name),
                        words.join(" ")
                    ));
                    show(&mut self.board, screens::pair_prompt(&inbox.name, &words));
                    Idle::PairAsk { inbox, index }
                }
                Ok(None) => {
                    show(&mut self.board, screens::pair_wait(Some("no phone has tapped yet")));
                    Idle::PairWait { index }
                }
                Err(BadgeError::NotForThisBadge) => {
                    show(
                        &mut self.board,
                        screens::pair_wait(Some("that phone's request is not for this badge. Try again.")),
                    );
                    Idle::PairWait { index }
                }
                Err(_) => {
                    self.pair_close();
                    self.note(&["pairing failed, try again"], index)
                }
            },
        }
    }

    fn pair_ask_step(&mut self, inbox: Inbox, index: usize) -> Idle<B> {
        match self.board.press() {
            Press::None => Idle::PairAsk { inbox, index },
            Press::Both => {
                let name = screens::phone_name(&inbox.name);
                match self.pair_accept(inbox) {
                    Ok(()) => self.note(&[&format!("paired with {name}"), "tap the phone again"], index),
                    Err(BadgeError::ChipHeld) => self.note(&[RESTART], index),
                    Err(BadgeError::UnpairUnfinished) => self.note(
                        &["finish the last unpair first, turn the badge off and on"],
                        index,
                    ),
                    Err(_) => self.note(&["pairing failed, try again"], index),
                }
            }
            Press::Left | Press::Right => {
                self.pair_decline();
                self.note(&["not paired"], index)
            }
        }
    }

    fn phone_step(&mut self, index: usize) -> Idle<B> {
        match self.board.press() {
            Press::None => Idle::Phone { index },
            Press::Both => match self.unpair() {
                Ok(()) => self.note(&["phone unpaired", "its taps end now"], index),
                Err(BadgeError::NotAllUpdated) => {
                    self.note(&["phone unpaired, some circles not updated"], index)
                }
                Err(BadgeError::ChipHeld) => self.note(&[RESTART], index),
                Err(_) => self.note(&["unpairing failed, try again"], index),
            },
            Press::Left | Press::Right => {
                self.show_chooser(index);
                Idle::Choose { index }
            }
        }
    }

    fn show_chooser(&mut self, index: usize) {
        let names = self.chooser_names();
        let title = if self.chip_held { "chip needs a restart" } else { "circles" };
        show(&mut self.board, screens::chooser_titled(title, &names, index));
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
            Press::Both if self.chooser_names()[index] == PAIR_PHONE => self.pair_start(index),
            Press::Both if self.chooser_names()[index] == PHONE => match self.paired_phone() {
                Some(name) => {
                    show(&mut self.board, screens::phone_page(&name));
                    Idle::Phone { index }
                }
                None => {
                    self.show_chooser(index);
                    Idle::Choose { index }
                }
            },
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
        let held: Vec<CircleId> = self.circles.iter().map(|c| c.circle_id).collect();
        self.cards.retain(|(id, _)| held.contains(id));
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
        let nonce = fresh_nonce(&mut self.board);
        let mut menu = Menu {
            pads: Pads { held, tier },
            down,
            rx: Vec::new(),
            page: Page::Words,
            connected: false,
            connect: None,
            pending: None,
            peer_seen: false,
            covered: false,
            confirm_forget: false,
            grant_entries: Vec::new(),
            grant_index: 0,
            confirm_revoke: false,
            nonce,
        };
        let c = &self.circles[self.current];
        show_page(&mut self.board, c, &menu);
        if let Some(e) = broken {
            show_status(&mut self.board, &e);
            menu.covered = true;
        }
        Idle::On(Box::new(menu))
    }

    fn menu_step(
        board: &mut B,
        circles: &[Circle],
        current: usize,
        m: &mut Menu<B>,
        cards: &mut Vec<(CircleId, Vec<u8>)>,
        clock: &mut Clock,
    ) -> MenuAction {
        let c = &circles[current];
        let asked = m.pending.is_some() || m.connect.is_some();
        let present = m.down.peer_present();
        if present != m.peer_seen {
            m.peer_seen = present;
            board.log(if present {
                "down link: a laptop is listening"
            } else {
                "down link: nothing plugged in"
            });
            if !present {
                // Whatever was allowed was allowed for that laptop while it was plugged in.
                m.connected = false;
                if m.connect.take().is_some() {
                    m.covered = false;
                    show_page(board, c, m);
                }
            }
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
            Self::menu_decode(board, circles, m, clock);
        }
        // A request's prompt drawn over the revoke or forget question replaces it. Once that
        // is answered the page is back, and a Both then is the page's, not an answer.
        if m.pending.is_some() || m.connect.is_some() {
            m.confirm_revoke = false;
            m.confirm_forget = false;
        }

        // A gesture made before the prompt was on the screen is not an answer to it: on the
        // Nano the prompt's refresh blocks for hundreds of milliseconds. Drop the gesture
        // seen on this call and count only gestures seen on a later call.
        if !asked && m.pending.is_some() {
            let _ = board.press();
            return MenuAction::Stay;
        }

        let gesture = board.press();
        if let Some(p) = m.connect.take() {
            match gesture {
                Press::None => m.connect = Some(p),
                Press::Both => {
                    m.connected = true;
                    board.log("laptop allowed until unplugged");
                    m.covered = false;
                    show_page(board, c, m);
                    Self::handle_request(board, circles, m, clock, p.seq, p.request);
                }
                Press::Left | Press::Right => {
                    board.log("laptop refused");
                    let busy = Response::Error {
                        code: ErrorCode::Busy,
                    };
                    respond(&mut m.down, p.seq, &busy.encode());
                    m.covered = false;
                    show_page(board, c, m);
                }
            }
            return MenuAction::Stay;
        }
        if let Some(p) = &m.pending {
            let yes = match gesture {
                Press::None => return MenuAction::Stay,
                Press::Both => true,
                Press::Left | Press::Right => false,
            };
            let mut served = Served {
                custody: &mut m.pads,
                apps: BadgeApps::on_board(circles, board, clock, m.nonce),
                signed_card: None,
            };
            let resp = serve(&mut served, &mut Answer(yes), &p.request);
            respond(&mut m.down, p.seq, &resp);
            if let Some((circle, card)) = served.signed_card {
                cards.retain(|(id, _)| *id != circle);
                cards.push((circle, card));
            }
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
                // A use under a grant may have been drawn over the page: its counts are read
                // again, not shown from before.
                if m.page == Page::Grants {
                    Self::read_grants(board, clock, c, m);
                    m.grant_index = m.grant_index.min(m.grant_entries.len().saturating_sub(1));
                }
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

        if m.confirm_revoke {
            match gesture {
                Press::None => {}
                Press::Both => {
                    if let Some(e) = m.grant_entries.get(m.grant_index) {
                        if grants::revoke(board, clock, &e.cti).is_err() {
                            show_status(board, "could not revoke");
                            m.covered = true;
                        }
                    }
                    m.confirm_revoke = false;
                    Self::read_grants(board, clock, c, m);
                    m.grant_index = m.grant_index.min(m.grant_entries.len().saturating_sub(1));
                    if !m.covered {
                        show_page(board, c, m);
                    }
                }
                Press::Left | Press::Right => {
                    m.confirm_revoke = false;
                    show_page(board, c, m);
                }
            }
            return MenuAction::Stay;
        }

        match gesture {
            Press::None => {}
            Press::Left | Press::Right => {
                let left = gesture == Press::Left;
                // On the grants page the caret moves first; the ring turns from the first
                // or last entry, or when there are none.
                if m.page == Page::Grants {
                    let last = m.grant_entries.len().saturating_sub(1);
                    if left && m.grant_index > 0 {
                        m.grant_index -= 1;
                        show_page(board, c, m);
                        return MenuAction::Stay;
                    }
                    if !left && m.grant_index < last {
                        m.grant_index += 1;
                        show_page(board, c, m);
                        return MenuAction::Stay;
                    }
                }
                m.page = if left {
                    m.page.previous()
                } else {
                    m.page.next()
                };
                if m.page == Page::Grants {
                    Self::read_grants(board, clock, c, m);
                    m.grant_index = if left {
                        m.grant_entries.len().saturating_sub(1)
                    } else {
                        0
                    };
                }
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
                Page::Grants => match m.grant_entries.get(m.grant_index) {
                    Some(e) => {
                        m.confirm_revoke = true;
                        show(board, screens::revoke_prompt(&e.agent));
                    }
                    None => show_page(board, c, m),
                },
                Page::Circles => return MenuAction::Choose,
                Page::Forget => {
                    m.confirm_forget = true;
                    show(board, screens::forget_prompt(&c.params.name));
                }
            },
        }
        MenuAction::Stay
    }

    /// Read the live grants of the current circle into the menu.
    fn read_grants(board: &mut B, clock: &Clock, c: &Circle, m: &mut Menu<B>) {
        m.grant_entries = grants::page_entries(board, clock, &c.circle_id);
    }

    /// Answer or queue every whole request frame in `rx`. Bytes that are not a frame, or a
    /// header longer than any request, are dropped one at a time to resync. A partial header
    /// left by garbage can claim a length that is never filled, so while the front is
    /// incomplete, a whole frame further on wins and the bytes before it are dropped.
    fn menu_decode(board: &mut B, circles: &[Circle], m: &mut Menu<B>, clock: &mut Clock) {
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
            if m.pending.is_some() || m.connect.is_some() {
                let busy = Response::Error {
                    code: ErrorCode::Busy,
                };
                respond(&mut m.down, seq, &busy.encode());
                continue;
            }
            if !m.connected {
                // The first request since the laptop was plugged in: ask about the laptop
                // before anything else, and hold the request for the answer.
                show(board, screens::connect_prompt());
                m.connect = Some(Pending {
                    seq,
                    request: payload,
                    chip: false,
                });
                continue;
            }
            Self::handle_request(board, circles, m, clock, seq, payload);
        }
    }

    /// Answer `payload` (a request frame's payload with sequence `seq`) now, or draw its
    /// question and hold it for the holder's answer.
    fn handle_request(
        board: &mut B,
        circles: &[Circle],
        m: &mut Menu<B>,
        clock: &mut Clock,
        seq: u32,
        payload: Vec<u8>,
    ) {
        {
            if m.nonce.is_none() {
                m.nonce = fresh_nonce(board);
            }
            let req = Request::decode(&payload).ok();
            // A request longer than its kind allows is refused before the holder is asked:
            // `serve` would refuse it after the answer, and the question would be wasted.
            if req.as_ref().is_some_and(|r| !r.length_ok(payload.len())) {
                board.log("request refused: too long");
                let malformed = Response::Error {
                    code: ErrorCode::Malformed,
                };
                respond(&mut m.down, seq, &malformed.encode());
                return;
            }
            let ask = match &req {
                Some(Request::Chip { circle, op }) => {
                    BadgeApps::on_board(circles, board, clock, m.nonce).prompt(circle, op)
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
                    let chip = matches!(req, Some(Request::Chip { .. }));
                    // A request under a grant that asks is asked, and answered, as the
                    // ordinary signature it is: nothing is counted against the grant.
                    let request = match req {
                        Some(Request::Chip {
                            circle,
                            op: ChipRequest::SshSignUnderGrant { data, content, .. },
                        }) => Request::Chip {
                            circle,
                            op: ChipRequest::SshSign {
                                data,
                                host: None,
                                content: Some(content),
                            },
                        }
                        .encode(),
                        _ => payload,
                    };
                    m.pending = Some(Pending { seq, request, chip });
                }
                // Identify, Remaining, Describe, Attest and anything malformed: answered now,
                // never asked.
                Ok(None) => {
                    let mut served = Served {
                        custody: &mut m.pads,
                        apps: BadgeApps::on_board(circles, board, clock, m.nonce),
                        signed_card: None,
                    };
                    let resp = serve(&mut served, &mut Answer(false), &payload);
                    let used = served.apps.grant_use.take();
                    let spent = served.apps.nonce_spent;
                    respond(&mut m.down, seq, &resp);
                    // A proof under the nonce was accepted: the nonce is spent, whether or not
                    // the signature followed.
                    if spent {
                        m.nonce = fresh_nonce(board);
                    }
                    // Signed under a grant: the badge shows what it signed until the next
                    // gesture or screen, without waiting.
                    // It replaces the revoke or forget question, as a prompt does: once it is
                    // dismissed the page is back, and a Both then is the page's, not an answer.
                    if let Some(u) = used {
                        board.note_grant_use(&u.cti, &u.agent, &u.subject, u.remaining, u.budget);
                        m.covered = true;
                        m.confirm_revoke = false;
                        m.confirm_forget = false;
                    }
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

/// 16 bytes from the board's random source for a link's nonce, or `None`, logged, when the
/// source fails its health tests.
fn fresh_nonce<B: Board>(board: &mut B) -> Option<[u8; 16]> {
    let mut nonce = [0u8; 16];
    match board.entropy().fill(&mut nonce) {
        Ok(()) => Some(nonce),
        Err(_) => {
            board.log("trng: failed, no grant nonce until a draw succeeds");
            None
        }
    }
}

/// The generations a bump payload moves from and to: its first 64 bytes. The badge reads only
/// bumps it signed itself, from its own public objects.
fn bump_ends(payload: &[u8]) -> Option<([u8; 32], [u8; 32])> {
    Some((
        payload.get(..32)?.try_into().ok()?,
        payload.get(32..64)?.try_into().ok()?,
    ))
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
    board.screen_drawn();
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
        Page::Grants => {
            let entries: Vec<(String, String)> = m
                .grant_entries
                .iter()
                .map(|e| (e.agent.clone(), e.line.clone()))
                .collect();
            let page = screens::grants_page(
                &c.params.name,
                &tier,
                m.page.index() + 1,
                Page::ALL.len(),
                &entries,
                m.grant_index,
            );
            show(board, page);
            return;
        }
        Page::Key => vec!["Both to spend 32 bytes"],
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

/// "1 chip object could not be deleted", or "N chip objects could not be deleted".
fn left_line(n: usize) -> String {
    if n == 1 {
        String::from("1 chip object could not be deleted")
    } else {
        format!("{n} chip objects could not be deleted")
    }
}
