//! The phone's chip client over a fake NFC reader whose card is the software chip's APDU
//! front end (`SoftChip::apdu`), set up by the badge's own chip code: the real SCP03 handshake
//! and secure messaging, and the policies the badge writes.

use cahoots_badge::chip::{first_boot, replace_slot, write_pairing};
use cahoots_badge::pairing::{words, Inbox};
use cahoots_badge::public::PublicObject;
use cahoots_chip::{ChipOps, Curve, KeyId, KeyKind, ObjectPolicy, SoftChip, TEST_OWNER_SECRET};
use cahoots_core::seat::SeatKey;
use cahoots_phone::apdu;
use cahoots_phone::nfc::{Nfc, Tap, TapError};
use cahoots_phone::PhoneError;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::sync::{Arc, Mutex};

const PHONE_KEY: [u8; 16] = *b"phone-slot-key-1";
const ROOT: [u8; 32] = [0x52; 32];
const HELLO: [u8; 32] = [0x48; 32];
const NONCE: [u8; 16] = [0x4E; 16];
const CIRCLE: [u8; 16] = [0xC1; 16];

#[derive(Default)]
struct Card {
    /// `None`: no tag in the field.
    chip: Option<SoftChip>,
    /// Flip a bit of the answer to the `n`th command from now.
    garble: Option<usize>,
    ended: Vec<(String, bool)>,
}

/// A reader whose card is a software chip.
#[derive(Clone, Default)]
struct FakeNfc(Arc<Mutex<Card>>);

impl FakeNfc {
    fn with(chip: SoftChip) -> FakeNfc {
        let f = FakeNfc::default();
        f.0.lock().unwrap().chip = Some(chip);
        f
    }

    fn chip<T>(&self, f: impl FnOnce(&mut SoftChip) -> T) -> T {
        f(self.0.lock().unwrap().chip.as_mut().unwrap())
    }
}

impl Nfc for FakeNfc {
    fn begin(&self, _prompt: String) -> Result<(), PhoneError> {
        match self.0.lock().unwrap().chip {
            Some(_) => Ok(()),
            None => Err(PhoneError::NoCard),
        }
    }

    fn transceive(&self, apdu: Vec<u8>) -> Result<Vec<u8>, PhoneError> {
        let mut card = self.0.lock().unwrap();
        let mut answer = card.chip.as_mut().ok_or(PhoneError::NoCard)?.apdu(&apdu);
        card.garble = match card.garble {
            Some(0) => {
                answer[0] ^= 1;
                None
            }
            Some(n) => Some(n - 1),
            None => None,
        };
        Ok(answer)
    }

    fn end(&self, message: String, failed: bool) -> Result<(), PhoneError> {
        self.0.lock().unwrap().ended.push((message, failed));
        Ok(())
    }
}

fn hmac(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).unwrap();
    m.update(data);
    m.finalize().into_bytes().into()
}

fn public_object(phone: Option<[u8; 33]>) -> PublicObject {
    PublicObject {
        circle_id: CIRCLE,
        relay: String::new(),
        manifest: vec![0x5A; 900],
        card: None,
        grant: Some(vec![0x47; 300]),
        generation: [0x33; 32],
        pending_bumps: Vec::new(),
        badge_nonce: [0x0B; 16],
        phone_public: phone,
    }
}

