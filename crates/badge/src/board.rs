//! The [`Board`] trait: everything the badge needs from the hardware under it.

use crate::chip::ChipRecord;
use crate::grants::GrantsRecord;
use crate::screens;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use cahoots_chip::ChipOps;
use cahoots_core::checkwords::WORDLIST;
use cahoots_core::circle_id::CircleId;
use cahoots_core::params::CircleParams;
use cahoots_hal::ledger::RecordStore;
use cahoots_hal::{Display, Entropy, Link, PadStore};
use core::fmt;
use zeroize::{Zeroize, Zeroizing};

/// A board-level storage or link failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardError {
    Storage,
    Link,
}

/// One gesture on the two buttons. The board debounces the buttons and detects the chord:
/// a Left or Right is held back for the chord window, so that pressing both buttons is
/// reported as `Both` and never also as a single press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// No gesture since the last call.
    None,
    /// The left button (A): the previous page, or refuse on a prompt.
    Left,
    /// The right button (B): the next page, or refuse on a prompt.
    Right,
    /// Both buttons together: select, keep or allow.
    Both,
}

/// The circle a badge belongs to: its parameters, this card's seat, the circle id, the six
/// check words the ceremony produced, this seat's signing key, the tier it declared and the
/// sealed manifest every seat signed.
///
/// The seat key and the root are secrets. `Debug` leaves them out, and the record zeroizes
/// them on drop. A board stores them as it stores the pad: in plaintext on a tier 0 or tier 1
/// board.
///
/// On a badge with a chip the seat key and the root live in the chip, in slot `chip_slot`:
/// then `seat_key` is all zeros and `root` is `None`. A record written before the chip
/// (`CAH2`) loads with neither, and signs with its software seat key as before.
#[derive(Clone, PartialEq, Eq)]
pub struct Circle {
    pub params: CircleParams,
    pub seat: u8,
    pub circle_id: CircleId,
    /// The check words as indices into `WORDLIST`. Stored at keep time, because spending
    /// erases pad bytes and words worked out from the pad later would differ.
    pub words: [u16; 6],
    /// This seat's signing key, the seed `cahoots_core::seat::SeatKey::from_seed` reads. All
    /// zeros when `chip_slot` is set: the key is in the chip.
    pub seat_key: [u8; 32],
    /// The chip slot that holds this circle's seat key, root and epoch counter, if any.
    pub chip_slot: Option<u8>,
    /// The circle root (`cahoots_core::root`), kept here by a badge without a chip.
    pub root: Option<[u8; 32]>,
    /// The tier this badge declared in the ceremony.
    pub tier: u8,
    /// The encoded `cahoots_core::manifest::SealedManifest`.
    pub manifest: Vec<u8>,
}

impl fmt::Debug for Circle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Circle")
            .field("params", &self.params)
            .field("seat", &self.seat)
            .field("circle_id", &self.circle_id)
            .field("words", &self.words)
            .field("chip_slot", &self.chip_slot)
            .field("tier", &self.tier)
            .field("manifest", &self.manifest)
            .finish_non_exhaustive()
    }
}

impl Drop for Circle {
    fn drop(&mut self) {
        self.seat_key.zeroize();
        if let Some(root) = self.root.as_mut() {
            root.zeroize();
        }
    }
}

impl Circle {
    /// The six check words, looked up in `WORDLIST`.
    pub fn check_words(&self) -> [&'static str; 6] {
        self.words
            .map(|i| WORDLIST[usize::from(i) % WORDLIST.len()])
    }

    /// The record a flash board stores, rewritten whole:
    ///
    /// `CAH3 | len u16 LE | params | seat | circle id | words | seat key 32 | tier u8 |
    /// manifest len u16 LE | manifest | flags u8 | slot u8 | root 32`
    ///
    /// `len` counts everything after itself; `words` is the six `u16` indices into
    /// `WORDLIST`, little-endian; `manifest` is the encoded `SealedManifest`. `flags` bit 0
    /// says `slot` is a chip slot, bit 1 that `root` is the circle root; an absent one is
    /// written as zeros. Without the last 34 bytes, and with the magic `CAH2`, it is the
    /// record from before the chip, which [`Circle::decode_record`] still reads.
    ///
    /// `None` if the record would not fit [`RECORD_MAX`] bytes. The result holds the seat key
    /// and the root: it zeroizes on drop.
    pub fn encode_record(&self) -> Option<Zeroizing<Vec<u8>>> {
        let params = self.params.encode();
        let mlen = u16::try_from(self.manifest.len()).ok()?;
        let len = params.len()
            + 1
            + self.circle_id.len()
            + WORDS_LEN
            + 32
            + 1
            + 2
            + self.manifest.len()
            + CAH3_TAIL;
        if 6 + len > RECORD_MAX {
            return None;
        }
        let mut body = Zeroizing::new(Vec::with_capacity(6 + len));
        body.extend_from_slice(MAGIC_CAH3);
        body.extend_from_slice(&(len as u16).to_le_bytes());
        body.extend_from_slice(&params);
        body.push(self.seat);
        body.extend_from_slice(&self.circle_id);
        for w in self.words {
            body.extend_from_slice(&w.to_le_bytes());
        }
        body.extend_from_slice(&self.seat_key);
        body.push(self.tier);
        body.extend_from_slice(&mlen.to_le_bytes());
        body.extend_from_slice(&self.manifest);
        let flags = u8::from(self.chip_slot.is_some()) | u8::from(self.root.is_some()) << 1;
        body.push(flags);
        body.push(self.chip_slot.unwrap_or(0));
        body.extend_from_slice(self.root.as_ref().unwrap_or(&[0; 32]));
        Some(body)
    }

