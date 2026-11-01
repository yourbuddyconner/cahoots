//! SCP03, the host's side: GlobalPlatform Card Specification v2.3, Amendment D (Secure Channel
//! Protocol 03), as the SE05x applet runs it for an AESKey session.
//!
//! The applet's AESKey object holds one AES-128 key, used as the static ENC, MAC and DEK keys
//! ([`StaticKeys::single`]). The handshake:
//!
//! 1. `INITIALIZE UPDATE` carries an 8-byte host challenge. The card answers with key
//!    diversification data (10), key information (3), its card challenge (8) and its card
//!    cryptogram (8).
//! 2. Both sides derive the session keys with the KDF in counter mode (NIST SP 800-108 with
//!    AES-CMAC as the PRF, Amendment D section 6.2.1): the input is eleven zero bytes, the
//!    derivation constant, a zero separator, the output length in bits (two bytes), the counter
//!    `01` and the context `host challenge || card challenge`. The constants are `04` for
//!    S-ENC (from the static ENC key), `06` for S-MAC and `07` for S-RMAC (from the static MAC
//!    key), `00` for the card cryptogram and `01` for the host cryptogram (from S-MAC, 64 bits).
//! 3. The host checks the card cryptogram, which proves the card holds the key, and sends
//!    `EXTERNAL AUTHENTICATE` with the security level, its host cryptogram and a C-MAC.
//!
//! Then every command is wrapped ([`Scp03::wrap`]) and every answer unwrapped
//! ([`Scp03::unwrap`]):
//!
//! - C-ENC: the encryption counter, 16 bytes big-endian, goes up by one for every command
//!   after `EXTERNAL AUTHENTICATE`, the first command using 1. A command with data has it
//!   padded (`80` then zeros to a multiple of 16) and encrypted with AES-CBC under S-ENC, with
//!   the ICV `AES(S-ENC, counter)`. A command with no data is not padded.
//! - C-MAC: `AES-CMAC(S-MAC, chaining value || CLA INS P1 P2 Lc || data)`, where the CLA has
//!   the secure messaging bit (`84`) and Lc counts the 8-byte MAC. Its first 8 bytes follow the
//!   data and all 16 become the next chaining value. The first chaining value, for `EXTERNAL
//!   AUTHENTICATE`, is 16 zero bytes. Le, when present, is not MACed.
//! - R-MAC: `AES-CMAC(S-RMAC, chaining value || response data || SW1 SW2)`, its first 8 bytes
//!   after the data, the chaining value the one of the command it answers.
//! - R-ENC: the response data decrypted with AES-CBC under S-ENC, the ICV `AES(S-ENC, counter)`
//!   with the counter's first byte set to `80`, then the padding removed.
//!
//! A status word other than `90 00` comes back bare, with no R-MAC and no data: the applet
//! answers an error that way (`nxScp03` in NXP's Plug & Trust middleware does not check one).

use aes::cipher::block_padding::NoPadding;
use aes::cipher::{BlockDecryptMut, BlockEncrypt, BlockEncryptMut, KeyInit, KeyIvInit};
use aes::Aes128;
use cmac::{Cmac, Mac};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// The security level the phone asks for: C-MAC, C-DECRYPTION, R-MAC and R-ENCRYPTION.
pub const LEVEL_FULL: u8 = 0x33;
const C_DECRYPTION: u8 = 0x02;
const R_MAC: u8 = 0x10;
const R_ENCRYPTION: u8 = 0x20;

/// The secure messaging class byte.
pub const SM_CLA: u8 = 0x84;

/// The derivation constants of Amendment D, table 6-1.
const CARD_CRYPTOGRAM: u8 = 0x00;
const HOST_CRYPTOGRAM: u8 = 0x01;
const DERIVE_S_ENC: u8 = 0x04;
const DERIVE_S_MAC: u8 = 0x06;
const DERIVE_S_RMAC: u8 = 0x07;

/// The static keys of a key set. The SE05x's AESKey object holds one key for all three.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct StaticKeys {
    pub enc: [u8; 16],
    pub mac: [u8; 16],
    pub dek: [u8; 16],
}