/// A chip as a badge leaves it: owner object, placeholder slot, inbox and an open pairing,
/// one circle in slot 3 with its seat key, root, a hello object for seat 1 and its public
/// object, and with `paired`, the phone's key in the slot. The owner session is closed, as
/// at the end of every I2C conversation.
fn badge_chip(paired: bool) -> SoftChip {
    let mut chip = SoftChip::with_seed([9; 32]);
    first_boot(&mut chip, &TEST_OWNER_SECRET, [0x0B; 16]).unwrap();
    write_pairing(&mut chip, &NONCE).unwrap();
    chip.generate(KeyId::new(KeyKind::Seat, 3), Curve::P256, &ObjectPolicy::owner_only()).unwrap();
    chip.put_hmac_key(KeyId::new(KeyKind::Root, 3), &ROOT, &ObjectPolicy::owner_and_slot_mac())
        .unwrap();
    chip.put_hmac_key(KeyId::new(KeyKind::Hello, 3 * 8 + 1), &HELLO, &ObjectPolicy::owner_and_slot_mac())
        .unwrap();
    let o = public_object(None);
    let bytes = o.padded(o.new_len().unwrap()).unwrap();
    chip.write_binary(KeyId::new(KeyKind::Public, 3), &bytes, &ObjectPolicy::public_object())
        .unwrap();
    if paired {
        replace_slot(&mut chip, Some(&PHONE_KEY)).unwrap();
    }
    chip.close_session().unwrap();
    chip
}

#[test]
fn a_tap_in_no_session_lists_public_objects_and_writes_the_inbox() {
    let nfc = FakeNfc::with(badge_chip(false));
    let mut tap = Tap::begin(&nfc, "Hold the phone to the card").unwrap();
    let pairing = tap.read_pairing().unwrap();
    assert_eq!(pairing.nonce, NONCE);
    let list = tap.list_public().unwrap();
    assert_eq!(list, vec![(3, public_object(None))]);

    let phone = SeatKey::from_seed(&[5; 32]).unwrap();
    let inbox = Inbox::signed(&phone, &pairing.nonce, "Conner's phone", PHONE_KEY);
    tap.write_inbox(&inbox).unwrap();
    // A tap in no session may not MAC with a root.
    assert_eq!(tap_mac_without_session(&nfc, KeyId::new(KeyKind::Root, 3)), 0x6986);
    tap.end("Done", false).unwrap();
    assert_eq!(nfc.0.lock().unwrap().ended, vec![("Done".to_string(), false)]);

    // The badge reads what the phone wrote, and both show the same words.
    let read = nfc.chip(|c| {
        c.open_owner(&TEST_OWNER_SECRET).unwrap();
        let b = c.read_binary(KeyId::INBOX).unwrap();
        c.close_session().unwrap();
        b
    });
    let got = Inbox::decode(&read).unwrap();
    assert!(got.verify(&NONCE));
    let p = phone;
    assert_eq!(got.public, p.public().0);
    assert_eq!(
        words(&NONCE, &got.public, &got.key),
        words(&NONCE, &p.public().0, &PHONE_KEY)
    );
}

/// The status word of a MACOneShot sent outside any session.
fn tap_mac_without_session(nfc: &FakeNfc, id: KeyId) -> u16 {
    let a = nfc.transceive(apdu::mac_one_shot(id.0, b"x").encode()).unwrap();
    apdu::split(&a).unwrap().1
}

#[test]
fn the_paired_key_opens_the_slot_and_the_chip_macs_under_secure_messaging() {
    let nfc = FakeNfc::with(badge_chip(true));
    let mut tap = Tap::begin(&nfc, "Hold the phone to the card").unwrap();
    let mut s = tap.open_slot(&PHONE_KEY).unwrap();

    let generation = [7u8; 32];
    let mut gen_input = b"chat/gen".to_vec();
    gen_input.extend_from_slice(&CIRCLE);
    gen_input.extend_from_slice(&generation);
    assert_eq!(s.hmac_root(3, &gen_input).unwrap(), hmac(&ROOT, &gen_input));

    let mut seal_input = b"seal/card".to_vec();
    seal_input.extend_from_slice(&CIRCLE);
    assert_eq!(s.hmac_root(3, &seal_input).unwrap(), hmac(&ROOT, &seal_input));

    let mut hello_input = b"cahoots/words/v1".to_vec();
    hello_input.extend_from_slice(&[2]);
    hello_input.extend_from_slice(&5u64.to_le_bytes());
    hello_input.push(1);
    assert_eq!(s.hmac_hello(3, 1, &hello_input).unwrap(), hmac(&HELLO, &hello_input));

    // The public object reads in the session too, under secure messaging.
    assert_eq!(s.read_public(3).unwrap(), public_object(None));
    // No root in slot 4.
    assert!(matches!(s.hmac_root(4, b"x"), Err(TapError::NotFound)));
    s.close().unwrap();

    // After the close the tap is in no session again.
    assert_eq!(tap_mac_without_session(&nfc, KeyId::new(KeyKind::Root, 3)), 0x6986);
    tap.end("Done", false).unwrap();
}

