//! The SE05x applet's commands, encoded by hand: what the phone sends the badge's chip over NFC.
//!
//! From NXP AN12413 (the SE05x APDU specification), with the TLV tags and the INS, P1 and P2
//! values as Nitrokey's `se05x` 0.4.2 encodes them (not a dependency here):
//!
//! | command | CLA INS P1 P2 | data | answer |
//! |---|---|---|---|
//! | SELECT | `00 A4 04 00` | the applet id ([`APPLET_ID`]) | the applet's version (7 bytes) |
//! | CreateSession | `80 04 00 1B` | `41` the authentication object's id (4) | `41` session id (8) |
//! | ProcessSessionCmd | `80 05 00 00` | `10` session id (8), `41` the command in the session | the command's answer |
//! | INITIALIZE UPDATE | `80 50 00 00` | host challenge (8) | diversification data (10), key information (3), card challenge (8), card cryptogram (8) |
//! | EXTERNAL AUTHENTICATE | `84 82 lvl 00` | host cryptogram (8), C-MAC (8): built by [`crate::scp03::Scp03::external_authenticate`] | none |
//! | CloseSession | `80 04 00 1C` | none | none |
//! | GetVersion | `80 04 00 20` | none | `41` the applet's version (7 bytes) |
//! | ReadObject | `80 02 00 00` | `41` id (4), `42` offset (2), `43` length (2) | `41` the bytes |
//! | ReadSize | `80 02 00 07` | `41` id (4) | `41` the size, big-endian |
//! | WriteBinary | `80 01 06 00` | `41` id (4), `42` offset (2), `44` data | none |
//! | MACOneShot | `80 03 0D 45` | `41` key id (4), `42` algorithm (`19` HMAC-SHA256), `43` data | `41` the MAC |
//!
//! The phone never creates an object, so WriteBinary carries no policy (`11`) and no file
//! length (`43`): it writes into an object the badge made. A session's commands go inside
//! ProcessSessionCmd; once its SCP03 handshake is done each is wrapped under secure messaging
//! first ([`crate::scp03::Scp03::wrap`]).
//!
//! Commands here are short APDUs: the phone reads and writes in pieces ([`READ_CHUNK`],
//! [`WRITE_CHUNK`]) small enough that a piece, encrypted and MACed and wrapped in a session,
//! still fits one short command and one short answer.

/// The SE05x IoT applet's id, the one the badge selects over I2C.
pub const APPLET_ID: [u8; 16] = [
    0xA0, 0x00, 0x00, 0x03, 0x96, 0x54, 0x53, 0x00, 0x00, 0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00,
];

/// The most bytes one ReadObject asks for: the answer's TLV (227 bytes) padded and MACed is 248,
/// inside a short answer's 256.
pub const READ_CHUNK: u16 = 224;
/// The most bytes one WriteBinary carries.
pub const WRITE_CHUNK: usize = 192;
/// The most plain data a command in a session may carry: encrypted (224) and MACed (232),
/// with its header and Le, inside ProcessSessionCmd's TLVs, it fits a short Lc.
pub const MAX_SESSION_DATA: usize = 223;

const NO_SM_CLA: u8 = 0x80;
const TAG_SESSION_ID: u8 = 0x10;
const TAG_1: u8 = 0x41;
const TAG_2: u8 = 0x42;
const TAG_3: u8 = 0x43;
const TAG_4: u8 = 0x44;
const HMAC_SHA256: u8 = 0x19;

/// One command APDU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Apdu {
    pub cla: u8,
    pub ins: u8,
    pub p1: u8,
    pub p2: u8,
    pub data: Vec<u8>,
    /// The answer's expected length, 1 to 65536; `None` when no data comes back.
    pub le: Option<usize>,
}

impl Apdu {
    fn new(ins: u8, p1: u8, p2: u8, data: Vec<u8>, le: Option<usize>) -> Apdu {
        Apdu { cla: NO_SM_CLA, ins, p1, p2, data, le }
    }

    /// The bytes: short when the data fits 255 bytes and the answer 256, extended otherwise.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![self.cla, self.ins, self.p1, self.p2];
        let short = self.data.len() <= 255 && self.le.is_none_or(|n| n <= 256);
        if !self.data.is_empty() {
            if short {
                out.push(self.data.len() as u8);
            } else {
                out.push(0);
                out.extend_from_slice(&(self.data.len() as u16).to_be_bytes());
            }
            out.extend_from_slice(&self.data);
        }
        if let Some(n) = self.le {
            if short {
                out.push(if n == 256 { 0 } else { n as u8 });
            } else {
                if self.data.is_empty() {
                    out.push(0);
                }
                out.extend_from_slice(&(if n == 65536 { 0 } else { n as u16 }).to_be_bytes());
            }
        }
        out
    }
}