impl StaticKeys {
    /// One key as ENC, MAC and DEK, as an AESKey session uses it.
    pub fn single(key: &[u8; 16]) -> StaticKeys {
        StaticKeys { enc: *key, mac: *key, dek: *key }
    }
}

/// Why an answer did not unwrap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScpError {
    /// Too short for an R-MAC, or the R-MAC is not the card's.
    BadMac,
    /// Encrypted data that is not whole blocks, or padding that is not `80 00..`.
    BadPadding,
}

fn cmac(key: &[u8; 16], parts: &[&[u8]]) -> [u8; 16] {
    let mut m = <Cmac<Aes128> as Mac>::new_from_slice(key).expect("a 16-byte key");
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

/// The KDF in counter mode of Amendment D section 6.2.1, one block: the first `bits / 8` bytes
/// (at most 16) of `AES-CMAC(key, 00 * 11 || constant || 00 || bits || 01 || context)`.
pub fn kdf(key: &[u8; 16], constant: u8, context: &[u8], bits: u16) -> [u8; 16] {
    let mut label = [0u8; 16];
    label[11] = constant;
    label[13..15].copy_from_slice(&bits.to_be_bytes());
    label[15] = 0x01;
    cmac(key, &[&label, context])
}

/// ISO/IEC 9797-1 method 2: `80` then zeros to a multiple of 16.
fn pad(data: &[u8]) -> Vec<u8> {
    let mut v = data.to_vec();
    v.push(0x80);
    while v.len() % 16 != 0 {
        v.push(0);
    }
    v
}

fn unpad(mut v: Vec<u8>) -> Result<Vec<u8>, ScpError> {
    let end = v.iter().rposition(|&b| b != 0).ok_or(ScpError::BadPadding)?;
    if v[end] != 0x80 || v.len() - end > 16 {
        return Err(ScpError::BadPadding);
    }
    v.truncate(end);
    Ok(v)
}

/// The secure channel of one session, from the host's side.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Scp03 {
    s_enc: [u8; 16],
    s_mac: [u8; 16],
    s_rmac: [u8; 16],
    context: [u8; 16],
    chaining: [u8; 16],
    counter: u128,
    level: u8,
}

impl Scp03 {
    /// The session for this key set and the two challenges of `INITIALIZE UPDATE`.
    pub fn new(keys: &StaticKeys, host_challenge: &[u8; 8], card_challenge: &[u8; 8]) -> Scp03 {
        let mut context = [0u8; 16];
        context[..8].copy_from_slice(host_challenge);
        context[8..].copy_from_slice(card_challenge);
        Scp03 {
            s_enc: kdf(&keys.enc, DERIVE_S_ENC, &context, 128),
            s_mac: kdf(&keys.mac, DERIVE_S_MAC, &context, 128),
            s_rmac: kdf(&keys.mac, DERIVE_S_RMAC, &context, 128),
            context,
            chaining: [0; 16],
            counter: 0,
            level: 0,
        }
    }

    /// The session keys S-ENC, S-MAC and S-RMAC, for the tests.
    pub fn session_keys(&self) -> ([u8; 16], [u8; 16], [u8; 16]) {
        (self.s_enc, self.s_mac, self.s_rmac)
    }

    /// The cryptogram a card holding the same keys sends.
    pub fn card_cryptogram(&self) -> [u8; 8] {
        let c = kdf(&self.s_mac, CARD_CRYPTOGRAM, &self.context, 64);
        c[..8].try_into().expect("8 bytes")
    }

    /// The host's cryptogram.
    pub fn host_cryptogram(&self) -> [u8; 8] {
        let c = kdf(&self.s_mac, HOST_CRYPTOGRAM, &self.context, 64);
        c[..8].try_into().expect("8 bytes")
    }

    /// True when `card` is the cryptogram of a card holding these keys. Constant time.
    pub fn check_card_cryptogram(&self, card: &[u8; 8]) -> bool {
        let want = self.card_cryptogram();
        want.iter().zip(card).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    }