    /// A whole record, header included, as [`Circle::encode_record`] writes it or as a `CAH2`
    /// record. `None` for any other magic (`CAH1`, from before the seat key, included: the
    /// holder runs the ceremony again), a length that does not match, or a field that does
    /// not parse.
    pub fn decode_record(bytes: &[u8]) -> Option<Circle> {
        let len = record_body_len(bytes.get(..6)?)?;
        let body = bytes.get(6..)?;
        if body.len() != len {
            return None;
        }
        let cah3 = &bytes[..4] == MAGIC_CAH3;
        let id_len = core::mem::size_of::<CircleId>();
        // The params encoding is 8 bytes then the name, its length in byte 7.
        let params_len = 8 + usize::from(*body.get(7)?);
        let id_end = params_len + 1 + id_len;
        let words_end = id_end + WORDS_LEN;
        let tail = words_end + 32 + 1 + 2;
        if body.len() < tail {
            return None;
        }
        let params = CircleParams::decode(&body[..params_len]).ok()?;
        let seat = body[params_len];
        let circle_id: CircleId = body[params_len + 1..id_end].try_into().ok()?;
        let mut words = [0u16; 6];
        for (w, b) in words
            .iter_mut()
            .zip(body[id_end..words_end].chunks_exact(2))
        {
            *w = u16::from_le_bytes([b[0], b[1]]);
            if usize::from(*w) >= WORDLIST.len() {
                return None;
            }
        }
        let mut seat_key = [0u8; 32];
        seat_key.copy_from_slice(&body[words_end..words_end + 32]);
        let tier = body[words_end + 32];
        let mlen = usize::from(u16::from_le_bytes([body[words_end + 33], body[words_end + 34]]));
        let extra = if cah3 { CAH3_TAIL } else { 0 };
        if body.len() != tail + mlen + extra {
            seat_key.zeroize();
            return None;
        }
        let manifest = body[tail..tail + mlen].to_vec();
        let (mut chip_slot, mut root) = (None, None);
        if cah3 {
            let t = &body[tail + mlen..];
            let (flags, slot, r) = (t[0], t[1], &t[2..34]);
            // Absent fields are zeros and unknown flags are refused, so one circle has one
            // record.
            if flags & !0b11 != 0
                || (flags & 1 == 0 && slot != 0)
                || (flags & 2 == 0 && r.iter().any(|&b| b != 0))
            {
                seat_key.zeroize();
                return None;
            }
            if flags & 1 != 0 {
                chip_slot = Some(slot);
            }
            if flags & 2 != 0 {
                root = Some(r.try_into().ok()?);
            }
        }
        Some(Circle {
            params,
            seat,
            circle_id,
            words,
            seat_key,
            chip_slot,
            root,
            tier,
            manifest,
        })
    }
}

/// The record from before the chip: no slot, no root.
pub const MAGIC_CAH2: &[u8; 4] = b"CAH2";
/// The record with a chip slot and a root.
pub const MAGIC_CAH3: &[u8; 4] = b"CAH3";
/// The largest whole record, header included: a 4 KiB flash sector.
pub const RECORD_MAX: usize = 4096;
/// The six check words, as `u16` word indices.
const WORDS_LEN: usize = 12;
/// `flags | slot | root` at the end of a `CAH3` record.
const CAH3_TAIL: usize = 1 + 1 + 32;
/// The shortest body: an empty name, no manifest.
const MIN_BODY: usize = 8 + 1 + core::mem::size_of::<CircleId>() + WORDS_LEN + 32 + 1 + 2;

/// From a record's first 6 bytes, the length of the body after them, if the magic is one
/// [`Circle::decode_record`] reads and the length could be a record. A flash board reads
/// the header, then that many bytes.
pub fn record_body_len(head: &[u8]) -> Option<usize> {
    let head: &[u8; 6] = head.try_into().ok()?;
    let magic = &head[..4];
    if magic != MAGIC_CAH2 && magic != MAGIC_CAH3 {
        return None;
    }
    let len = usize::from(u16::from_le_bytes([head[4], head[5]]));
    if len < MIN_BODY || 6 + len > RECORD_MAX {
        return None;
    }
    Some(len)
}