/// A BER-TLV with a one-byte tag.
pub fn tlv(tag: u8, value: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match value.len() {
        n @ 0..=0x7F => out.push(n as u8),
        n @ 0x80..=0xFF => out.extend_from_slice(&[0x81, n as u8]),
        n => {
            out.push(0x82);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
    }
    out.extend_from_slice(value);
    out
}

/// The value of the first TLV tagged `tag` in an answer.
pub fn find_tlv(mut d: &[u8], tag: u8) -> Option<&[u8]> {
    while d.len() >= 2 {
        let (n, skip) = match d[1] {
            n @ 0..=0x7F => (n as usize, 2),
            0x81 if d.len() >= 3 => (d[2] as usize, 3),
            0x82 if d.len() >= 4 => (u16::from_be_bytes([d[2], d[3]]) as usize, 4),
            _ => return None,
        };
        let v = d.get(skip..skip + n)?;
        if d[0] == tag {
            return Some(v);
        }
        d = &d[skip + n..];
    }
    None
}

/// An answer split into its data and status word; `None` if it has no status word.
pub fn split(answer: &[u8]) -> Option<(&[u8], u16)> {
    let n = answer.len().checked_sub(2)?;
    Some((&answer[..n], u16::from_be_bytes([answer[n], answer[n + 1]])))
}

pub fn select() -> Apdu {
    Apdu { cla: 0x00, ins: 0xA4, p1: 0x04, p2: 0x00, data: APPLET_ID.to_vec(), le: Some(256) }
}

pub fn create_session(object_id: u32) -> Apdu {
    Apdu::new(0x04, 0x00, 0x1B, tlv(TAG_1, &object_id.to_be_bytes()), Some(256))
}

/// `command`, as encoded bytes, run in the session `session_id`.
pub fn process_session(session_id: &[u8; 8], command: &[u8]) -> Apdu {
    let mut data = tlv(TAG_SESSION_ID, session_id);
    data.extend_from_slice(&tlv(TAG_1, command));
    Apdu::new(0x05, 0x00, 0x00, data, Some(256))
}

pub fn initialize_update(host_challenge: &[u8; 8]) -> Apdu {
    Apdu::new(0x50, 0x00, 0x00, host_challenge.to_vec(), Some(256))
}

pub fn close_session() -> Apdu {
    Apdu::new(0x04, 0x00, 0x1C, Vec::new(), None)
}

pub fn get_version() -> Apdu {
    Apdu::new(0x04, 0x00, 0x20, Vec::new(), Some(256))
}

pub fn read_object(object_id: u32, offset: u16, length: u16) -> Apdu {
    let mut data = tlv(TAG_1, &object_id.to_be_bytes());
    data.extend_from_slice(&tlv(TAG_2, &offset.to_be_bytes()));
    data.extend_from_slice(&tlv(TAG_3, &length.to_be_bytes()));
    Apdu::new(0x02, 0x00, 0x00, data, Some(256))
}

pub fn read_size(object_id: u32) -> Apdu {
    Apdu::new(0x02, 0x00, 0x07, tlv(TAG_1, &object_id.to_be_bytes()), Some(256))
}

pub fn write_binary(object_id: u32, offset: u16, bytes: &[u8]) -> Apdu {
    let mut data = tlv(TAG_1, &object_id.to_be_bytes());
    data.extend_from_slice(&tlv(TAG_2, &offset.to_be_bytes()));
    data.extend_from_slice(&tlv(TAG_4, bytes));
    Apdu::new(0x01, 0x06, 0x00, data, None)
}

pub fn mac_one_shot(key_id: u32, bytes: &[u8]) -> Apdu {
    let mut data = tlv(TAG_1, &key_id.to_be_bytes());
    data.extend_from_slice(&tlv(TAG_2, &[HMAC_SHA256]));
    data.extend_from_slice(&tlv(TAG_3, bytes));
    Apdu::new(0x03, 0x0D, 0x45, data, Some(256))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_is_the_diagnostics_screens_select() {
        // ios/Cahoots/NfcDiagnosticsView.swift: 00 A4 04 00 10 <aid> 00.
        let mut want = vec![0x00, 0xA4, 0x04, 0x00, 0x10];
        want.extend_from_slice(&APPLET_ID);
        want.push(0x00);
        assert_eq!(select().encode(), want);
    }

    #[test]
    fn commands_encode_as_the_applet_reads_them() {
        assert_eq!(
            create_session(0x4C41_0500).encode(),
            hex::decode("800400 1B06 41044c410500 00".replace(' ', "")).unwrap()
        );
        assert_eq!(close_session().encode(), hex::decode("8004001c").unwrap());
        assert_eq!(
            mac_one_shot(0x4C41_0203, b"ab").encode(),
            hex::decode("8003 0d45 0d 41044c410203 420119 43026162 00".replace(' ', "")).unwrap()
        );
        assert_eq!(
            read_object(0x4C41_07FF, 224, 224).encode(),
            hex::decode("80020000 0e 41044c4107ff 420200e0 430200e0 00".replace(' ', "")).unwrap()
        );
    }

    #[test]
    fn long_commands_are_extended() {
        let a = Apdu::new(0x01, 0x06, 0x00, vec![7; 300], None).encode();
        assert_eq!(&a[4..7], &[0x00, 0x01, 0x2C]);
        assert_eq!(a.len(), 7 + 300);
    }

    #[test]
    fn tlvs_read_back() {
        let mut d = tlv(0x41, &[1; 200]);
        d.extend_from_slice(&tlv(0x42, &[2; 3]));
        assert_eq!(find_tlv(&d, 0x41), Some(&[1u8; 200][..]));
        assert_eq!(find_tlv(&d, 0x42), Some(&[2u8; 3][..]));
        assert_eq!(find_tlv(&d, 0x43), None);
    }
}
