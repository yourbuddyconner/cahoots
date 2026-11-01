//! Card circles: the circles of a badge this phone is paired to.
//!
//! Pairing takes two taps. The first ([`Phone::pair_card_start`]) draws a fresh slot key,
//! seals it to the key the badge's open pairing shows, writes it to the badge's inbox signed
//! with this phone's own key, and counts the badge's circles from its pairing object. Their
//! public objects are not readable before the press: only the badge and its paired phone
//! read them. The owner checks the six words
//! on both screens and presses both buttons on the badge. The second tap
//! ([`Phone::pair_card_finish`]) opens the phone slot with that key, which works only once the
//! badge took it, keeps each circle as a card circle, fetches the keys its chat needs, posts
//! any bump the badge left for a device to post, and publishes the badge's card in the chat.
//!
//! A card circle holds no pad, no root, no pair keys and no seat key. It chats as the badge's
//! seat under one generation's key. When a bump moves the circle on, the chat waits
//! (`needs_tap`) until a tap ([`Phone::tap_for_keys`]) gives the next key. Hello words come
//! from a tap too. The slot key is kept in the keychain under `cahoots:card:` and the badge's
//! nonce, behind Face ID.

use crate::chat::{self, Circle};
use crate::nfc::{sealed_inbox, Session, Tap, TapError};
use crate::store::{self, CardCircle, ChatRecord, CircleRecord, SettingsRecord};
use crate::{signer, HelloWords, PairPreview, Phone, PhoneError, PHONE_KEY};
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

/// What the reader says while a card circle's chat waits for keys.
const KEYS_PROMPT: &str = "Hold the phone to the card to fetch keys.";

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

/// A circle of the badge, read from its public object at the second tap, checked.
struct Kept {
    object: PublicObject,
    manifest: Manifest,
    seat: u8,
    badge_public: SeatPublic,
    /// The generation the chat starts in, then each one a bump moves to.
    generations: Vec<[u8; 32]>,
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

    /// The hello words with `other` at `number` in a card circle, from a tap.
    pub(crate) fn card_hello(
        &self,
        c: &Circle,
        other: u8,
        number: u32,
    ) -> Result<HelloWords, PhoneError> {
        let card = c.record.card.as_ref().ok_or(PhoneError::NotPaired)?;
        let key = self.slot_key(&card.key_label)?;
        let (slot, me) = (card.circle_slot, c.record.seat);
        let (mine, theirs) = self.tap_slot(PROMPT, &key, |s| {
            Ok((
                s.hmac_hello(slot, other, &hello_label(me, number))?,
                s.hmac_hello(slot, other, &hello_label(other, number))?,
            ))
        })?;
        Ok(HelloWords {
            mine: hello_words(&mine),
            theirs: hello_words(&theirs),
            number,
        })
    }

    /// Keep one circle of the badge as a card circle, or bring a card circle already held up
    /// to the pairing, then read its relay, post the bumps it forwards, and publish the
    /// badge's card in its chat. `None` when the phone holds the circle with a seat of its own.
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
    /// card circle, fetch the keys its chat needs, post any bump the badge left for a device
    /// to post, and publish the badge's card in the chat. Returns the ids of the card circles.
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
                out.push((slot, o, keys));
            }
            Ok(out)
        })?;
        let Some(badge_nonce) = read.first().map(|(_, o, _)| o.badge_nonce) else {
            self.lock().pairing = None;
            return Ok(Vec::new());
        };
        let label = key_label(&badge_nonce);
        self.lock().pairing = None;
        let mut kept = Vec::new();
        let mut first = None;
        for (slot, o, keys) in read {
            let Some(k) = check(o, &mine) else {
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

    /// Tap the card for the keys a card circle's chat waits for (`needs_tap`), and for the
    /// bumps it still has to post. With nothing to fetch it does not tap. `NotPaired` when
    /// the badge no longer knows this phone:
    /// offer `forget_card`.
    pub fn tap_for_keys(&self, circle: String) -> Result<(), PhoneError> {
        self.with_circle(&circle, |c, me| {
            let card = c.record.card.clone().ok_or_else(|| PhoneError::NotFound {
                message: "This circle is not from a badge.".into(),
            })?;
            let wanted = c.wanted_generations();
            // Nothing to fetch: no tap, and no Face ID for the key.
            if wanted.is_empty() {
                return Ok(());
            }
            let key = me.slot_key(&card.key_label)?;
            let mine = me.my_public()?;
            let circle_id = c.circle_id;
            let slot = card.circle_slot;
            let keys = me.tap_slot(KEYS_PROMPT, &key, |s| {
                // Only a slot the pairing object names: the badge wrote it.
                let o = s.read_public(slot)?;
                if o.circle_id != circle_id || o.phone_public != Some(mine) {
                    return Err(TapError::NotPaired);
                }
                let mut keys = Vec::new();
                for g in wanted {
                    let k = s.hmac_root(slot, &generation_label(&circle_id, &g))?;
                    keys.push((g, Zeroizing::new(k)));
                }
                Ok(keys)
            })?;
            c.take_keys(keys)
        })
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
