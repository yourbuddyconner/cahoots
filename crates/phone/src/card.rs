//! Card circles: the circles of a badge this phone is paired to.
//!
//! Pairing takes two taps. The first ([`Phone::pair_card_start`]) draws a fresh slot key,
//! seals it to the key the badge's open pairing shows, writes it to the badge's inbox signed
//! with this phone's own key, and counts the badge's circles from its pairing object. Their
//! public objects are not readable before the press: only the badge and its paired phone
//! read them. The owner checks the six words
//! on both screens and presses both buttons on the badge. The second tap
//! ([`Phone::pair_card_finish`]) opens the phone slot with that key, which works only once the
//! badge took it, keeps each circle as a card circle, fetches the keys its chat needs and a
//! pocket of hello words, posts any bump the badge left for a device to post, and publishes
//! the badge's card in the chat.
//!
//! A card circle holds no pad, no root, no pair keys and no seat key. It chats as the badge's
//! seat under one generation's key. When a bump moves the circle on, the chat waits
//! (`needs_tap`) until a tap gives the next key. Hello words need no tap of their own: a tap
//! fetches the next [`POCKET_WORDS`] with each other seat ahead, and `next_hello` asks for a
//! tap (`NeedsTap`) only when they run out. One tap ([`Phone::tap_card`]) does all of it for
//! every card circle, and [`Phone::card_status`] says what it would do. The slot key is kept
//! in the keychain under `cahoots:card:` and the badge's nonce, behind Face ID.

use crate::chat::{self, Circle};
use crate::nfc::{sealed_inbox, Session, Tap, TapError};
use crate::store::{self, CardCircle, ChatRecord, CircleRecord, PocketWords, SettingsRecord};
use crate::{
    signer, CardStatus, HelloWords, PairPreview, Phone, PhoneError, State, TapReport, WordsLeft,
    PHONE_KEY,
};
use cahoots_badge::public::PublicObject;
use cahoots_core::manifest::{Manifest, SealedManifest};
use cahoots_core::seat::SeatPublic;
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

/// What the reader shows while it looks for the card.
const PROMPT: &str = "Hold the phone to the card.";
/// What the phone calls a badge whose pairing object gives no name.
pub const NO_BADGE_NAME: &str = "your badge";

/// "1 circle", "3 circles".
fn circle_count(n: usize) -> String {
    match n {
        1 => String::from("1 circle"),
        n => format!("{n} circles"),
    }
}

/// A count as a `u32`, for the app.
fn as_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A generation's id and its key.
type GenKey = ([u8; 32], Zeroizing<[u8; 32]>);

/// The data whose HMAC under a circle's root is a chat generation's key:
/// `"chat/gen" || circle_id || generation_id`.
fn generation_label(circle_id: &[u8; 16], generation: &[u8; 32]) -> Vec<u8> {
    let mut d = cahoots_core::root::CHAT_GEN_LABEL.to_vec();
    d.extend_from_slice(circle_id);
    d.extend_from_slice(generation);
    d
}

/// The data whose HMAC under a hello object gives the words `speaker` says at `counter`:
/// `"cahoots/words/v1" || speaker || counter as u32 little-endian || 0x01`, the expand step of
/// `cahoots_core::pairwise::words`, whose extract step is the hello object's value.
fn hello_label(speaker: u8, counter: u32) -> Vec<u8> {
    let mut d = cahoots_core::pairwise::WORDS_DOMAIN.to_vec();
    d.push(speaker);
    d.extend_from_slice(&counter.to_le_bytes());
    d.push(1);
    d
}

/// The two words of a hello MAC: its first four bytes, little-endian, 11 bits each.
fn hello_words(mac: &[u8; 32]) -> String {
    use cahoots_core::checkwords::WORDLIST;
    let v = u32::from_le_bytes(mac[..4].try_into().expect("4 bytes"));
    [
        WORDLIST[(v & 0x7ff) as usize],
        WORDLIST[((v >> 11) & 0x7ff) as usize],
    ]
    .join(" ")
}

/// The ends of a bump's payload: the generation it moves from and the one it moves to.
fn bump_ends(payload: &[u8]) -> Option<([u8; 32], [u8; 32])> {
    Some((
        payload.get(..32)?.try_into().ok()?,
        payload.get(32..64)?.try_into().ok()?,
    ))
}

