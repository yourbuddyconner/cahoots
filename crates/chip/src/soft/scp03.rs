//! SCP03, the card's side, for the software chip's APDU front end: GlobalPlatform Card
//! Specification v2.3, Amendment D, as the SE05x applet runs it for an AESKey session, with
//! the object's one AES key as the static ENC, MAC and DEK keys.
//!
//! Written apart from the phone's host side (`cahoots_phone::scp03`) so the phone's client is
//! checked against a second reading of the specification, not against itself: the KDF in
//! counter mode with the constants `04` (S-ENC), `06` (S-MAC), `07` (S-RMAC), `00` (card
//! cryptogram) and `01` (host cryptogram); the C-MAC over the chaining value and the command
//! with the secure messaging class; C-ENC with the ICV `AES(S-ENC, counter)`; R-MAC over the
//! chaining value, the data and the status word; R-ENC with the counter's first byte `80`.

use aes::cipher::block_padding::NoPadding;
use aes::cipher::{BlockDecryptMut, BlockEncrypt, BlockEncryptMut, KeyInit, KeyIvInit};
use aes::Aes128;
use alloc::vec::Vec;
use cmac::{Cmac, Mac};

pub(super) const C_MAC: u8 = 0x01;
pub(super) const C_DECRYPTION: u8 = 0x02;
pub(super) const R_MAC: u8 = 0x10;
pub(super) const R_ENCRYPTION: u8 = 0x20;

fn cmac(key: &[u8; 16], parts: &[&[u8]]) -> [u8; 16] {
    let mut m = <Cmac<Aes128> as Mac>::new_from_slice(key).expect("a 16-byte key");
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

/// One block of the counter-mode KDF: label (eleven zeros and the constant), separator,
/// length in bits, counter 1, context.
fn derive(key: &[u8; 16], constant: u8, context: &[u8; 16], bits: u16) -> [u8; 16] {
    let mut input = [0u8; 32];
    input[11] = constant;
    input[13..15].copy_from_slice(&bits.to_be_bytes());
    input[15] = 1;
    input[16..].copy_from_slice(context);
    cmac(key, &[&input])
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// One session's secure channel, from the card's side.
pub(super) struct Card {
    s_enc: [u8; 16],
    s_mac: [u8; 16],
    s_rmac: [u8; 16],
    context: [u8; 16],
    chaining: [u8; 16],
    counter: u128,
    pub(super) level: u8,
}

impl Card {
    pub(super) fn new(key: &[u8; 16], host_challenge: &[u8], card_challenge: &[u8; 8]) -> Card {
        let mut context = [0u8; 16];
        context[..8].copy_from_slice(host_challenge);
        context[8..].copy_from_slice(card_challenge);
        Card {
            s_enc: derive(key, 0x04, &context, 128),
            s_mac: derive(key, 0x06, &context, 128),
            s_rmac: derive(key, 0x07, &context, 128),
            context,
            chaining: [0; 16],
            counter: 0,
            level: 0,
        }
    }

    pub(super) fn card_cryptogram(&self) -> [u8; 8] {
        derive(&self.s_mac, 0x00, &self.context, 64)[..8].try_into().expect("8 bytes")
    }

    /// Check `EXTERNAL AUTHENTICATE` (`84 82 level 00 10 || host cryptogram || C-MAC`). On
    /// success the level is set and the chaining value is the command's MAC.
    pub(super) fn external_authenticate(&mut self, apdu: &[u8]) -> bool {
        if apdu.len() != 21 || apdu[..2] != [0x84, 0x82] || apdu[3] != 0 || apdu[4] != 0x10 {
            return false;
        }
        let level = apdu[2];
        if level & C_MAC == 0 {
            return false;
        }
        let host = derive(&self.s_mac, 0x01, &self.context, 64);
        if !same(&host[..8], &apdu[5..13]) {
            return false;
        }
        let full = cmac(&self.s_mac, &[&[0u8; 16], &apdu[..13]]);
        if !same(&full[..8], &apdu[13..21]) {
            return false;
        }
        self.chaining = full;
        self.level = level;
        true
    }

    fn icv(&self, response: bool) -> [u8; 16] {
        let mut b = self.counter.to_be_bytes();
        if response {
            b[0] = 0x80;
        }
        let mut block = b.into();
        Aes128::new(&self.s_enc.into()).encrypt_block(&mut block);
        block.into()
    }

    /// The plain data of a command under secure messaging: `header` is `CLA INS P1 P2 Lc` as
    /// sent, `body` the data with the C-MAC at its end. `None` if the MAC or the padding is
    /// wrong.
    pub(super) fn open_command(&mut self, header: &[u8], body: &[u8]) -> Option<Vec<u8>> {
        self.counter += 1;
        if body.len() < 8 {
            return None;
        }
        let (data, mac) = body.split_at(body.len() - 8);
        let full = cmac(&self.s_mac, &[&self.chaining, header, data]);
        if !same(&full[..8], mac) {
            return None;
        }
        self.chaining = full;
        if self.level & C_DECRYPTION == 0 || data.is_empty() {
            return Some(data.to_vec());
        }
        if data.len() % 16 != 0 {
            return None;
        }
        let mut buf = data.to_vec();
        let icv = self.icv(false);
        cbc::Decryptor::<Aes128>::new(&self.s_enc.into(), &icv.into())
            .decrypt_padded_mut::<NoPadding>(&mut buf)
            .ok()?;
        let end = buf.iter().rposition(|&b| b != 0)?;
        if buf[end] != 0x80 || buf.len() - end > 16 {
            return None;
        }
        buf.truncate(end);
        Some(buf)
    }

    /// The answer to the last command with status `90 00`: the data encrypted when the level
    /// asks for it, then the R-MAC.
    pub(super) fn seal_response(&self, data: &[u8]) -> Vec<u8> {
        let mut body = if self.level & R_ENCRYPTION != 0 && !data.is_empty() {
            let mut buf = data.to_vec();
            buf.push(0x80);
            while buf.len() % 16 != 0 {
                buf.push(0);
            }
            let n = buf.len();
            let icv = self.icv(true);
            cbc::Encryptor::<Aes128>::new(&self.s_enc.into(), &icv.into())
                .encrypt_padded_mut::<NoPadding>(&mut buf, n)
                .expect("whole blocks");
            buf
        } else {
            data.to_vec()
        };
        if self.level & R_MAC != 0 {
            let mac = cmac(&self.s_rmac, &[&self.chaining, &body, &[0x90, 0x00]]);
            body.extend_from_slice(&mac[..8]);
        }
        body
    }
}
