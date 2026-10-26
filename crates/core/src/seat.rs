//! Seat keys: one P-256 ECDSA key pair per seat per circle (spec §2).
//!
//! The 32-byte seed is the secret scalar. Signatures are deterministic (RFC 6979) over SHA-256
//! of the message, as raw `r || s`. Public keys travel as 33-byte SEC1 compressed points.

use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeatError {
    /// The seed is zero or not below the curve order; draw another.
    BadSeed,
    /// The bytes are not a valid SEC1 compressed P-256 point.
    BadPublic,
}

/// A seat's secret signing key. The scalar is zeroized on drop.
pub struct SeatKey(SigningKey);

/// A seat's public key, SEC1 compressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SeatPublic(pub [u8; 33]);

impl SeatKey {
    /// Treats `seed` as the secret scalar. Fails on zero or a value not below the curve order.
    pub fn from_seed(seed: &[u8; 32]) -> Result<SeatKey, SeatError> {
        SigningKey::from_slice(seed)
            .map(SeatKey)
            .map_err(|_| SeatError::BadSeed)
    }

    /// The scalar, for storage; `from_seed` reads it back.
    pub fn to_bytes(&self) -> Zeroizing<[u8; 32]> {
        let mut field = self.0.to_bytes();
        let mut out = Zeroizing::new([0u8; 32]);
        out.copy_from_slice(&field);
        field.zeroize();
        out
    }

    pub fn public(&self) -> SeatPublic {
        let point = self.0.verifying_key().to_encoded_point(true);
        let mut out = [0u8; 33];
        out.copy_from_slice(point.as_bytes());
        SeatPublic(out)
    }

    /// Deterministic ECDSA (RFC 6979) over SHA-256 of `msg`, raw `r || s`.
    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        let sig: Signature = self.0.sign(msg);
        sig.to_bytes().into()
    }
}

impl SeatPublic {
    /// Accepts only a 33-byte SEC1 compressed point on the curve.
    pub fn parse(bytes: &[u8]) -> Result<SeatPublic, SeatError> {
        let arr: [u8; 33] = bytes.try_into().map_err(|_| SeatError::BadPublic)?;
        if arr[0] != 0x02 && arr[0] != 0x03 {
            return Err(SeatError::BadPublic);
        }
        VerifyingKey::from_sec1_bytes(&arr).map_err(|_| SeatError::BadPublic)?;
        Ok(SeatPublic(arr))
    }

    /// True if `sig` is a valid signature by this key over `msg`. False for a key that does not
    /// parse.
    pub fn verify(&self, msg: &[u8], sig: &[u8; 64]) -> bool {
        let Ok(key) = VerifyingKey::from_sec1_bytes(&self.0) else {
            return false;
        };
        let Ok(sig) = Signature::from_slice(sig) else {
            return false;
        };
        key.verify(msg, &sig).is_ok()
    }
}