/// The generation a newly paired phone's chat starts in, and the ones it moves to after: the
/// public object's bumps, oldest first, each moving from where the one before moves to, and
/// ending at the object's generation. `None` when they do not chain.
fn generations(o: &PublicObject) -> Option<Vec<[u8; 32]>> {
    let Some(first) = o.pending_bumps.first() else {
        return Some(vec![o.generation]);
    };
    let mut out = vec![bump_ends(first)?.0];
    for b in &o.pending_bumps {
        let (from, to) = bump_ends(b)?;
        if Some(&from) != out.last() {
            return None;
        }
        out.push(to);
    }
    (out.last() == Some(&o.generation)).then_some(out)
}

/// The keychain label of the slot key for the badge whose nonce is `badge_nonce`.
fn key_label(badge_nonce: &[u8; 16]) -> String {
    format!("cahoots:card:{}", hex::encode(badge_nonce))
}

/// The seat this hello is with, checked: another seat of the circle. Returns the hello number
/// with it.
pub(crate) fn card_hello_seat(c: &Circle, other: u8) -> Result<u32, PhoneError> {
    if other == c.record.seat || other >= c.params.n {
        return Err(PhoneError::NotFound {
            message: "Hello words are not available for this seat.".into(),
        });
    }
    Ok(c.record.pair_counters[usize::from(other)])
}

/// How many hello numbers with each other seat a tap fetches ahead, from the seat's counter.
pub(crate) const POCKET_WORDS: u32 = 8;

/// A seat with this many hello numbers left, or fewer, is listed in [`CardStatus::words_low`]:
/// the app says to tap the card soon.
const WORDS_LOW: u32 = 3;

/// A pocket entry that holds two MACs.
fn usable(p: &PocketWords) -> bool {
    p.mine.len() == 32 && p.theirs.len() == 32
}

/// The words of the pair `pocket` holds with `other` at `number`: this seat's, then the other
/// seat's.
fn pocket_words(pocket: &[PocketWords], other: u8, number: u32) -> Option<(String, String)> {
    pocket
        .iter()
        .filter(|p| p.other == other && p.number == number)
        .find_map(|p| {
            let mine: [u8; 32] = p.mine[..].try_into().ok()?;
            let theirs: [u8; 32] = p.theirs[..].try_into().ok()?;
            Some((hello_words(&mine), hello_words(&theirs)))
        })
}

/// A card circle's pocket. Empty for any other circle, and for a record from before pockets.
fn pocket_of(c: &Circle) -> &[PocketWords] {
    c.record
        .card
        .as_ref()
        .and_then(|card| card.hello_pocket.as_deref())
        .unwrap_or(&[])
}

/// How many hello numbers with `other` the pocket holds from the seat's counter on, the
/// counter's own included.
pub(crate) fn pocket_left(c: &Circle, other: u8) -> u32 {
    let Some(&counter) = c.record.pair_counters.get(usize::from(other)) else {
        return 0;
    };
    let left = pocket_of(c)
        .iter()
        .filter(|p| p.other == other && p.number >= counter && usable(p))
        .count();
    u32::try_from(left).unwrap_or(u32::MAX)
}

/// Whether the pocket holds the hello words with `other` at `number`.
pub(crate) fn pocket_holds(c: &Circle, other: u8, number: u32) -> bool {
    pocket_words(pocket_of(c), other, number).is_some()
}

/// The hello words with `other` at its current number in a card circle, from the pocket, with
/// no tap. `NeedsTap` when the pocket does not hold them.
pub(crate) fn pocket_hello(c: &Circle, other: u8) -> Result<HelloWords, PhoneError> {
    let number = card_hello_seat(c, other)?;
    let (mine, theirs) = pocket_words(pocket_of(c), other, number).ok_or(PhoneError::NeedsTap)?;
    Ok(HelloWords {
        mine,
        theirs,
        number,
        card_left: Some(pocket_left(c, other)),
    })
}