    /// The `EXTERNAL AUTHENTICATE` command for `level`, MACed. From here on commands are
    /// wrapped at that level.
    pub fn external_authenticate(&mut self, level: u8) -> Vec<u8> {
        self.level = level;
        let header = [SM_CLA, 0x82, level, 0x00, 0x10];
        let host = self.host_cryptogram();
        let full = cmac(&self.s_mac, &[&self.chaining, &header, &host]);
        self.chaining = full;
        let mut apdu = header.to_vec();
        apdu.extend_from_slice(&host);
        apdu.extend_from_slice(&full[..8]);
        apdu
    }

    fn counter_block(&self, response: bool) -> [u8; 16] {
        let mut b = self.counter.to_be_bytes();
        if response {
            b[0] = 0x80;
        }
        let cipher = Aes128::new(&self.s_enc.into());
        let mut block = b.into();
        cipher.encrypt_block(&mut block);
        block.into()
    }

    /// A command under secure messaging: `cla` gets the secure messaging bit, `data` is
    /// encrypted when the level asks for it and is not empty, and the C-MAC follows. `le`
    /// adds a short Le byte. The data, encrypted and MACed, must fit a short Lc: at most 239
    /// bytes encrypted, 247 not.
    pub fn wrap(&mut self, cla: u8, ins: u8, p1: u8, p2: u8, data: &[u8], le: Option<u8>) -> Vec<u8> {
        self.counter += 1;
        let body = if self.level & C_DECRYPTION != 0 && !data.is_empty() {
            let icv = self.counter_block(false);
            let mut buf = pad(data);
            let n = buf.len();
            cbc::Encryptor::<Aes128>::new(&self.s_enc.into(), &icv.into())
                .encrypt_padded_mut::<NoPadding>(&mut buf, n)
                .expect("whole blocks");
            buf
        } else {
            data.to_vec()
        };
        let lc = u8::try_from(body.len() + 8).expect("the data fits a short Lc");
        let header = [cla | 0x04, ins, p1, p2, lc];
        let full = cmac(&self.s_mac, &[&self.chaining, &header, &body]);
        self.chaining = full;
        let mut apdu = header.to_vec();
        apdu.extend_from_slice(&body);
        apdu.extend_from_slice(&full[..8]);
        if let Some(le) = le {
            apdu.push(le);
        }
        apdu
    }

