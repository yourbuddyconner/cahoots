//! A chip in software, for tests and the simulated badge.
//!
//! Keys live in a map. Attestation is signed by the fixed test CA in [`crate::testca`], so the
//! checks a verifier makes run in `cargo test` without hardware. Not secret: never use it to
//! guard a real key.

use crate::ids::KeyId;
use crate::ops::{Attested, ChipError, ChipOps, Curve, Signature64, Version};
use crate::testca;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use hmac::{Hmac, Mac};
use p256::ecdsa::signature::hazmat::PrehashSigner;
use p256::ecdsa::signature::Signer;
use rand_core::{CryptoRng, RngCore};
use sha2::{Digest, Sha256};

/// The chip's unique id: 17 bytes, the same on every soft chip.
pub const SOFT_UID: &[u8; 17] = b"softchip-00000000";

enum Object {
    P256(p256::ecdsa::SigningKey),
    Ed25519(ed25519_dalek::SigningKey),
    Hmac([u8; 32]),
    Counter(u64),
}

/// A counter-mode generator over SHA-256. Enough for test keys; not for real ones.
struct SoftRng {
    seed: [u8; 32],
    counter: u64,
}

impl RngCore for SoftRng {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.fill_bytes(&mut b);
        u32::from_le_bytes(b)
    }
    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.fill_bytes(&mut b);
        u64::from_le_bytes(b)
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for chunk in dest.chunks_mut(32) {
            let mut h = Sha256::new();
            h.update(self.seed);
            h.update(self.counter.to_be_bytes());
            self.counter += 1;
            chunk.copy_from_slice(&h.finalize()[..chunk.len()]);
        }
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}
impl CryptoRng for SoftRng {}

/// A software chip. See the module docs.
pub struct SoftChip {
    objects: BTreeMap<u32, Object>,
    rng: SoftRng,
    clock: u64,
    ca: p256::ecdsa::SigningKey,
}

impl SoftChip {
    /// A chip with an empty store, its key generator seeded from the system. Only with `std`:
    /// without it there is no entropy, so callers must pass a seed to [`SoftChip::with_seed`].
    #[cfg(all(feature = "std", not(target_os = "none")))]
    pub fn new() -> SoftChip {
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed).expect("the system has no randomness");
        SoftChip::with_seed(seed)
    }

    /// A chip whose generated keys and random bytes follow from `seed`, for repeatable tests.
    pub fn with_seed(seed: [u8; 32]) -> SoftChip {
        let mut ca = [0u8; 32];
        for (i, b) in ca.iter_mut().enumerate() {
            *b = hex_byte(testca::KEY_HEX.as_bytes(), i);
        }
        SoftChip {
            objects: BTreeMap::new(),
            rng: SoftRng { seed, counter: 0 },
            clock: 0,
            ca: p256::ecdsa::SigningKey::from_slice(&ca).expect("the test CA key is valid"),
        }
    }
}

/// The first bytes of [`SoftChip::export`]'s output.
const EXPORT_MAGIC: &[u8; 9] = b"SOFTCHIP1";

impl SoftChip {
    /// Everything the chip holds, so a simulated badge can keep its chip across restarts: the
    /// generator's seed and position, the clock, and every object with its secret.
    ///
    /// `SOFTCHIP1 | seed 32 | counter u64 | clock u64 | count u32 | (id u32 | tag u8 | value)*`,
    /// big-endian, objects in id order; the tag is 1 for P-256 (a 32-byte scalar), 2 for
    /// Ed25519 (a 32-byte seed), 3 for an HMAC key (32 bytes), 4 for a counter (u64).
    ///
    /// The output holds secret keys in the clear, as a software chip does in memory.
    pub fn export(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(EXPORT_MAGIC);
        out.extend_from_slice(&self.rng.seed);
        out.extend_from_slice(&self.rng.counter.to_be_bytes());
        out.extend_from_slice(&self.clock.to_be_bytes());
        out.extend_from_slice(&(self.objects.len() as u32).to_be_bytes());
        for (id, o) in &self.objects {
            out.extend_from_slice(&id.to_be_bytes());
            match o {
                Object::P256(k) => {
                    out.push(1);
                    out.extend_from_slice(&k.to_bytes());
                }
                Object::Ed25519(k) => {
                    out.push(2);
                    out.extend_from_slice(&k.to_bytes());
                }
                Object::Hmac(k) => {
                    out.push(3);
                    out.extend_from_slice(k);
                }
                Object::Counter(c) => {
                    out.push(4);
                    out.extend_from_slice(&c.to_be_bytes());
                }
            }
        }
        out
    }

    /// A chip from [`SoftChip::export`]'s output, or `None` if the bytes are not exactly that.
    pub fn import(bytes: &[u8]) -> Option<SoftChip> {
        let mut r = bytes.strip_prefix(EXPORT_MAGIC.as_slice())?;
        let mut take = |n: usize| -> Option<&[u8]> {
            if r.len() < n {
                return None;
            }
            let (head, rest) = r.split_at(n);
            r = rest;
            Some(head)
        };
        let seed: [u8; 32] = take(32)?.try_into().ok()?;
        let counter = u64::from_be_bytes(take(8)?.try_into().ok()?);
        let clock = u64::from_be_bytes(take(8)?.try_into().ok()?);
        let count = u32::from_be_bytes(take(4)?.try_into().ok()?);
        let mut chip = SoftChip::with_seed(seed);
        chip.rng.counter = counter;
        chip.clock = clock;
        for _ in 0..count {
            let id = u32::from_be_bytes(take(4)?.try_into().ok()?);
            let object = match take(1)?[0] {
                1 => Object::P256(p256::ecdsa::SigningKey::from_slice(take(32)?).ok()?),
                2 => Object::Ed25519(ed25519_dalek::SigningKey::from_bytes(
                    take(32)?.try_into().ok()?,
                )),
                3 => Object::Hmac(take(32)?.try_into().ok()?),
                4 => Object::Counter(u64::from_be_bytes(take(8)?.try_into().ok()?)),
                _ => return None,
            };
            if chip.objects.insert(id, object).is_some() {
                return None;
            }
        }
        if !r.is_empty() {
            return None;
        }
        Some(chip)
    }
}

