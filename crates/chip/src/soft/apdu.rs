//! The software chip's APDU front end: the commands a phone sends over NFC, decoded as the
//! SE05x applet decodes them and routed into the same [`SoftChip`] the badge tests use, with
//! the identity of the session they arrive in.
//!
//! The commands (NXP AN12413, the SE05x APDU specification; the TLV tags and INS, P1 and P2
//! values as Nitrokey's `se05x` 0.4.2 encodes them):
//!
//! | command | CLA INS P1 P2 | data |
//! |---|---|---|
//! | SELECT | `00 A4 04 00` | the applet id |
//! | CreateSession | `80 04 00 1B` | `41` object id (4) |
//! | ProcessSessionCmd | `80 05 00 00` | `10` session id (8), `41` the command |
//! | INITIALIZE UPDATE | `80 50 00 00` | host challenge (8), in the session |
//! | EXTERNAL AUTHENTICATE | `84 82 lvl 00` | host cryptogram (8), C-MAC (8), in the session |
//! | CloseSession | `80 04 00 1C` | none |
//! | ReadObject | `80 02 00 00` | `41` id, `42` offset (2), `43` length (2) |
//! | ReadSize | `80 02 00 07` | `41` id |
//! | WriteBinary | `80 01 06 00` | `11` policy, `41` id, `42` offset (2), `43` file length (2), `44` data |
//! | MACOneShot | `80 03 0D 45` | `41` key id, `42` algorithm (`19` HMAC-SHA256), `43` data |
//!
//! A session opened against the phone slot's AES object runs SCP03 ([`super::scp03`]), and a
//! command inside it, once `EXTERNAL AUTHENTICATE` has passed, must carry a valid C-MAC (and be
//! encrypted at a level with C-DECRYPTION); its answer is encrypted and MACed at the level.
//! Such a command runs as the slot with secure messaging. A command outside a session runs in
//! no session. A wrong MAC, or a failed `EXTERNAL AUTHENTICATE`, ends the session with `6982`.
//!
//! Status words: `90 00`; `6986` for a policy refusal (the applet's answer on the bench);
//! `6982` for a session that failed secure messaging; `6985` for a session that cannot be
//! opened (the chip is in its badge's owner session, or the object is not an AES key) or a
//! command out of order in one; `6A82` for no such object; `6A89` for an id in use; `6A80` for
//! bad data; `6700` for a bad length; `6D00` for a command it does not know; `6E00` before the
//! applet is selected.
//!
//! Not modelled: a policy in WriteBinary (a phone never makes objects; it is refused with
//! `6A80`), and the user-id session, which only the badge opens, over I2C.

use super::scp03::Card;
use super::{read_rights, sign_rights, write_rights, Object, SoftChip};
use crate::ids::KeyId;
use crate::ops::{ChipError, ChipOps, ObjectPolicy, Session};
use alloc::vec;
use alloc::vec::Vec;
use rand_core::RngCore;

/// The SE05x IoT applet's id.
pub const APPLET_ID: [u8; 16] = [
    0xA0, 0x00, 0x00, 0x03, 0x96, 0x54, 0x53, 0x00, 0x00, 0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00,
];

/// What SELECT answers: the applet version 7.2.0, its configuration and the secure box version.
const SELECT_ANSWER: [u8; 7] = [7, 2, 0, 0x3F, 0xFF, 0x01, 0x0B];

const SW_OK: u16 = 0x9000;
const SW_WRONG_LENGTH: u16 = 0x6700;
const SW_SM_FAILED: u16 = 0x6982;
const SW_CONDITIONS: u16 = 0x6985;
const SW_NOT_ALLOWED: u16 = 0x6986;
const SW_WRONG_DATA: u16 = 0x6A80;
const SW_NOT_FOUND: u16 = 0x6A82;
const SW_EXISTS: u16 = 0x6A89;
const SW_UNKNOWN_INS: u16 = 0x6D00;
const SW_NOT_SELECTED: u16 = 0x6E00;

const HMAC_SHA256: u8 = 0x19;

/// An NFC session the phone opened with CreateSession.
pub(super) struct NfcSession {
    id: [u8; 8],
    key: [u8; 16],
    stage: Stage,
}