/// The indices into `WORDLIST` of six check words, or `None` if one is not in the list.
pub fn word_indices(words: &[&str; 6]) -> Option<[u16; 6]> {
    let mut out = [0u16; 6];
    for (slot, w) in out.iter_mut().zip(words) {
        *slot = WORDLIST.iter().position(|x| x == w)? as u16;
    }
    Some(out)
}

/// The place a circle's record, pad and ledger are in, on a board with `held.len()` places:
/// the place whose record holds `circle`, else the place a pad was written to for it
/// (`pending`), else the lowest place that holds nothing and is not pending for another.
/// `None` if every place is taken.
pub fn place_for(
    held: &[Option<CircleId>],
    pending: Option<(CircleId, usize)>,
    circle: &CircleId,
) -> Option<usize> {
    if let Some(p) = held.iter().position(|h| h.as_ref() == Some(circle)) {
        return Some(p);
    }
    match pending {
        Some((id, p)) if id == *circle && held.get(p).is_some_and(Option::is_none) => Some(p),
        _ => (0..held.len())
            .find(|&p| held[p].is_none() && pending.is_none_or(|(_, q)| q != p)),
    }
}

/// For [`Board::store_circles`] on a board with `held.len()` places: which of `circles` each
/// place holds after the store. A circle keeps the place its record is in; a new one takes
/// its pending place, else the lowest place left free. A place whose circle is not in the
/// list is free. An error if one circle id is listed twice or the circles do not fit.
pub fn assign_places(
    held: &[Option<CircleId>],
    pending: Option<(CircleId, usize)>,
    circles: &[Circle],
) -> Result<Vec<Option<usize>>, BoardError> {
    let mut out: Vec<Option<usize>> = vec![None; held.len()];
    let listed = |id: &CircleId| circles.iter().any(|c| c.circle_id == *id);
    let mut placed = vec![false; circles.len()];
    for (i, c) in circles.iter().enumerate() {
        if circles[..i].iter().any(|d| d.circle_id == c.circle_id) {
            return Err(BoardError::Storage);
        }
        if let Some(p) = held.iter().position(|h| h.as_ref() == Some(&c.circle_id)) {
            out[p] = Some(i);
            placed[i] = true;
        }
    }
    let free = |out: &[Option<usize>], p: usize| {
        out[p].is_none() && held[p].as_ref().is_none_or(|h| !listed(h))
    };
    for (i, c) in circles.iter().enumerate() {
        if placed[i] {
            continue;
        }
        let p = match pending {
            Some((id, p)) if id == c.circle_id && p < held.len() && free(&out, p) => p,
            _ => (0..held.len())
                .find(|&p| free(&out, p))
                .ok_or(BoardError::Storage)?,
        };
        out[p] = Some(i);
    }
    Ok(out)
}

/// One board: drivers plus a pin table. The badge logic is generic over this trait.
pub trait Board {
    type Entropy: Entropy;
    type Pad: PadStore;
    type Records: RecordStore;
    type Display: Display;
    type Link: Link;