/// In the slot session `s`, ask the hello objects of the circle in `slot` for the words seat
/// `me` and each other seat below `n` say at the [`POCKET_WORDS`] numbers from that seat's
/// counter in `counters`, for each number `pocket` does not hold yet. Pairs below a seat's
/// counter leave the pocket. The pocket changes only once every MAC came back. Returns how
/// many pairs were added.
fn fill(
    pocket: &mut Vec<PocketWords>,
    slot: u8,
    me: u8,
    n: u8,
    counters: &[u32],
    s: &mut Session<'_, '_>,
) -> Result<u32, TapError> {
    let mut added = Vec::new();
    for other in (0..n).filter(|o| *o != me) {
        let Some(&from) = counters.get(usize::from(other)) else {
            continue;
        };
        for number in (0..POCKET_WORDS).map(|i| from.wrapping_add(i)) {
            if pocket
                .iter()
                .any(|p| p.other == other && p.number == number && usable(p))
            {
                continue;
            }
            let mine = s.hmac_hello(slot, other, &hello_label(me, number))?;
            let theirs = s.hmac_hello(slot, other, &hello_label(other, number))?;
            added.push(PocketWords {
                other,
                number,
                mine: mine.to_vec().into(),
                theirs: theirs.to_vec().into(),
            });
        }
    }
    let count = u32::try_from(added.len()).unwrap_or(u32::MAX);
    pocket.retain(|p| {
        usable(p)
            && counters
                .get(usize::from(p.other))
                .is_some_and(|counter| p.number >= *counter)
    });
    pocket.extend(added);
    Ok(count)
}

/// A card circle's pocket topped up in the slot session `s`, as [`fill`] does, and how many
/// pairs were added. The circle does not change: the caller keeps the new pocket once the
/// session closed without error, and drops it if it did not.
pub(crate) fn pocket_fill(
    c: &Circle,
    s: &mut Session<'_, '_>,
) -> Result<(Vec<PocketWords>, u32), TapError> {
    let mut pocket = pocket_of(c).to_vec();
    let Some(card) = c.record.card.as_ref() else {
        return Ok((pocket, 0));
    };
    let (slot, me, n) = (card.circle_slot, c.record.seat, c.params.n);
    let added = fill(&mut pocket, slot, me, n, &c.record.pair_counters, s)?;
    Ok((pocket, added))
}

/// A circle of the badge, read from its public object at the second tap, checked.
struct Kept {
    object: PublicObject,
    manifest: Manifest,
    seat: u8,
    badge_public: SeatPublic,
    /// The generation the chat starts in, then each one a bump moves to.
    generations: Vec<[u8; 32]>,
    /// The hello words the tap fetched ahead, beside those a card circle held already.
    pocket: Vec<PocketWords>,
}

/// The public object of a circle this phone is paired for, checked: its manifest is sealed by
/// every seat and is this circle's, and its device grant is signed by a seat for this phone's
/// key. `None` for anything else, which is not kept.
fn check(object: PublicObject, mine: &[u8; 33]) -> Option<Kept> {
    let manifest = SealedManifest::decode(&object.manifest).ok()?.verify().ok()?;
    if manifest.circle_id != object.circle_id {
        return None;
    }
    let (seat, grantee, _) =
        cahoots_objects::grant::verify_device_grant(object.grant.as_ref()?, &manifest).ok()?;
    if grantee.0 != *mine {
        return None;
    }
    let badge_public = *manifest.seat_key(seat)?;
    let generations = generations(&object)?;
    Some(Kept {
        object,
        manifest,
        seat,
        badge_public,
        generations,
        pocket: Vec::new(),
    })
}

impl Phone {
    /// This phone's own public key, SEC1 compressed.
    fn my_public(&self) -> Result<[u8; 33], PhoneError> {
        self.me().public[..].try_into().map_err(|_| PhoneError::Storage {
            message: "This phone's own key could not be read.".into(),
        })
    }

    /// The slot key kept for a badge. Face ID guards it. No key, or one that is not a key, is
    /// a phone the card does not know.
    fn slot_key(&self, label: &str) -> Result<Zeroizing<[u8; 16]>, PhoneError> {
        let secret = Zeroizing::new(
            self.signer
                .get_secret(label.into())?
                .ok_or(PhoneError::NotPaired)?,
        );
        let key: [u8; 16] = secret[..].try_into().map_err(|_| PhoneError::NotPaired)?;
        Ok(Zeroizing::new(key))
    }

    /// Where the hello words of a circle the second tap keeps start: the counters and the
    /// pocket of a card circle this phone holds already, else every counter at 0 and an empty
    /// pocket. `None` for a circle this phone holds with a seat of its own, or cannot read:
    /// the tap does not keep it.
    fn pocket_start(&self, k: &Kept) -> Option<(Vec<u32>, Vec<PocketWords>)> {
        let id_hex = hex::encode(k.object.circle_id);
        if !store::circle_ids(&self.dir).contains(&id_hex) {
            return Some((vec![0; usize::from(k.manifest.params.n)], Vec::new()));
        }
        self.with_circle(&id_hex, |c, _| {
            Ok(c.record.card.as_ref().map(|card| {
                let pocket = card.hello_pocket.clone().unwrap_or_default();
                (c.record.pair_counters.clone(), pocket)
            }))
        })
        .ok()
        .flatten()
    }