    /// The data of an answer to the last wrapped command with status `90 00`: the R-MAC checked
    /// and the data decrypted.
    pub fn unwrap(&mut self, response: &[u8], sw: u16) -> Result<Vec<u8>, ScpError> {
        let body = if self.level & R_MAC != 0 {
            if response.len() < 8 {
                return Err(ScpError::BadMac);
            }
            let (body, mac) = response.split_at(response.len() - 8);
            let want = cmac(&self.s_rmac, &[&self.chaining, body, &sw.to_be_bytes()]);
            if want[..8].iter().zip(mac).fold(0u8, |acc, (a, b)| acc | (a ^ b)) != 0 {
                return Err(ScpError::BadMac);
            }
            body
        } else {
            response
        };
        if self.level & R_ENCRYPTION == 0 || body.is_empty() {
            return Ok(body.to_vec());
        }
        if body.len() % 16 != 0 {
            return Err(ScpError::BadPadding);
        }
        let icv = self.counter_block(true);
        let mut buf = body.to_vec();
        cbc::Decryptor::<Aes128>::new(&self.s_enc.into(), &icv.into())
            .decrypt_padded_mut::<NoPadding>(&mut buf)
            .map_err(|_| ScpError::BadPadding)?;
        unpad(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> Vec<u8> {
        hex::decode(s).unwrap()
    }

    fn a8(s: &str) -> [u8; 8] {
        h(s).try_into().unwrap()
    }

    /// RFC 4493 section 4, examples 1 and 2: the PRF the KDF and the MACs are built on.
    #[test]
    fn aes_cmac_matches_rfc_4493() {
        let k: [u8; 16] = h("2b7e151628aed2a6abf7158809cf4f3c").try_into().unwrap();
        assert_eq!(cmac(&k, &[]).to_vec(), h("bb1d6929e95937287fa37d129b756746"));
        assert_eq!(
            cmac(&k, &[&h("6bc1bee22e409f96e93d7e117393172a")]).to_vec(),
            h("070a16b46b4d4144f79bdd9dd04a287c")
        );
    }

    /// The worked example in Osmocom pySim, `tests/unittests/test_globalplatform.py`, class
    /// `SCP03_Test_AES128_33` (GPL-2.0-or-later, Harald Welte): the AES-128 key set ENC
    /// `00..0f`, MAC `10..1f`, DEK `20..2f` at security level `33`. GlobalPlatform publishes no
    /// vectors of its own for Amendment D.
    fn pysim_aes128_33() -> (Scp03, [u8; 8]) {
        let keys = StaticKeys {
            enc: h("000102030405060708090a0b0c0d0e0f").try_into().unwrap(),
            mac: h("101112131415161718191a1b1c1d1e1f").try_into().unwrap(),
            dek: h("202122232425262728292a2b2c2d2e2f").try_into().unwrap(),
        };
        // INITIALIZE UPDATE `8050300008fdf38259a1e0de4400`, answered with
        // `00000000000000000000 300370 3b1aca81e821f219 081cdc01c26b372d 000003`.
        let host = a8("fdf38259a1e0de44");
        let card = a8("3b1aca81e821f219");
        (Scp03::new(&keys, &host, &card), a8("081cdc01c26b372d"))
    }

    #[test]
    fn the_card_cryptogram_matches_the_worked_example() {
        let (s, card_cryptogram) = pysim_aes128_33();
        assert_eq!(s.card_cryptogram(), card_cryptogram);
        assert!(s.check_card_cryptogram(&card_cryptogram));
        assert!(!s.check_card_cryptogram(&[0; 8]));
    }

    #[test]
    fn external_authenticate_matches_the_worked_example() {
        let (mut s, _) = pysim_aes128_33();
        assert_eq!(
            s.external_authenticate(LEVEL_FULL),
            h("84823300108c36f96bcc00724a4e13ad591d7da3f0")
        );
    }

    /// The example's first command, GET DATA for the EID (`80E2910006BF3E035C015A00`), its
    /// wrapped form and the card's encrypted, MACed answer.
    #[test]
    fn a_wrapped_command_and_its_answer_match_the_worked_example() {
        let (mut s, _) = pysim_aes128_33();
        s.external_authenticate(LEVEL_FULL);
        let cmd = s.wrap(0x80, 0xE2, 0x91, 0x00, &h("bf3e035c015a"), Some(0));
        assert_eq!(cmd, h("84e2910018267a85dfe4a98fca6fb0527e0dfecce4914e40401433c87f00"));
        let rsp = h("f3ba2b1013aa6224f5e1c138d71805c569e5439b47576260b75fc021b25097cb2e68f8a0144975b9");
        assert_eq!(
            s.unwrap(&rsp, 0x9000).unwrap(),
            h("bf3e125a1089882119900000000000000000000005")
        );
    }

    #[test]
    fn a_changed_answer_does_not_unwrap() {
        let (mut s, _) = pysim_aes128_33();
        s.external_authenticate(LEVEL_FULL);
        s.wrap(0x80, 0xE2, 0x91, 0x00, &h("bf3e035c015a"), Some(0));
        let mut rsp = h("f3ba2b1013aa6224f5e1c138d71805c569e5439b47576260b75fc021b25097cb2e68f8a0144975b9");
        rsp[3] ^= 1;
        assert_eq!(s.unwrap(&rsp, 0x9000), Err(ScpError::BadMac));
    }

    #[test]
    fn padding_round_trips() {
        for n in [0, 1, 15, 16, 17, 31, 32] {
            let d: Vec<u8> = (0..n as u8).collect();
            let p = pad(&d);
            assert_eq!(p.len() % 16, 0);
            assert_eq!(unpad(p).unwrap(), d);
        }
        assert_eq!(unpad(vec![0; 16]), Err(ScpError::BadPadding));
    }
}