#[test]
fn a_wrong_key_does_not_open_the_slot() {
    let nfc = FakeNfc::with(badge_chip(true));
    let mut tap = Tap::begin(&nfc, "Hold the phone to the card").unwrap();
    let e = tap.open_slot(b"not-the-slot-key").err().unwrap();
    assert!(matches!(e, TapError::NotPaired));
    assert_eq!(PhoneError::from(e).to_string(), "This card no longer knows this phone.");
}

#[test]
fn an_unpaired_badge_does_not_open_the_slot() {
    let nfc = FakeNfc::with(badge_chip(false));
    let mut tap = Tap::begin(&nfc, "Hold the phone to the card").unwrap();
    assert!(matches!(tap.open_slot(&PHONE_KEY), Err(TapError::NotPaired)));
}

#[test]
fn the_slot_may_not_mac_with_a_seat_key() {
    let nfc = FakeNfc::with(badge_chip(true));
    let mut tap = Tap::begin(&nfc, "Hold the phone to the card").unwrap();
    let mut s = tap.open_slot(&PHONE_KEY).unwrap();
    let e = s.mac(KeyId::new(KeyKind::Seat, 3), b"x").err().unwrap();
    assert!(matches!(e, TapError::NotAllowed));
    // Nor read the inbox.
    assert!(matches!(s.mac(KeyId::INBOX, b"x"), Err(TapError::NotAllowed)));
    // The session still works after a refusal.
    assert_eq!(s.hmac_root(3, b"x").unwrap(), hmac(&ROOT, b"x"));
}

#[test]
fn a_card_busy_with_its_badge_says_so() {
    let mut chip = badge_chip(true);
    chip.open_owner(&TEST_OWNER_SECRET).unwrap();
    let nfc = FakeNfc::with(chip);
    let mut tap = Tap::begin(&nfc, "Hold the phone to the card").unwrap();
    let e = tap.open_slot(&PHONE_KEY).err().unwrap();
    assert!(matches!(e, TapError::CardBusy));
    assert_eq!(
        PhoneError::from(e).to_string(),
        "The card is busy with its badge. Try again in a moment."
    );
}

#[test]
fn no_tag_is_no_card() {
    let nfc = FakeNfc::default();
    let e = Tap::begin(&nfc, "Hold the phone to the card").err().unwrap();
    assert!(matches!(e, TapError::NoCard));
    assert_eq!(PhoneError::from(e).to_string(), "No card found. Hold the phone to the chip.");
}

#[test]
fn a_changed_answer_in_the_session_is_refused() {
    let nfc = FakeNfc::with(badge_chip(true));
    let mut tap = Tap::begin(&nfc, "Hold the phone to the card").unwrap();
    let mut s = tap.open_slot(&PHONE_KEY).unwrap();
    nfc.0.lock().unwrap().garble = Some(0);
    assert!(matches!(s.hmac_root(3, b"x"), Err(TapError::BadAnswer)));
}

#[test]
fn a_tap_dropped_without_ending_ends_as_failed() {
    let nfc = FakeNfc::with(badge_chip(false));
    {
        let _tap = Tap::begin(&nfc, "Hold the phone to the card").unwrap();
    }
    assert_eq!(nfc.0.lock().unwrap().ended, vec![(String::new(), true)]);
}
