//! The badge's chip over NFC: one tap at a time, through the [`Nfc`] callback (Core NFC on
//! iOS), with no state of its own beyond the tap.
//!
//! A [`Tap`] begins a reader session and selects the applet. In no session it reads what any
//! tap may read ([`Tap::read_pairing`], [`Tap::read_public`], [`Tap::list_public`]) and writes
//! the inbox ([`Tap::write_inbox`]). [`Tap::open_slot`] opens the phone slot's AESKey session
//! with the key the phone drew at pairing, under SCP03 at C-MAC, C-ENC, R-MAC and R-ENC
//! ([`crate::scp03`]), and the [`Session`] asks the chip for HMACs with a circle's root or
//! hello objects. Nothing here touches the `Phone`'s records.

use crate::apdu::{self, Apdu};
use crate::scp03::{Scp03, StaticKeys, LEVEL_FULL};
use crate::PhoneError;
use cahoots_badge::pairing::{Inbox, PairingObject};
use cahoots_badge::public::PublicObject;
use cahoots_chip::{KeyId, KeyKind};
use cahoots_core::seat::SeatSigner;
use rand_core::{OsRng, RngCore};

/// The NFC reader. Swift runs one `NFCTagReaderSession` per tap.
#[uniffi::export(callback_interface)]
pub trait Nfc: Send + Sync {
    /// Start a reader session showing `prompt`, and return once a card is connected.
    /// `PhoneError::NoCard` when none came within the session's time or the person cancelled.
    fn begin(&self, prompt: String) -> Result<(), PhoneError>;
    /// Send one command APDU and return the answer: its data, then SW1 and SW2.
    fn transceive(&self, apdu: Vec<u8>) -> Result<Vec<u8>, PhoneError>;
    /// End the reader session, showing `message`, as a failure when `failed`.
    fn end(&self, message: String, failed: bool) -> Result<(), PhoneError>;
}

/// No NFC reader: every tap finds no card.
pub struct NoNfc;

impl Nfc for NoNfc {
    fn begin(&self, _prompt: String) -> Result<(), PhoneError> {
        Err(PhoneError::NoCard)
    }
    fn transceive(&self, _apdu: Vec<u8>) -> Result<Vec<u8>, PhoneError> {
        Err(PhoneError::NoCard)
    }
    fn end(&self, _message: String, _failed: bool) -> Result<(), PhoneError> {
        Ok(())
    }
}

/// The most circle slots a badge has: public objects are tried at slots 0 to 15.
pub const CIRCLE_SLOTS: u8 = 16;
/// Seats in a circle: hello objects are `circle_slot * 8 + other_seat`.
pub const SEATS: u8 = 8;

/// Why a tap failed.
#[derive(Debug)]
pub enum TapError {
    /// No card answered.
    NoCard,
    /// The card would not open a session: it is busy with its badge (`6985` or `6982`).
    CardBusy,
    /// The card has no slot for this phone, or the slot's key is not this phone's.
    NotPaired,
    /// The object's policy refuses this session (`6986` or `6982`).
    NotAllowed,
    /// No object has that id (`6A82`).
    NotFound,
    /// What the object holds is not what was asked for.
    Malformed,
    /// The card's answer did not check: no status word, a wrong R-MAC or wrong padding.
    BadAnswer,
    /// Data too long for one command in a session.
    TooLong,
    /// The card's INITIALIZE UPDATE answer does not name SCP03.
    NotScp03,
    /// The pairing object says no pairing is open.
    NoPairing,
    /// Any other status word.
    Status(u16),
    /// The reader said so.
    Phone(PhoneError),
}

impl From<PhoneError> for TapError {
    fn from(e: PhoneError) -> Self {
        match e {
            PhoneError::NoCard => TapError::NoCard,
            PhoneError::CardBusy => TapError::CardBusy,
            e => TapError::Phone(e),
        }
    }
}

