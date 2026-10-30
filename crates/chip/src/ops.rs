//! The chip's operations and the values they return.

use crate::ids::KeyId;
use alloc::vec::Vec;

/// A signature as 64 bytes: P-256 `r || s`, Ed25519 `R || S`.
pub type Signature64 = [u8; 64];

/// The curve of a key made in the chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    P256,
    Ed25519,
}

/// Why a chip call failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipError {
    /// No chip answered.
    NoChip,
    /// The bus or framing failed.
    Transport,
    /// The chip returned this status word.
    Status(u16),
    /// No object of the right kind has this id.
    NoSuchKey,
    /// The chip is busy; try again.
    Busy,
}

/// The chip's applet version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
pub struct Version {
    #[n(0)]
    pub major: u8,
    #[n(1)]
    pub minor: u8,
    #[n(2)]
    pub patch: u8,
}

/// A key's attestation: the chip vouches that it made the key and holds it.
#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
pub struct Attested {
    /// The attested public key.
    #[n(0)]
    #[cbor(with = "minicbor::bytes")]
    pub public: Vec<u8>,
    /// The chip's unique id.
    #[n(1)]
    #[cbor(with = "minicbor::bytes")]
    pub uid: Vec<u8>,
    /// The caller's freshness value, echoed.
    #[n(2)]
    #[cbor(with = "minicbor::bytes")]
    pub freshness: [u8; 16],
    /// The chip's monotonic timestamp.
    #[n(3)]
    #[cbor(with = "minicbor::bytes")]
    pub timestamp: Vec<u8>,
    /// The attestation signature over `public || uid || freshness || timestamp`.
    #[n(4)]
    #[cbor(with = "minicbor::bytes")]
    pub signature: Vec<u8>,
    /// The attestation key's public key (P-256, SEC1 uncompressed, as in its certificate).
    #[n(5)]
    #[cbor(with = "minicbor::bytes")]
    pub attest_public: Vec<u8>,
    /// The attestation key's certificate, DER.
    #[n(6)]
    #[cbor(with = "minicbor::bytes")]
    pub attest_cert_der: Vec<u8>,
}

/// What the rest of the system asks of a chip.
pub trait ChipOps {
    fn version(&mut self) -> Result<Version, ChipError>;
    /// `n` random bytes from the chip.
    fn random(&mut self, n: usize) -> Result<Vec<u8>, ChipError>;
    /// Makes a key and returns its public key (SEC1 compressed for P-256).
    fn generate(&mut self, id: KeyId, curve: Curve) -> Result<Vec<u8>, ChipError>;
    /// The public key of a signing key. HMAC keys and counters have none: `NoSuchKey`.
    fn public(&mut self, id: KeyId) -> Result<Vec<u8>, ChipError>;
    fn delete(&mut self, id: KeyId) -> Result<(), ChipError>;
    fn exists(&mut self, id: KeyId) -> Result<bool, ChipError>;
    /// Every object in the Cahoots id range, in id order.
    fn ids(&mut self) -> Result<Vec<KeyId>, ChipError>;
    fn sign_p256(&mut self, id: KeyId, digest: &[u8; 32]) -> Result<Signature64, ChipError>;
    fn sign_ed25519(&mut self, id: KeyId, message: &[u8]) -> Result<Signature64, ChipError>;
    fn attest(&mut self, id: KeyId, freshness: &[u8; 16]) -> Result<Attested, ChipError>;
    /// Stores an HMAC key that can be used but never read back.
    fn put_hmac_key(&mut self, id: KeyId, key: &[u8; 32]) -> Result<(), ChipError>;
    fn hmac(&mut self, id: KeyId, data: &[u8]) -> Result<[u8; 32], ChipError>;
    fn put_counter(&mut self, id: KeyId, value: u64) -> Result<(), ChipError>;
    /// Adds one and returns the new value.
    fn counter_next(&mut self, id: KeyId) -> Result<u64, ChipError>;
    /// Reads a counter without moving it. `NoSuchKey` if there is none.
    fn counter_get(&mut self, id: KeyId) -> Result<u64, ChipError>;
}