    /// The ids of every card circle on this phone, in order, each loaded into `st` and kept
    /// there as `with_circle` keeps it. A circle that cannot be read is left out.
    fn card_circles(&self, st: &mut State) -> Vec<String> {
        let mut ids = Vec::new();
        for id in store::circle_ids(&self.dir) {
            if !st.circles.contains_key(&id) {
                let Ok(c) = Circle::load(&self.circle_dir(&id)) else {
                    continue;
                };
                st.circles.insert(id.clone(), c);
            }
            if st.circles[&id].record.is_card() {
                ids.push(id);
            }
        }
        ids
    }

    /// One tap that opens the phone slot with `key` and runs `f` in its session. The reader
    /// ends with the error's sentence when anything fails.
    fn tap_slot<T>(
        &self,
        prompt: &str,
        key: &[u8; 16],
        f: impl FnOnce(&mut Session<'_, '_>) -> Result<T, TapError>,
    ) -> Result<T, PhoneError> {
        let mut tap = Tap::begin(self.nfc.as_ref(), prompt)?;
        let r = tap.open_slot(key).and_then(|mut s| {
            let v = f(&mut s)?;
            s.close()?;
            Ok(v)
        });
        match r {
            Ok(v) => {
                tap.end("Done.", false)?;
                Ok(v)
            }
            Err(e) => {
                let e = PhoneError::from(e);
                let _ = tap.end(&e.to_string(), true);
                Err(e)
            }
        }
    }

    /// Keep one circle of the badge as a card circle, with the hello words the tap fetched,
    /// or bring a card circle already held up to the pairing, then read its relay, post the
    /// bumps it forwards, and publish the badge's card in its chat. `None` when the phone
    /// holds the circle with a seat of its own.
    fn keep_card_circle(
        &self,
        slot: u8,
        k: Kept,
        mut keys: Vec<GenKey>,
        badge_name: &str,
        label: &str,
    ) -> Result<Option<String>, PhoneError> {
        let id_hex = hex::encode(k.object.circle_id);
        let start = k.generations[0];
        let start_key = keys.remove(0).1;
        let card = CardCircle {
            badge_name: badge_name.into(),
            circle_slot: slot,
            badge_public: k.badge_public.0.to_vec().into(),
            generation_key: Some(*start_key),
            generation: start,
            key_label: label.into(),
            forward: k.object.pending_bumps.iter().map(|b| b.clone().into()).collect(),
            hello_pocket: Some(k.pocket),
        };
        if store::circle_ids(&self.dir).contains(&id_hex) {
            let held = self.with_circle(&id_hex, |c, _| {
                if !c.record.is_card() {
                    return Ok(false);
                }
                c.record.card = Some(card);
                c.record.save(&c.dir)?;
                // A chat that is not in the generation the badge has now moves to it.
                if c.session.generation_id() != start
                    && c.session.pending_generation() != Some(start)
                {
                    c.session.install_generation(start, start_key);
                    c.save_chat()?;
                }
                Ok(true)
            })?;
            if !held {
                return Ok(None);
            }
        } else {
            let n = usize::from(k.manifest.params.n);
            let relay = match k.object.relay.as_str() {
                "" => self.lock().relay.clone(),
                r => chat::check_relay(r)?,
            };
            let record = CircleRecord {
                params: k.manifest.params.encode().into(),
                circle_id: k.object.circle_id.to_vec().into(),
                seat: k.seat,
                tier: k.manifest.seats[usize::from(k.seat)].tier,
                label: "card".into(),
                words: Vec::new(),
                manifest: k.object.manifest.clone().into(),
                root: Vec::new().into(),
                seat_label: String::new(),
                seat_public: Vec::new().into(),
                pair_keys: vec![Vec::new().into(); n],
                pair_counters: vec![0; n],
                card: Some(card),
            };
            let chat = ChatRecord {
                state: Vec::new().into(),
                outbox: Vec::new(),
                journal: Vec::new(),
                pending_bump: None,
                pending_card: None,
                card_due: false,
            };
            let settings = SettingsRecord {
                relay,
                unread: 0,
                face_override: None,
            };
            store::create_card_circle(&self.dir, &id_hex, &record, &chat, &settings)?;
        }
        // The circle is kept whatever happens next: a relay out of reach leaves the bumps and
        // the card waiting for the next poll.
        let manifest = k.manifest;
        let badge_card = k.object.card.filter(|signed| {
            cahoots_objects::Card::verify(signed, &manifest).is_ok_and(|c| c.seat == k.seat)
        });
        self.with_circle(&id_hex, |c, me| {
            c.take_keys(keys)?;
            let _ = c.poll(me.http.as_ref());
            if let Some(signed) = badge_card {
                let _ = c.publish_card(me.http.as_ref(), signed);
            }
            Ok(())
        })?;
        Ok(Some(id_hex))
    }
}