    /// The random source. The board wraps it in `cahoots_hal::health::Tested`.
    fn entropy(&mut self) -> &mut Self::Entropy;
    fn display(&mut self) -> &mut Self::Display;
    /// The next gesture since the last call, consumed, or `Press::None`.
    fn press(&mut self) -> Press;
    /// The screen's refresh is done. A board may log it; a bench driver presses only then,
    /// since a gesture seen during the refresh is not taken as an answer.
    fn screen_drawn(&mut self) {}
    /// Button A (left) is held right now (level). Used for the wipe at boot.
    fn a_down(&self) -> bool;
    fn delay_ms(&mut self, ms: u32);
    /// The two links, `(down, up)`. The badge calls this once per ceremony and again when
    /// it enters the idle menu, so every call must return usable links to the same two
    /// connectors: shared handles to links built once, or fresh unit links.
    fn links(&mut self) -> (Self::Link, Self::Link);
    /// How many circles the board can hold. A board that holds one keeps the default and
    /// implements [`Board::load_circle`] and [`Board::save_circle`]; the defaults of
    /// [`Board::load_circles`] and [`Board::store_circles`] adapt it.
    fn max_circles(&self) -> usize {
        1
    }
    /// Every circle the board holds, in the order of its places.
    fn load_circles(&mut self) -> Vec<Circle> {
        self.load_circle().into_iter().collect()
    }
    /// Store the circles the badge holds, replacing the last list: a circle not in it loses
    /// its record. Each circle keeps the place its record is in; a new one takes the place its
    /// pad was written to by [`Board::write_pad`]. An error if they do not fit.
    ///
    /// The default is for a board with one record: [`Board::forget`] erased the record
    /// already, so an empty list has nothing left to do.
    fn store_circles(&mut self, circles: &[Circle]) -> Result<(), BoardError> {
        match circles {
            [] => Ok(()),
            [c] => self.save_circle(c),
            _ => Err(BoardError::Storage),
        }
    }
    /// The one circle of a board with one record.
    fn load_circle(&self) -> Option<Circle> {
        None
    }
    /// Store the one circle of a board with one record, replacing it.
    fn save_circle(&mut self, c: &Circle) -> Result<(), BoardError> {
        let _ = c;
        Err(BoardError::Storage)
    }
    /// Forget circle `circle`: its record first, then the `pad_len` bytes of its pad and its
    /// ledger. The record goes first so that a failure or a power loss part way never leaves
    /// a record over a blank pad, which would serve the erased bytes as keys; pad bytes left
    /// in a free place are overwritten by the next [`Board::write_pad`] there.
    fn forget(&mut self, circle: &CircleId, pad_len: u32) -> Result<(), BoardError>;
    /// Store a new pad for circle `circle` and return a handle to it. On a board with many
    /// places, a circle not stored yet takes the lowest free place, and keeps it when the
    /// badge stores the circle.
    fn write_pad(&mut self, circle: &CircleId, pad: &[u8]) -> Result<Self::Pad, BoardError>;
    /// Open the stored pad of `len` bytes of circle `circle`.
    fn open_pad(&mut self, circle: &CircleId, len: u32) -> Result<Self::Pad, BoardError>;
    /// The ledger's record region of circle `circle`. `fresh` clears it first.
    fn records(&mut self, circle: &CircleId, fresh: bool) -> Result<Self::Records, BoardError>;
    /// `(name, pad_len)` if this card is the bottom of the pile and proposes a circle.
    fn proposal(&self) -> (String, u32);
    fn tier(&self) -> u8;
    /// Board-specific self-test lines. The badge adds the common ones.
    fn selftest(&mut self) -> Vec<String>;
    fn log(&mut self, line: &str);
    /// The text of the screen the badge is about to draw, top line first. A board that
    /// keeps a text copy of the screen (a test fake, the host simulator) records it here.
    /// A `secret` screen shows key bytes: a board must not write those lines to a log, a
    /// trace or anywhere else that outlives the screen.
    fn note_screen(&mut self, lines: &[String], secret: bool) {
        let _ = (lines, secret);
    }
    /// The board's secure element, if it has one. A board without one keeps the seat key and
    /// the circle root in software, in the circle record.
    fn chip(&mut self) -> Option<&mut dyn ChipOps> {
        None
    }
    /// The chip record (see [`crate::chip`]): `Ok(None)` if none is stored, an error if one is
    /// stored but cannot be read or does not decode. The two differ: the badge deletes chip
    /// objects the record does not know, so an unreadable record must not pass for an empty
    /// one. A board without a chip stores none.
    fn chip_record_load(&mut self) -> Result<Option<ChipRecord>, BoardError> {
        Ok(None)
    }
    /// Store the chip record, replacing the last. A board without a chip keeps nothing.
    fn chip_record_store(&mut self, r: &ChipRecord) -> Result<(), BoardError> {
        let _ = r;
        Ok(())
    }
    /// The grants record (see [`crate::grants`]): `Ok(None)` if none is stored, an error if one
    /// is stored but cannot be read or does not decode. A board that keeps no grants keeps the
    /// default, and every grant it issues is unknown at its first use.
    fn grants_load(&mut self) -> Result<Option<GrantsRecord>, BoardError> {
        Ok(None)
    }
    /// Store the grants record, replacing the last.
    fn grants_store(&mut self, r: &GrantsRecord) -> Result<(), BoardError> {
        let _ = r;
        Ok(())
    }
    /// Seconds since boot. A board with no timer returns 0.
    fn uptime_secs(&self) -> u64 {
        0
    }
    /// A signature was just made under a grant: show what was signed until the next screen,
    /// and log it. `cti` is the grant's id.
    fn note_grant_use(
        &mut self,
        cti: &[u8; 16],
        agent: &str,
        subject: &str,
        remaining: u32,
        budget: u32,
    ) {
        let s = screens::grant_signed(agent, subject, remaining, budget);
        self.note_screen(&s.lines, s.secret);
        self.display().draw(&s.frame);
        self.screen_drawn();
        let short: String = cti[..4].iter().map(|b| alloc::format!("{b:02x}")).collect();
        self.log(&alloc::format!(
            "grant {short}: signed git commit \"{subject}\", {remaining} left"
        ));
    }
}