impl From<TapError> for PhoneError {
    fn from(e: TapError) -> Self {
        let nfc = |m: &str| PhoneError::Nfc { message: m.into() };
        match e {
            TapError::NoCard => PhoneError::NoCard,
            TapError::CardBusy => PhoneError::CardBusy,
            TapError::NotPaired => PhoneError::NotPaired,
            TapError::NotAllowed => nfc("The card did not allow that."),
            TapError::NotFound => nfc("The card does not hold that."),
            TapError::Malformed => nfc("The card holds something this app cannot read."),
            TapError::BadAnswer | TapError::TooLong => {
                nfc("The card’s answer was garbled. Hold the phone still and try again.")
            }
            TapError::NotScp03 => nfc("The card does not offer the secure channel this app needs."),
            TapError::NoPairing => nfc("Open “pair a phone” on your badge, then hold the phone to the card again."),
            TapError::Status(sw) => PhoneError::Nfc {
                message: format!("The card answered with code {sw:04X}. Try again."),
            },
            TapError::Phone(e) => e,
        }
    }
}

/// A status word other than `90 00`, as an error.
fn refused(sw: u16) -> TapError {
    match sw {
        0x6982 | 0x6986 => TapError::NotAllowed,
        0x6A82 => TapError::NotFound,
        sw => TapError::Status(sw),
    }
}

/// Sends a command and returns the answer's data on `90 00`.
trait Channel {
    fn exchange(&mut self, command: Apdu) -> Result<Vec<u8>, TapError>;
}

fn read_binary(ch: &mut dyn Channel, id: KeyId) -> Result<Vec<u8>, TapError> {
    let size = ch.exchange(apdu::read_size(id.0))?;
    let size = apdu::find_tlv(&size, 0x41).ok_or(TapError::BadAnswer)?;
    if size.is_empty() || size.len() > 8 {
        return Err(TapError::BadAnswer);
    }
    let size = size.iter().fold(0u64, |n, &b| n << 8 | u64::from(b));
    let size = u16::try_from(size).map_err(|_| TapError::Malformed)?;
    let mut out = Vec::with_capacity(size as usize);
    let mut offset = 0u16;
    while offset < size {
        let n = apdu::READ_CHUNK.min(size - offset);
        let a = ch.exchange(apdu::read_object(id.0, offset, n))?;
        let part = apdu::find_tlv(&a, 0x41).ok_or(TapError::BadAnswer)?;
        if part.len() != n as usize {
            return Err(TapError::BadAnswer);
        }
        out.extend_from_slice(part);
        offset += n;
    }
    Ok(out)
}

fn read_public(ch: &mut dyn Channel, circle_slot: u8) -> Result<PublicObject, TapError> {
    if circle_slot >= CIRCLE_SLOTS {
        return Err(TapError::NotFound);
    }
    let bytes = read_binary(ch, KeyId::new(KeyKind::Public, circle_slot))?;
    PublicObject::decode(&bytes).map_err(|_| TapError::Malformed)
}