enum Stage {
    /// Created: INITIALIZE UPDATE comes next.
    Created,
    /// INITIALIZE UPDATE answered: EXTERNAL AUTHENTICATE comes next.
    Challenged(Card),
    /// Authenticated: commands under secure messaging.
    Open(Card),
}

/// A command APDU, short or extended.
struct Command<'a> {
    cla: u8,
    ins: u8,
    p1: u8,
    p2: u8,
    /// `CLA INS P1 P2` and Lc as sent, for the C-MAC.
    header: &'a [u8],
    data: &'a [u8],
}

fn parse(apdu: &[u8]) -> Option<Command<'_>> {
    if apdu.len() < 4 {
        return None;
    }
    let (cla, ins, p1, p2) = (apdu[0], apdu[1], apdu[2], apdu[3]);
    let rest = &apdu[4..];
    let (lc_len, lc) = match rest.len() {
        // Case 1 or case 2 short.
        0 | 1 => (0, 0),
        // Case 2 extended: 00 Le Le.
        3 if rest[0] == 0 => (0, 0),
        _ if rest[0] != 0 => (1, rest[0] as usize),
        _ if rest.len() >= 3 => (3, u16::from_be_bytes([rest[1], rest[2]]) as usize),
        _ => return None,
    };
    if lc_len == 0 {
        return Some(Command { cla, ins, p1, p2, header: &apdu[..4], data: &[] });
    }
    let data_end = lc_len + lc;
    let tail = rest.len().checked_sub(data_end)?;
    // Le: none, one byte (short) or two (extended).
    if !(tail == 0 || (lc_len == 1 && tail == 1) || (lc_len == 3 && tail == 2)) {
        return None;
    }
    Some(Command {
        cla,
        ins,
        p1,
        p2,
        header: &apdu[..4 + lc_len],
        data: &rest[lc_len..data_end],
    })
}

/// The TLVs of a command's data, by tag; `None` if they do not parse.
fn tlvs(mut d: &[u8]) -> Option<Vec<(u8, &[u8])>> {
    let mut out = Vec::new();
    while !d.is_empty() {
        if d.len() < 2 {
            return None;
        }
        let tag = d[0];
        let (n, skip) = match d[1] {
            n @ 0..=0x7F => (n as usize, 2),
            0x81 if d.len() >= 3 => (d[2] as usize, 3),
            0x82 if d.len() >= 4 => (u16::from_be_bytes([d[2], d[3]]) as usize, 4),
            _ => return None,
        };
        let v = d.get(skip..skip + n)?;
        out.push((tag, v));
        d = &d[skip + n..];
    }
    Some(out)
}

fn find<'a>(t: &[(u8, &'a [u8])], tag: u8) -> Option<&'a [u8]> {
    t.iter().find(|(k, _)| *k == tag).map(|(_, v)| *v)
}

