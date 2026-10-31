//! Deposits (channels spec §4): an epoch key sealed for one reader, stored on the relay and
//! opaque to it.
//!
//! | key | value |
//! |---|---|
//! | 0 | version 1 |
//! | 1 | channel_id bstr 16 |
//! | 2 | epoch_id bstr 20 |
//! | 3 | to bstr 32: the link id for a link deposit, the signing key's key id for a key deposit |
//! | 4 | kind uint: 0 link, 1 key |
//! | 5 | sealed bstr |
//!
//! A link deposit is ChaCha20-Poly1305 under the link's seal key. A key deposit is HPKE base mode
//! to the member's X-Wing key, `sealed = enc || ct` (needs the `hpke` feature, which `std`
//! turns on). Both use the same AAD, [`aad`].
//!
//! Every deposit is signed ([`Deposit::sign`]): a COSE_Sign1 with the canonical map as the
//! embedded payload, kid the key id of the signing member key, external AAD [`DOMAIN`]. The
//! signer is the member key of the seat that issued the epoch, whose grant carries `govern`; a
//! reader installs a deposit only once that grant chain verifies ([`verify_signed`] checks the
//! signature, the member core the chain). The deposit's id stays the SHA-256 of the unsigned
//! map, so a rotation's ids do not depend on the signature.

use alloc::vec::Vec;
use cahoots_core::cose;
use cahoots_core::seat::{SeatPublic, SeatSigner};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::ChaCha20Poly1305;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::cbor::{canonical, encode_with, Enc, EncResult, Reader, VERSION};
use crate::keys::{kid_of, EpochId, Key};
use crate::link::LinkSecret;
use crate::{ChannelError, ChannelId};

/// The deposit domain: the AAD prefix, and the HPKE info of a key deposit.
pub const DOMAIN: &[u8] = b"cahoots/deposit/v1";

pub const KIND_LINK: u8 = 0;
pub const KIND_KEY: u8 = 1;

/// The length of an X-Wing encapsulated key, the first part of a key deposit's `sealed`.
pub const XWING_ENC_LEN: usize = 1120;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deposit {
    pub channel_id: ChannelId,
    pub epoch_id: EpochId,
    pub to: [u8; 32],
    pub kind: u8,
    pub sealed: Vec<u8>,
}

impl Deposit {
    /// The canonical CBOR map.
    pub fn encode(&self) -> Vec<u8> {
        encode_with(|e| self.write(e))
    }

    fn write(&self, e: &mut Enc) -> EncResult {
        e.map(6)?;
        e.u8(0)?.u8(VERSION)?;
        e.u8(1)?.bytes(&self.channel_id)?;
        e.u8(2)?.bytes(&self.epoch_id.encode())?;
        e.u8(3)?.bytes(&self.to)?;
        e.u8(4)?.u8(self.kind)?;
        e.u8(5)?.bytes(&self.sealed)?;
        Ok(())
    }

    /// Parse a canonical deposit. Refuses an unknown kind, and an epoch id that names another
    /// channel than the deposit's ([`ChannelError::WrongChannel`]).
    pub fn decode(bytes: &[u8]) -> Result<Deposit, ChannelError> {
        let mut r = Reader::open(bytes, 6)?;
        let channel_id = r.array()?;
        let epoch_id = EpochId::decode(r.bytes()?).ok_or(ChannelError::Malformed)?;
        let to = r.array()?;
        let kind = u8::try_from(r.u64()?).map_err(|_| ChannelError::Malformed)?;
        let sealed = r.bytes()?.to_vec();
        r.finish()?;
        if kind != KIND_LINK && kind != KIND_KEY {
            return Err(ChannelError::Malformed);
        }
        if epoch_id.channel_id != channel_id {
            return Err(ChannelError::WrongChannel);
        }
        let deposit = Deposit {
            channel_id,
            epoch_id,
            to,
            kind,
            sealed,
        };
        let encoded = deposit.encode();
        canonical(deposit, encoded, bytes)
    }

    /// The deposit id, `SHA-256(encode())`, that a rotation names: the unsigned map's hash.
    pub fn id(&self) -> [u8; 32] {
        Sha256::digest(self.encode()).into()
    }

    /// Sign the deposit with the issuing member key: a COSE_Sign1 over [`Deposit::encode`]
    /// (embedded), kid `kid_of(signer)`, external AAD [`DOMAIN`]. These are the bytes the relay
    /// stores and serves.
    pub fn sign<S: SeatSigner>(&self, mut signer: S) -> Vec<u8> {
        let kid = kid_of(&signer.public());
        cose::sign1(signer, &kid, &self.encode(), DOMAIN, false)
    }

    fn aad(&self) -> Vec<u8> {
        aad(&self.channel_id, &self.epoch_id, &self.to)
    }
}

/// A signed deposit's signer kid and the deposit it carries, read without checking the
/// signature: for finding the signer's grant chain, and for the relay's index. A kid that is not
/// 32 bytes is [`ChannelError::Signature`].
pub fn peek_signed(bytes: &[u8]) -> Result<([u8; 32], Deposit), ChannelError> {
    let kid: [u8; 32] = cose::kid1(bytes)
        .map_err(ChannelError::Cose)?
        .try_into()
        .map_err(|_| ChannelError::Signature)?;
    let payload = cose::payload1(bytes).map_err(ChannelError::Cose)?;
    Ok((kid, Deposit::decode(&payload)?))
}