fn list_public(ch: &mut dyn Channel) -> Result<Vec<(u8, PublicObject)>, TapError> {
    let mut out = Vec::new();
    for slot in 0..CIRCLE_SLOTS {
        match read_public(ch, slot) {
            Ok(o) => out.push((slot, o)),
            Err(TapError::NotFound | TapError::NotAllowed | TapError::Malformed) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// The inbox a phone holding `signer` writes for the pairing the badge has open, named
/// `name`: `key`, the phone slot's AES key, sealed to the badge's ephemeral key under a fresh
/// ephemeral key of the phone's, and everything signed ([`Inbox::sealed`]). `NoPairing` when
/// the pairing object's nonce is zero, `Malformed` when its key is not a point.
pub fn sealed_inbox<S: SeatSigner>(
    mut signer: S,
    pairing: &PairingObject,
    name: &str,
    key: &[u8; 16],
) -> Result<Inbox, TapError> {
    if pairing.nonce == [0; 16] {
        return Err(TapError::NoPairing);
    }
    // A draw that is not a valid scalar (about one in 2^32) is drawn again. Eight failures
    // mean the badge's key is not a point.
    for _ in 0..8 {
        let mut ephemeral = zeroize::Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(ephemeral.as_mut());
        if let Some(inbox) =
            Inbox::sealed(&mut signer, &pairing.nonce, &pairing.badge_key, name, key, &ephemeral)
        {
            return Ok(inbox);
        }
    }
    Err(TapError::Malformed)
}

/// One tap: a reader session with a card connected and the applet selected. Dropped without
/// [`Tap::end`], it ends the reader session as failed.
pub struct Tap<'a> {
    nfc: &'a dyn Nfc,
    ended: bool,
}

impl<'a> Tap<'a> {
    /// Start a reader session showing `prompt`, wait for a card and select the applet.
    pub fn begin(nfc: &'a dyn Nfc, prompt: &str) -> Result<Tap<'a>, TapError> {
        nfc.begin(prompt.into())?;
        let mut tap = Tap { nfc, ended: false };
        tap.select()?;
        Ok(tap)
    }

    /// Select the applet. This ends any session the card had open.
    pub fn select(&mut self) -> Result<(), TapError> {
        match self.send(&apdu::select().encode())? {
            (_, 0x9000) => Ok(()),
            // The applet is not there: this is not a badge's chip.
            (_, 0x6A82) => Err(TapError::NoCard),
            (_, sw) => Err(refused(sw)),
        }
    }

    fn send(&self, bytes: &[u8]) -> Result<(Vec<u8>, u16), TapError> {
        let answer = self.nfc.transceive(bytes.to_vec())?;
        let (data, sw) = apdu::split(&answer).ok_or(TapError::BadAnswer)?;
        Ok((data.to_vec(), sw))
    }

    /// The pairing object: the nonce of the pairing the badge has open (zeros when none) and
    /// the badge's name.
    pub fn read_pairing(&mut self) -> Result<PairingObject, TapError> {
        PairingObject::decode(&read_binary(self, KeyId::PAIRING)?).ok_or(TapError::Malformed)
    }

    /// The public object of the circle in `circle_slot` (0 to 15).
    pub fn read_public(&mut self, circle_slot: u8) -> Result<PublicObject, TapError> {
        read_public(self, circle_slot)
    }

    /// Every circle slot whose public object reads, in slot order.
    pub fn list_public(&mut self) -> Result<Vec<(u8, PublicObject)>, TapError> {
        list_public(self)
    }

    /// Write `inbox` (see [`sealed_inbox`]) over the start of the badge's inbox.
    pub fn write_inbox(&mut self, inbox: &Inbox) -> Result<(), TapError> {
        let bytes = inbox.encode();
        for (i, part) in bytes.chunks(apdu::WRITE_CHUNK).enumerate() {
            let offset = (i * apdu::WRITE_CHUNK) as u16;
            self.exchange(apdu::write_binary(KeyId::INBOX.0, offset, part))?;
        }
        Ok(())
    }

    /// Open the phone slot's session with `key`, under SCP03 at C-MAC, C-ENC, R-MAC and R-ENC.
    /// `CardBusy` when the card will not open a session, `NotPaired` when the card has no slot
    /// or its key is not `key`.
    pub fn open_slot(&mut self, key: &[u8; 16]) -> Result<Session<'_, 'a>, TapError> {
        let busy = |sw: u16| match sw {
            0x6985 | 0x6982 => TapError::CardBusy,
            0x6A82 => TapError::NotPaired,
            sw => TapError::Status(sw),
        };
        let (a, sw) = self.send(&apdu::create_session(KeyId::SLOT.0).encode())?;
        if sw != 0x9000 {
            return Err(busy(sw));
        }
        let id: [u8; 8] = apdu::find_tlv(&a, 0x41)
            .and_then(|v| v.try_into().ok())
            .ok_or(TapError::BadAnswer)?;
        let mut host = [0u8; 8];
        OsRng.fill_bytes(&mut host);
        let init = apdu::initialize_update(&host).encode();
        let (a, sw) = self.send(&apdu::process_session(&id, &init).encode())?;
        if sw != 0x9000 {
            return Err(busy(sw));
        }
        if a.len() < 29 {
            return Err(TapError::BadAnswer);
        }
        // Key information: key version, the protocol (03 for SCP03), its parameter.
        if a[11] != 0x03 {
            return Err(TapError::NotScp03);
        }
        let card: [u8; 8] = a[13..21].try_into().expect("8 bytes");
        let cryptogram: [u8; 8] = a[21..29].try_into().expect("8 bytes");
        let mut scp = Scp03::new(&StaticKeys::single(key), &host, &card);
        if !scp.check_card_cryptogram(&cryptogram) {
            return Err(TapError::NotPaired);
        }
        let auth = scp.external_authenticate(LEVEL_FULL);
        let (_, sw) = self.send(&apdu::process_session(&id, &auth).encode())?;
        match sw {
            0x9000 => Ok(Session { tap: self, id, scp, closed: false }),
            0x6982 => Err(TapError::NotPaired),
            sw => Err(busy(sw)),
        }
    }

    /// End the reader session, showing `message`.
    pub fn end(mut self, message: &str, failed: bool) -> Result<(), TapError> {
        self.ended = true;
        self.nfc.end(message.into(), failed)?;
        Ok(())
    }
}

impl Channel for Tap<'_> {
    fn exchange(&mut self, command: Apdu) -> Result<Vec<u8>, TapError> {
        match self.send(&command.encode())? {
            (data, 0x9000) => Ok(data),
            (_, sw) => Err(refused(sw)),
        }
    }
}

impl Drop for Tap<'_> {
    fn drop(&mut self) {
        if !self.ended {
            let _ = self.nfc.end(String::new(), true);
        }
    }
}

/// The phone slot's session, under SCP03. Dropped without [`Session::close`], it closes the
/// session if the card still answers.
pub struct Session<'t, 'a> {
    tap: &'t mut Tap<'a>,
    id: [u8; 8],
    scp: Scp03,
    closed: bool,
}

impl Session<'_, '_> {
    /// `HMAC-SHA256` with the chip's HMAC object `key` over `data`.
    pub fn mac(&mut self, key: KeyId, data: &[u8]) -> Result<[u8; 32], TapError> {
        let a = self.exchange(apdu::mac_one_shot(key.0, data))?;
        apdu::find_tlv(&a, 0x41)
            .and_then(|m| m.try_into().ok())
            .ok_or(TapError::BadAnswer)
    }

    /// The HMAC of `data` with the root of the circle in `circle_slot`.
    pub fn hmac_root(&mut self, circle_slot: u8, data: &[u8]) -> Result<[u8; 32], TapError> {
        if circle_slot >= CIRCLE_SLOTS {
            return Err(TapError::NotFound);
        }
        self.mac(KeyId::new(KeyKind::Root, circle_slot), data)
    }

    /// The HMAC of `data` with the hello object the circle in `circle_slot` keeps for the seat
    /// `other`.
    pub fn hmac_hello(&mut self, circle_slot: u8, other: u8, data: &[u8]) -> Result<[u8; 32], TapError> {
        if circle_slot >= CIRCLE_SLOTS || other >= SEATS {
            return Err(TapError::NotFound);
        }
        self.mac(KeyId::new(KeyKind::Hello, circle_slot * SEATS + other), data)
    }

    /// The public object of the circle in `circle_slot`, read in the session.
    pub fn read_public(&mut self, circle_slot: u8) -> Result<PublicObject, TapError> {
        read_public(self, circle_slot)
    }

    /// Every circle slot whose public object reads, read in the session.
    pub fn list_public(&mut self) -> Result<Vec<(u8, PublicObject)>, TapError> {
        list_public(self)
    }

    /// Close the session. The tap stays open, in no session.
    pub fn close(mut self) -> Result<(), TapError> {
        self.closed = true;
        self.exchange(apdu::close_session()).map(|_| ())
    }
}

impl Channel for Session<'_, '_> {
    fn exchange(&mut self, c: Apdu) -> Result<Vec<u8>, TapError> {
        if c.data.len() > apdu::MAX_SESSION_DATA {
            return Err(TapError::TooLong);
        }
        let inner = self.scp.wrap(c.cla, c.ins, c.p1, c.p2, &c.data, c.le.map(|_| 0));
        let (data, sw) = self.tap.send(&apdu::process_session(&self.id, &inner).encode())?;
        if sw != 0x9000 {
            return Err(refused(sw));
        }
        self.scp.unwrap(&data, sw).map_err(|_| TapError::BadAnswer)
    }
}

impl Drop for Session<'_, '_> {
    fn drop(&mut self) {
        if !self.closed {
            self.closed = true;
            let _ = self.exchange(apdu::close_session());
        }
    }
}