#[cfg(all(feature = "std", not(target_os = "none")))]
impl Default for SoftChip {
    fn default() -> Self {
        SoftChip::new()
    }
}

fn hex_byte(s: &[u8], i: usize) -> u8 {
    fn nib(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => 0,
        }
    }
    nib(s[2 * i]) << 4 | nib(s[2 * i + 1])
}

fn public_of(o: &Object) -> Result<Vec<u8>, ChipError> {
    match o {
        Object::P256(k) => Ok(k.verifying_key().to_encoded_point(true).as_bytes().to_vec()),
        Object::Ed25519(k) => Ok(k.verifying_key().to_bytes().to_vec()),
        Object::Hmac(_) | Object::Counter(_) => Err(ChipError::NoSuchKey),
    }
}

impl ChipOps for SoftChip {
    fn version(&mut self) -> Result<Version, ChipError> {
        Ok(Version { major: 7, minor: 2, patch: 0 })
    }

    fn random(&mut self, n: usize) -> Result<Vec<u8>, ChipError> {
        let mut v = alloc::vec![0u8; n];
        self.rng.fill_bytes(&mut v);
        Ok(v)
    }

    fn generate(&mut self, id: KeyId, curve: Curve) -> Result<Vec<u8>, ChipError> {
        let object = match curve {
            Curve::P256 => Object::P256(p256::ecdsa::SigningKey::random(&mut self.rng)),
            Curve::Ed25519 => Object::Ed25519(ed25519_dalek::SigningKey::generate(&mut self.rng)),
        };
        let public = public_of(&object)?;
        self.objects.insert(id.0, object);
        Ok(public)
    }

    fn public(&mut self, id: KeyId) -> Result<Vec<u8>, ChipError> {
        public_of(self.objects.get(&id.0).ok_or(ChipError::NoSuchKey)?)
    }

    fn delete(&mut self, id: KeyId) -> Result<(), ChipError> {
        self.objects.remove(&id.0).map(|_| ()).ok_or(ChipError::NoSuchKey)
    }

    fn exists(&mut self, id: KeyId) -> Result<bool, ChipError> {
        Ok(self.objects.contains_key(&id.0))
    }

    fn ids(&mut self) -> Result<Vec<KeyId>, ChipError> {
        Ok(self.objects.keys().map(|&k| KeyId(k)).filter(|k| k.in_range()).collect())
    }

    fn sign_p256(&mut self, id: KeyId, digest: &[u8; 32]) -> Result<Signature64, ChipError> {
        match self.objects.get(&id.0) {
            Some(Object::P256(k)) => {
                let sig: p256::ecdsa::Signature =
                    k.sign_prehash(digest).map_err(|_| ChipError::Status(0x6A80))?;
                Ok(sig.to_bytes().into())
            }
            _ => Err(ChipError::NoSuchKey),
        }
    }

    fn sign_ed25519(&mut self, id: KeyId, message: &[u8]) -> Result<Signature64, ChipError> {
        match self.objects.get(&id.0) {
            Some(Object::Ed25519(k)) => Ok(k.sign(message).to_bytes()),
            _ => Err(ChipError::NoSuchKey),
        }
    }

    fn attest(&mut self, id: KeyId, freshness: &[u8; 16]) -> Result<Attested, ChipError> {
        let public = public_of(self.objects.get(&id.0).ok_or(ChipError::NoSuchKey)?)?;
        self.clock += 1;
        let timestamp = self.clock.to_be_bytes().to_vec();
        let mut body = public.clone();
        body.extend_from_slice(SOFT_UID);
        body.extend_from_slice(freshness);
        body.extend_from_slice(&timestamp);
        let sig: p256::ecdsa::Signature = self.ca.sign(&body);
        Ok(Attested {
            public,
            uid: SOFT_UID.to_vec(),
            freshness: *freshness,
            timestamp,
            signature: sig.to_bytes().to_vec(),
            attest_public: self.ca.verifying_key().to_encoded_point(false).as_bytes().to_vec(),
            attest_cert_der: testca::ROOT_DER.to_vec(),
        })
    }

    fn put_hmac_key(&mut self, id: KeyId, key: &[u8; 32]) -> Result<(), ChipError> {
        self.objects.insert(id.0, Object::Hmac(*key));
        Ok(())
    }

    fn hmac(&mut self, id: KeyId, data: &[u8]) -> Result<[u8; 32], ChipError> {
        match self.objects.get(&id.0) {
            Some(Object::Hmac(k)) => {
                let mut m = <Hmac<Sha256> as Mac>::new_from_slice(k).map_err(|_| ChipError::Status(0x6A80))?;
                m.update(data);
                Ok(m.finalize().into_bytes().into())
            }
            _ => Err(ChipError::NoSuchKey),
        }
    }

    fn put_counter(&mut self, id: KeyId, value: u64) -> Result<(), ChipError> {
        self.objects.insert(id.0, Object::Counter(value));
        Ok(())
    }

    fn counter_next(&mut self, id: KeyId) -> Result<u64, ChipError> {
        match self.objects.get_mut(&id.0) {
            Some(Object::Counter(c)) => {
                *c = c.checked_add(1).ok_or(ChipError::Status(0x6985))?;
                Ok(*c)
            }
            _ => Err(ChipError::NoSuchKey),
        }
    }
}