#[uniffi::export]
impl Phone {
    /// The first tap of pairing with a badge, after the owner opened "pair a phone" on it:
    /// draw a fresh key for the badge's phone slot, seal it to the badge, write it to the
    /// badge's inbox under this phone's `name` (signed with this phone's key, which may ask
    /// for Face ID), and count the badge's circles. Show the words: the badge shows the same
    /// six, and the owner presses both buttons on the badge when they match. Then
    /// `pair_card_finish`. Starting again forgets the key of an earlier start.
    pub fn pair_card_start(&self, name: String) -> Result<PairPreview, PhoneError> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(PhoneError::Ceremony {
                message: "Enter a name for this phone.".into(),
            });
        }
        if name.len() > cahoots_badge::pairing::MAX_NAME {
            return Err(PhoneError::Ceremony {
                message: "That name is too long for the badge. Try a shorter one.".into(),
            });
        }
        self.lock().pairing = None;
        let mut key = Zeroizing::new([0u8; 16]);
        OsRng.fill_bytes(key.as_mut());
        let mine = self.my_public()?;
        let public = SeatPublic::parse(&mine).map_err(|_| PhoneError::Storage {
            message: "This phone's own key could not be read.".into(),
        })?;
        let mut tap = Tap::begin(self.nfc.as_ref(), PROMPT)?;
        let read = (|| -> Result<_, PhoneError> {
            // A badge that never opened a pairing has no pairing object yet.
            let pairing = match tap.read_pairing() {
                Err(TapError::NotFound) => Err(TapError::NoPairing),
                r => r,
            }?;
            let mut seat = signer::Seat {
                signer: self.signer.as_ref(),
                label: PHONE_KEY.into(),
                public,
                error: None,
            };
            let inbox = sealed_inbox(&mut seat, &pairing, &name, &key)?;
            if let Some(e) = seat.error.take() {
                return Err(e);
            }
            let circles = pairing.circle_slots().len();
            if circles == 0 {
                return Err(PhoneError::Nfc {
                    message: "This badge holds no circles yet. Make a circle with it first."
                        .into(),
                });
            }
            tap.write_inbox(&inbox)?;
            Ok((pairing, circles))
        })();
        let (pairing, circles) = match read {
            Ok(r) => r,
            Err(e) => {
                let _ = tap.end(&e.to_string(), true);
                return Err(e);
            }
        };
        tap.end("Check the words on the badge.", false)?;
        let words = cahoots_badge::pairing::words(&pairing.nonce, &pairing.badge_key, &mine, &key)
            .iter()
            .map(|w| w.to_string())
            .collect();
        let badge_name = match pairing.name.as_str() {
            "" => String::from(NO_BADGE_NAME),
            name => String::from(name),
        };
        self.lock().pairing = Some(crate::PendingPair {
            key,
            badge_name: badge_name.clone(),
        });
        Ok(PairPreview {
            badge_name,
            circles: circle_count(circles),
            words,
        })
    }

    /// The second tap of pairing, after the owner pressed both buttons on the badge: open the
    /// phone slot with the key from `pair_card_start`, keep each of the badge's circles as a
    /// card circle, fetch the keys its chat needs and the next [`POCKET_WORDS`] hello words
    /// with each other seat, post any bump the badge left for a device to post, and publish
    /// the badge's card in the chat. Returns the ids of the card circles.
    /// `NotPaired` while the badge has not taken this phone: the owner presses both buttons,
    /// then this is tried again.
    pub fn pair_card_finish(&self) -> Result<Vec<String>, PhoneError> {
        let (key, badge_name) = {
            let st = self.lock();
            let p = st.pairing.as_ref().ok_or_else(|| PhoneError::NotFound {
                message: "Start again: hold the phone to the card once, then press both buttons on the badge.".into(),
            })?;
            (p.key.clone(), p.badge_name.clone())
        };
        let mine = self.my_public()?;
        let read = self.tap_slot(PROMPT, &key, |s| {
            let mut out = Vec::new();
            for (slot, o) in s.list_public()? {
                if o.phone_public != Some(mine) {
                    return Err(TapError::NotPaired);
                }
                let mut keys = Vec::new();
                for g in generations(&o).unwrap_or_default() {
                    let k = s.hmac_root(slot, &generation_label(&o.circle_id, &g))?;
                    keys.push((g, Zeroizing::new(k)));
                }
                // After the keys, the hello words ahead, for a circle this tap keeps.
                let nonce = o.badge_nonce;
                let mut checked = check(o, &mine);
                if let Some(k) = checked.as_mut() {
                    if let Some((counters, mut pocket)) = self.pocket_start(k) {
                        fill(&mut pocket, slot, k.seat, k.manifest.params.n, &counters, s)?;
                        k.pocket = pocket;
                    }
                }
                out.push((slot, nonce, checked, keys));
            }
            Ok(out)
        })?;
        let Some(badge_nonce) = read.first().map(|(_, nonce, _, _)| *nonce) else {
            self.lock().pairing = None;
            return Ok(Vec::new());
        };
        let label = key_label(&badge_nonce);
        self.lock().pairing = None;
        // One badge per phone: the card circles of another stay until it is forgotten.
        let other_badge = store::circle_ids(&self.dir).iter().any(|id| {
            CircleRecord::load(&self.circle_dir(id))
                .is_ok_and(|r| r.card.as_ref().is_some_and(|card| card.key_label != label))
        });
        if other_badge {
            return Err(PhoneError::Ceremony {
                message: "Forget this card first.".into(),
            });
        }
        let mut kept = Vec::new();
        let mut first = None;
        for (slot, _, checked, keys) in read {
            let Some(k) = checked else {
                continue;
            };
            match self.keep_card_circle(slot, k, keys, &badge_name, &label) {
                Ok(Some(id)) => kept.push(id),
                Ok(None) => {}
                Err(e) => {
                    first.get_or_insert(e);
                }
            }
        }
        // The slot key is kept only for a badge with a card circle on this phone.
        if !kept.is_empty() {
            self.signer.put_secret(label, key.to_vec())?;
        }
        first.map_or(Ok(kept), Err)
    }

    /// One tap on the card for everything the card circles need: the key each chat waits for
    /// and the keys the bumps still to post move to, and the next [`POCKET_WORDS`] hello words
    /// with each other seat. The slot key is read first, which may ask for Face ID, and only
    /// then does the reader open. Nothing is kept unless the whole tap went through. Then the
    /// bumps the badge left for a device to post are sent. `NotPaired` when the badge no
    /// longer knows this phone: offer `forget_card`.
    pub fn tap_card(&self) -> Result<TapReport, PhoneError> {
        let mut st = self.lock();
        let ids = self.card_circles(&mut st);
        let label_of = |c: &Circle| c.record.card.as_ref().map(|card| card.key_label.clone());
        let Some(label) = ids.first().and_then(|id| label_of(&st.circles[id])) else {
            return Err(PhoneError::NotFound {
                message: "Pair a card first.".into(),
            });
        };
        // One tap reaches one card: the circles of the badge whose key opens its slot.
        let ids: Vec<String> = ids
            .into_iter()
            .filter(|id| label_of(&st.circles[id]).as_ref() == Some(&label))
            .collect();
        // Face ID for the key comes before the reader's sheet.
        let key = self.slot_key(&label)?;
        let mine = self.my_public()?;
        let circles = &st.circles;
        let fetched = self.tap_slot(PROMPT, &key, |s| {
            let mut out = Vec::new();
            for id in &ids {
                let c = &circles[id];
                let Some(slot) = c.record.card.as_ref().map(|card| card.circle_slot) else {
                    continue;
                };
                // Only a slot the pairing object names: the badge wrote it. A circle the badge
                // forgot, or whose slot holds another circle now, is passed over. The slot key
                // opened this session, so the badge still knows this phone.
                let o = match s.read_public(slot) {
                    Err(TapError::NotFound) => continue,
                    r => r?,
                };
                if o.circle_id != c.circle_id {
                    continue;
                }
                if o.phone_public != Some(mine) {
                    return Err(TapError::NotPaired);
                }
                let mut keys = Vec::new();
                for g in c.wanted_generations() {
                    let k = s.hmac_root(slot, &generation_label(&c.circle_id, &g))?;
                    keys.push((g, Zeroizing::new(k)));
                }
                let (pocket, added) = pocket_fill(c, s)?;
                out.push((id.clone(), keys, pocket, added));
            }
            Ok(out)
        })?;
        // The session closed without error: the circles take what it fetched.
        let mut report = TapReport {
            keys: 0,
            bumps: 0,
            words: 0,
        };
        let mut first = None;
        for (id, keys, pocket, added) in fetched {
            let Some(c) = st.circles.get_mut(&id) else {
                continue;
            };
            report.keys = report.keys.saturating_add(as_u32(keys.len()));
            report.words = report.words.saturating_add(added);
            if let Some(card) = c.record.card.as_mut() {
                card.hello_pocket = Some(pocket);
            }
            if let Err(e) = c.record.save(&c.dir).and_then(|()| c.take_keys(keys)) {
                first.get_or_insert(e);
                continue;
            }
            // The bumps the badge left for a device to post go now that their keys are here:
            // a poll reads the relay to the end, seals them into the outbox and posts them. A
            // relay out of reach leaves them for the next poll. A bump leaves the list once
            // it is sealed, or found on the relay already.
            let waiting = |c: &Circle| c.record.card.as_ref().map_or(0, |card| card.forward.len());
            let before = waiting(c);
            if before > 0 {
                let _ = c.poll(self.http.as_ref());
                let posted = before.saturating_sub(waiting(c));
                report.bumps = report.bumps.saturating_add(as_u32(posted));
            }
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut me = self.me();
        me.card_last_tap = Some(now);
        if let Err(e) = me.save(&self.dir) {
            first.get_or_insert(e);
        }
        first.map_or(Ok(report), Err)
    }

    /// What a tap on the card would do now, from the card circles on this phone. It does not
    /// tap and does not ask for Face ID.
    pub fn card_status(&self) -> Result<CardStatus, PhoneError> {
        let mut st = self.lock();
        let ids = self.card_circles(&mut st);
        let mut status = CardStatus {
            paired: !ids.is_empty(),
            badge: NO_BADGE_NAME.into(),
            circles: as_u32(ids.len()),
            keys_wanted: 0,
            bumps_to_post: 0,
            words_low: Vec::new(),
            last_tap: self.me().card_last_tap,
        };
        for (i, id) in ids.iter().enumerate() {
            let c = &st.circles[id];
            let Some(card) = c.record.card.as_ref() else {
                continue;
            };
            if i == 0 && !card.badge_name.is_empty() {
                status.badge = card.badge_name.clone();
            }
            if !c.wanted_generations().is_empty() {
                status.keys_wanted = status.keys_wanted.saturating_add(1);
            }
            status.bumps_to_post = status
                .bumps_to_post
                .saturating_add(as_u32(card.forward.len()));
            for other in (0..c.params.n).filter(|o| *o != c.record.seat) {
                let left = pocket_left(c, other);
                if left <= WORDS_LOW {
                    status.words_low.push(WordsLeft {
                        circle: id.clone(),
                        seat: other,
                        left,
                    });
                }
            }
        }
        Ok(status)
    }

    /// Forget every card circle on this phone, and the slot keys kept for their badges: for
    /// a badge that unpaired this phone, or to stop using one.
    pub fn forget_card(&self) -> Result<(), PhoneError> {
        let mut st = self.lock();
        st.pairing = None;
        let mut labels: Vec<String> = Vec::new();
        for id in store::circle_ids(&self.dir) {
            let dir = self.circle_dir(&id);
            let Ok(record) = CircleRecord::load(&dir) else {
                continue;
            };
            let Some(card) = &record.card else {
                continue;
            };
            if !labels.contains(&card.key_label) {
                labels.push(card.key_label.clone());
            }
            st.circles.remove(&id);
            std::fs::remove_dir_all(&dir).map_err(store::io)?;
        }
        drop(st);
        for label in labels {
            self.signer.put_secret(label, Vec::new())?;
        }
        Ok(())
    }
}