/// Verify a signed deposit under `signer_key` and return the deposit. The COSE kid must be
/// `kid_of(signer_key)` and the signature must verify with external AAD [`DOMAIN`]
/// ([`ChannelError::Signature`] otherwise). Whether the key may issue deposits (its grant holds
/// `govern`) is the caller's check.
pub fn verify_signed(bytes: &[u8], signer_key: &SeatPublic) -> Result<Deposit, ChannelError> {
    let kid = cose::kid1(bytes).map_err(ChannelError::Cose)?;
    if kid[..] != kid_of(signer_key) {
        return Err(ChannelError::Signature);
    }
    let payload =
        cose::verify1(signer_key, bytes, None, DOMAIN).map_err(|_| ChannelError::Signature)?;
    Deposit::decode(&payload)
}

/// `DOMAIN || channel_id || epoch_id || to`.
pub fn aad(channel_id: &ChannelId, epoch_id: &EpochId, to: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(DOMAIN.len() + 16 + crate::keys::EPOCH_ID_LEN + 32);
    out.extend_from_slice(DOMAIN);
    out.extend_from_slice(channel_id);
    out.extend_from_slice(&epoch_id.encode());
    out.extend_from_slice(to);
    out
}

/// `epoch (u32 LE) || 0x00 * 8`. One link deposit per link per epoch, so one nonce per key.
fn link_nonce(epoch: u32) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..4].copy_from_slice(&epoch.to_le_bytes());
    n
}

/// A 32-byte plaintext as a key; anything else is not a deposit of ours.
fn as_key(plaintext: Vec<u8>) -> Result<Key, ChannelError> {
    let plaintext = Zeroizing::new(plaintext);
    let key: [u8; 32] = plaintext[..]
        .try_into()
        .map_err(|_| ChannelError::Malformed)?;
    Ok(Zeroizing::new(key))
}

/// Seal `epoch_key` for the holders of a link, to the link's id, under the seal key it derives
/// for the epoch's channel.
pub fn seal_for_link(epoch_key: &[u8; 32], secret: &LinkSecret, epoch_id: &EpochId) -> Deposit {
    let channel_id = epoch_id.channel_id;
    let to = secret.id();
    let seal_key = secret.seal_key(&channel_id);
    let sealed = ChaCha20Poly1305::new(seal_key.as_ref().into())
        .encrypt(
            &link_nonce(epoch_id.epoch).into(),
            Payload {
                msg: epoch_key,
                aad: &aad(&channel_id, epoch_id, &to),
            },
        )
        .expect("ChaCha20-Poly1305 encryption cannot fail for in-memory inputs");
    Deposit {
        channel_id,
        epoch_id: *epoch_id,
        to,
        kind: KIND_LINK,
        sealed,
    }
}

/// Open a link deposit with the link's secret. A key deposit is [`ChannelError::Malformed`]; the
/// wrong secret or changed bytes are [`ChannelError::Tag`].
pub fn open_with_link(d: &Deposit, secret: &LinkSecret) -> Result<Key, ChannelError> {
    if d.kind != KIND_LINK {
        return Err(ChannelError::Malformed);
    }
    let seal_key = secret.seal_key(&d.channel_id);
    let plaintext = ChaCha20Poly1305::new(seal_key.as_ref().into())
        .decrypt(
            &link_nonce(d.epoch_id.epoch).into(),
            Payload {
                msg: &d.sealed,
                aad: &d.aad(),
            },
        )
        .map_err(|_| ChannelError::Tag)?;
    as_key(plaintext)
}

/// Seal `epoch_key` to a member's X-Wing public key in HPKE base mode, info [`DOMAIN`].
/// `to_kid` is the key id of the member's signing key. A bad public key is
/// [`ChannelError::Hpke`].
#[cfg(feature = "hpke")]
pub fn seal_for_key(
    epoch_key: &[u8; 32],
    epoch_id: &EpochId,
    to_kid: [u8; 32],
    xwing_pk: &[u8],
    rng: &mut impl rand_core::CryptoRng,
) -> Result<Deposit, ChannelError> {
    let channel_id = epoch_id.channel_id;
    let (enc, ct) = cahoots_hpke::seal_base(
        cahoots_hpke::Suite::XWing,
        xwing_pk,
        DOMAIN,
        &aad(&channel_id, epoch_id, &to_kid),
        epoch_key,
        rng,
    )
    .map_err(|_| ChannelError::Hpke)?;
    let mut sealed = enc;
    sealed.extend_from_slice(&ct);
    Ok(Deposit {
        channel_id,
        epoch_id: *epoch_id,
        to: to_kid,
        kind: KIND_KEY,
        sealed,
    })
}

/// Open a key deposit with the member's X-Wing secret key. A link deposit, or `sealed` no longer
/// than an encapsulated key, is [`ChannelError::Malformed`]; the wrong key or changed bytes are
/// [`ChannelError::Hpke`].
#[cfg(feature = "hpke")]
pub fn open_with_key(d: &Deposit, xwing_sk: &[u8]) -> Result<Key, ChannelError> {
    if d.kind != KIND_KEY || d.sealed.len() <= XWING_ENC_LEN {
        return Err(ChannelError::Malformed);
    }
    let (enc, ct) = d.sealed.split_at(XWING_ENC_LEN);
    let plaintext = cahoots_hpke::open_base(
        cahoots_hpke::Suite::XWing,
        xwing_sk,
        enc,
        DOMAIN,
        &d.aad(),
        ct,
    )
    .map_err(|_| ChannelError::Hpke)?;
    as_key(plaintext)
}