fn tlv(tag: u8, v: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match v.len() {
        n @ 0..=0x7F => out.push(n as u8),
        n @ 0x80..=0xFF => out.extend_from_slice(&[0x81, n as u8]),
        n => {
            out.push(0x82);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
    }
    out.extend_from_slice(v);
    out
}

fn object_id(v: Option<&[u8]>) -> Option<KeyId> {
    Some(KeyId(u32::from_be_bytes(v?.try_into().ok()?)))
}

fn u16_of(v: Option<&[u8]>) -> Option<Option<usize>> {
    match v {
        None => Some(None),
        Some(b) => Some(Some(u16::from_be_bytes(b.try_into().ok()?) as usize)),
    }
}

fn status(e: ChipError) -> u16 {
    match e {
        ChipError::NotAllowed => SW_NOT_ALLOWED,
        ChipError::NoSuchKey => SW_NOT_FOUND,
        ChipError::Exists => SW_EXISTS,
        ChipError::BadSession => SW_CONDITIONS,
        ChipError::Status(s) => s,
        ChipError::NoChip | ChipError::Transport | ChipError::Busy => 0x6F00,
    }
}

fn answer(mut data: Vec<u8>, sw: u16) -> Vec<u8> {
    data.extend_from_slice(&sw.to_be_bytes());
    data
}

impl SoftChip {
    /// Answer one command APDU as the SE05x applet would over NFC: the response data and the
    /// status word. See the module docs for what it decodes.
    pub fn apdu(&mut self, apdu: &[u8]) -> Vec<u8> {
        let Some(c) = parse(apdu) else {
            return answer(Vec::new(), SW_WRONG_LENGTH);
        };
        if c.cla == 0x00 && c.ins == 0xA4 && c.p1 == 0x04 {
            // Selecting the applet ends any session the phone had.
            self.nfc = None;
            if c.data == APPLET_ID {
                self.selected = true;
                return answer(SELECT_ANSWER.to_vec(), SW_OK);
            }
            return answer(Vec::new(), SW_NOT_FOUND);
        }
        if !self.selected {
            return answer(Vec::new(), SW_NOT_SELECTED);
        }
        match (c.cla, c.ins, c.p1, c.p2) {
            (0x80, 0x04, 0x00, 0x1B) => self.create_session(c.data),
            (0x80, 0x05, 0x00, 0x00) => self.process_session(c.data),
            (0x80, 0x04, 0x00, 0x1C) => answer(Vec::new(), SW_OK),
            (0x80, ..) => {
                let (data, sw) = self.run_as(Session::None, &c);
                answer(data, sw)
            }
            _ => answer(Vec::new(), SW_UNKNOWN_INS),
        }
    }

    fn create_session(&mut self, data: &[u8]) -> Vec<u8> {
        let Some(id) = tlvs(data).and_then(|t| object_id(find(&t, 0x41))) else {
            return answer(Vec::new(), SW_WRONG_DATA);
        };
        // The chip serves its badge's session over I2C: no session for a tap.
        if self.session == Session::Owner {
            return answer(Vec::new(), SW_CONDITIONS);
        }
        let key = match self.objects.get(&id.0) {
            None => return answer(Vec::new(), SW_NOT_FOUND),
            Some(e) => match &e.object {
                Object::Aes(k) => *k,
                _ => return answer(Vec::new(), SW_CONDITIONS),
            },
        };
        let mut sid = [0u8; 8];
        self.rng.fill_bytes(&mut sid);
        self.nfc = Some(NfcSession { id: sid, key, stage: Stage::Created });
        answer(tlv(0x41, &sid), SW_OK)
    }

    fn process_session(&mut self, data: &[u8]) -> Vec<u8> {
        let Some(t) = tlvs(data) else {
            return answer(Vec::new(), SW_WRONG_DATA);
        };
        let (Some(sid), Some(inner)) = (find(&t, 0x10), find(&t, 0x41)) else {
            return answer(Vec::new(), SW_WRONG_DATA);
        };
        let Some(mut s) = self.nfc.take() else {
            return answer(Vec::new(), SW_CONDITIONS);
        };
        if s.id != sid {
            self.nfc = Some(s);
            return answer(Vec::new(), SW_CONDITIONS);
        }
        let Some(c) = parse(inner) else {
            self.nfc = Some(s);
            return answer(Vec::new(), SW_WRONG_LENGTH);
        };
        match s.stage {
            Stage::Created => {
                if (c.cla, c.ins) != (0x80, 0x50) || c.data.len() != 8 {
                    self.nfc = Some(s);
                    return answer(Vec::new(), SW_CONDITIONS);
                }
                let mut challenge = [0u8; 8];
                self.rng.fill_bytes(&mut challenge);
                let card = Card::new(&s.key, c.data, &challenge);
                let mut out = vec![0u8; 10];
                out.extend_from_slice(&[0x00, 0x03, 0x00]);
                out.extend_from_slice(&challenge);
                out.extend_from_slice(&card.card_cryptogram());
                s.stage = Stage::Challenged(card);
                self.nfc = Some(s);
                answer(out, SW_OK)
            }
            Stage::Challenged(mut card) => {
                if !card.external_authenticate(inner) {
                    // A host that does not hold the key ends the session.
                    return answer(Vec::new(), SW_SM_FAILED);
                }
                s.stage = Stage::Open(card);
                self.nfc = Some(s);
                answer(Vec::new(), SW_OK)
            }
            Stage::Open(mut card) => {
                if c.cla != 0x84 {
                    return answer(Vec::new(), SW_SM_FAILED);
                }
                let Some(plain) = card.open_command(c.header, c.data) else {
                    return answer(Vec::new(), SW_SM_FAILED);
                };
                let cmd = Command { cla: 0x80, ins: c.ins, p1: c.p1, p2: c.p2, header: &[], data: &plain };
                if (cmd.ins, cmd.p1, cmd.p2) == (0x04, 0x00, 0x1C) {
                    return answer(card.seal_response(&[]), SW_OK);
                }
                let (data, sw) = self.run_as(Session::Slot { scp03: true }, &cmd);
                let out = if sw == SW_OK { card.seal_response(&data) } else { Vec::new() };
                s.stage = Stage::Open(card);
                self.nfc = Some(s);
                answer(out, sw)
            }
        }
    }

    /// Run a plain command in `session`, leaving the chip's own session as it was.
    fn run_as(&mut self, session: Session, c: &Command<'_>) -> (Vec<u8>, u16) {
        let saved = self.session;
        self.session = session;
        let r = self.command(c);
        self.session = saved;
        match r {
            Ok(data) => (data, SW_OK),
            Err(sw) => (Vec::new(), sw),
        }
    }

    fn command(&mut self, c: &Command<'_>) -> Result<Vec<u8>, u16> {
        let t = tlvs(c.data).ok_or(SW_WRONG_DATA)?;
        let id = || object_id(find(&t, 0x41)).ok_or(SW_WRONG_DATA);
        match (c.ins, c.p1, c.p2) {
            // ReadObject.
            (0x02, 0x00, 0x00) => {
                let offset = u16_of(find(&t, 0x42)).ok_or(SW_WRONG_DATA)?.unwrap_or(0);
                let length = u16_of(find(&t, 0x43)).ok_or(SW_WRONG_DATA)?;
                let b = self.read_binary(id()?).map_err(status)?;
                let end = length.map_or(b.len(), |n| offset + n);
                let part = b.get(offset..end).ok_or(SW_WRONG_DATA)?;
                Ok(tlv(0x41, part))
            }
            // ReadSize.
            (0x02, 0x00, 0x07) => match self.get(id()?, read_rights).map_err(status)? {
                Object::Binary(b) => Ok(tlv(0x41, &(b.len() as u16).to_be_bytes())),
                _ => Err(SW_WRONG_DATA),
            },
            // WriteBinary.
            (0x01, 0x06, 0x00) => {
                if find(&t, 0x11).is_some() {
                    return Err(SW_WRONG_DATA);
                }
                let id = id()?;
                let offset = u16_of(find(&t, 0x42)).ok_or(SW_WRONG_DATA)?.unwrap_or(0);
                let file_length = u16_of(find(&t, 0x43)).ok_or(SW_WRONG_DATA)?;
                let data = find(&t, 0x44).unwrap_or(&[]);
                if !self.objects.contains_key(&id.0) {
                    // A new object with the applet's default policy: every right for anyone.
                    let n = file_length.ok_or(SW_NOT_FOUND)?;
                    let mut body = vec![0u8; n];
                    body.get_mut(offset..offset + data.len()).ok_or(SW_WRONG_DATA)?.copy_from_slice(data);
                    return self
                        .write_binary(id, &body, &ObjectPolicy::any_session())
                        .map(|()| Vec::new())
                        .map_err(status);
                }
                if file_length.is_some() {
                    return Err(SW_WRONG_DATA);
                }
                match self.get(id, write_rights).map_err(status)? {
                    Object::Binary(b) => {
                        b.get_mut(offset..offset + data.len())
                            .ok_or(SW_WRONG_DATA)?
                            .copy_from_slice(data);
                        Ok(Vec::new())
                    }
                    _ => Err(SW_WRONG_DATA),
                }
            }
            // MACOneShot, generate.
            (0x03, 0x0D, 0x45) => {
                if find(&t, 0x42) != Some(&[HMAC_SHA256]) {
                    return Err(SW_WRONG_DATA);
                }
                let id = id()?;
                // The policy first: an object the session may not use is refused whatever it is.
                self.get(id, sign_rights).map_err(status)?;
                let mac = self.hmac(id, find(&t, 0x43).unwrap_or(&[])).map_err(|e| match e {
                    ChipError::NoSuchKey => SW_WRONG_DATA,
                    e => status(e),
                })?;
                Ok(tlv(0x41, &mac))
            }
            _ => Err(SW_UNKNOWN_INS),
        }
    }
}
