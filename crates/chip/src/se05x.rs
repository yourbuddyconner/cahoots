//! The real chip: an NXP SE051 on I2C, through Nitrokey's `se05x` driver (LGPL-3.0-only,
//! used only from the firmware, behind [`ChipOps`] so it can be replaced; see
//! `crates/rp2350/README.md`).
//!
//! Object ids are the Cahoots ids as they are ([`KeyId`] is already the 32-bit id the applet
//! uses), so a seat key is object `0x4C41_01ss`, a root `0x4C41_02ss` and an epoch counter
//! `0x4C41_03ss`. Every command runs in the default session: no user id, no SCP03.
//!
//! What is not yet checked against a real chip is marked "bench" below: the Ed25519 byte
//! order, the counter increment, the attestation object ids and the attested payload.

use crate::ids::KeyId;
use crate::ops::{Attested, ChipError, ChipOps, Curve, Signature64, Version};
use alloc::vec;
use alloc::vec::Vec;
use embedded_hal::delay::DelayNs;
use embedded_hal::i2c::{ErrorKind, I2c, NoAcknowledgeSource};
use se05x::se05x::commands::{
    CheckObjectExists, DeleteSecureObject, EcdsaSign, EddsaSign, GetRandom, GetVersion,
    MacOneShotGenerate, ReadAttestObject, ReadEcCurveList, ReadIdList, ReadObject, ReadSize,
    WriteCounter, WriteEcKey, WriteSymmKey,
};
use se05x::se05x::constants::PRIME256V1_INITIALIZER;
use se05x::se05x::policies::{ObjectAccessRule, ObjectPolicyFlags, Policy, PolicySet};
use se05x::se05x::{
    AttestationAlgo, Be, CounterSize, EcCurve, EcDsaSignatureAlgo, Error as SeError, MacAlgo,
    ObjectId, P1KeyType, Se05X, SecureObjectFilter, SymmKeyType,
};
use se05x::t1::{Error as T1Error, I2CErrorNack};

/// The SE05x family's I2C address.
pub const ADDRESS: u8 = 0x48;

/// The attestation key NXP provisions in the chip: the device-specific ECC key pair, and the
/// binary object holding its certificate. From NXP AN12436 "SE050 configurations" and the
/// SE051 user guidelines (the middleware's `ex_attest` uses the same pair). Unconfirmed for
/// our SE051 part until the first real bundle is read on the bench.
pub const ATTEST_KEY: u32 = 0xF000_0012;
pub const ATTEST_CERT: u32 = 0xF000_0013;

/// NXP's middleware stores and returns Ed25519 public keys and the two halves of a signature
/// in little-endian order, the reverse of RFC 8032. Bench: confirm with one signature checked
/// on the laptop; nothing in the badge makes Ed25519 keys today.
const ED25519_REVERSED: bool = true;

/// The response buffer: the largest response is an attestation (about 250 bytes) or a
/// chunk of the certificate; the applet's longest APDU payload is 889 bytes.
const BUF: usize = 1024;
/// The certificate is read in chunks of this many bytes.
const CHUNK: u16 = 512;

/// The I2C bus as the driver wants it, with the bus error reduced to its kind so the driver
/// can see a NACK. The driver's own embedded-hal 1.0 wrapper requires the HAL's error type
/// to be `ErrorKind` itself, which the RP2350 HAL's is not; this adapter is the thin step
/// between them.
pub struct Bus<I>(pub I);

/// A bus error, by kind.
#[derive(Debug, Clone, Copy)]
pub struct BusError(pub ErrorKind);

impl I2CErrorNack for BusError {
    fn is_address_nack(&self) -> bool {
        matches!(self.0, ErrorKind::NoAcknowledge(NoAcknowledgeSource::Address))
    }
    fn is_data_nack(&self) -> bool {
        matches!(self.0, ErrorKind::NoAcknowledge(NoAcknowledgeSource::Data))
    }
}

fn kind<E: embedded_hal::i2c::Error>(e: E) -> BusError {
    BusError(e.kind())
}

impl<I: I2c> se05x::embedded_hal::i2c::Read<u8> for Bus<I> {
    type Error = BusError;
    fn read(&mut self, address: u8, buffer: &mut [u8]) -> Result<(), BusError> {
        self.0.read(address, buffer).map_err(kind)
    }
}

impl<I: I2c> se05x::embedded_hal::i2c::Write<u8> for Bus<I> {
    type Error = BusError;
    fn write(&mut self, address: u8, bytes: &[u8]) -> Result<(), BusError> {
        self.0.write(address, bytes).map_err(kind)
    }
}

impl<I: I2c> se05x::embedded_hal::i2c::WriteRead<u8> for Bus<I> {
    type Error = BusError;
    fn write_read(&mut self, address: u8, bytes: &[u8], buffer: &mut [u8]) -> Result<(), BusError> {
        self.0.write_read(address, bytes, buffer).map_err(kind)
    }
}

/// The delay as the driver wants it.
pub struct Wait<D>(pub D);

impl<D: DelayNs> se05x::embedded_hal::Delay for Wait<D> {
    fn delay_us(&mut self, us: u32) {
        self.0.delay_us(us);
    }
}

fn err(e: SeError) -> ChipError {
    match e {
        SeError::T1(T1Error::AddressNack) => ChipError::NoChip,
        SeError::T1(_) | SeError::Line(_) | SeError::Tlv | SeError::Unknown => ChipError::Transport,
        SeError::Status(s) => ChipError::Status(s.to_u16()),
    }
}

fn oid(id: KeyId) -> ObjectId {
    ObjectId(id.0.to_be_bytes())
}

/// The default session, which every command here runs in.
const ALL_SESSIONS: ObjectId = ObjectId([0, 0, 0, 0]);

/// A seat key signs, shows its public key, can be attested and deleted; nothing else. The
/// private key cannot be read in any case.
const SEAT_POLICY: [Policy; 1] = [Policy {
    object_id: ALL_SESSIONS,
    access_rule: ObjectAccessRule::from_flags(
        ObjectPolicyFlags::ALLOW_SIGN
            .union(ObjectPolicyFlags::ALLOW_VERIFY)
            .union(ObjectPolicyFlags::ALLOW_READ)
            .union(ObjectPolicyFlags::ALLOW_ATTESTATION)
            .union(ObjectPolicyFlags::ALLOW_DELETE),
    ),
}];

/// A root computes HMACs (the applet counts MAC generation as signing) and can be deleted;
/// it is never read back.
const ROOT_POLICY: [Policy; 1] = [Policy {
    object_id: ALL_SESSIONS,
    access_rule: ObjectAccessRule::from_flags(
        ObjectPolicyFlags::ALLOW_SIGN.union(ObjectPolicyFlags::ALLOW_DELETE),
    ),
}];

/// SEC1 compressed from the uncompressed `04 || X || Y` the applet returns.
fn compress(point: &[u8]) -> Result<Vec<u8>, ChipError> {
    match point {
        [0x04, rest @ ..] if rest.len() == 64 => {
            let mut out = Vec::with_capacity(33);
            out.push(0x02 | (rest[63] & 1));
            out.extend_from_slice(&rest[..32]);
            Ok(out)
        }
        _ => Err(ChipError::Transport),
    }
}

/// One DER INTEGER as 32 big-endian bytes, and what follows it.
fn der_int(b: &[u8]) -> Option<([u8; 32], &[u8])> {
    let (&tag, b) = b.split_first()?;
    let (&len, b) = b.split_first()?;
    let len = len as usize;
    if tag != 0x02 || len == 0 || len > 33 || b.len() < len {
        return None;
    }
    let (v, rest) = b.split_at(len);
    let v = match v {
        [0, tail @ ..] if v.len() == 33 => tail,
        _ if v.len() == 33 => return None,
        _ => v,
    };
    let mut out = [0u8; 32];
    out[32 - v.len()..].copy_from_slice(v);
    Some((out, rest))
}

/// `SEQUENCE { INTEGER r, INTEGER s }` as raw `r || s`.
fn der_to_raw(sig: &[u8]) -> Result<Signature64, ChipError> {
    let parse = || {
        let (&tag, b) = sig.split_first()?;
        let (&len, b) = b.split_first()?;
        if tag != 0x30 || len as usize != b.len() {
            return None;
        }
        let (r, b) = der_int(b)?;
        let (s, b) = der_int(b)?;
        if !b.is_empty() {
            return None;
        }
        let mut out = [0u8; 64];
        out[..32].copy_from_slice(&r);
        out[32..].copy_from_slice(&s);
        Some(out)
    };
    parse().ok_or(ChipError::Transport)
}

/// The P-256 public key in a DER certificate: the bytes after the fixed SubjectPublicKeyInfo
/// header for an `id-ecPublicKey` on `prime256v1`.
fn cert_p256_key(cert: &[u8]) -> Option<Vec<u8>> {
    const SPKI: [u8; 26] = [
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];
    let at = cert.windows(SPKI.len()).position(|w| w == SPKI)? + SPKI.len();
    cert.get(at..at + 65).map(<[u8]>::to_vec)
}

fn reversed(b: &[u8]) -> Vec<u8> {
    b.iter().rev().copied().collect()
}

/// The SE051 as a [`ChipOps`].
pub struct Se05xChip<I: I2c, D: DelayNs> {
    se: Se05X<Bus<I>, Wait<D>>,
    buf: Vec<u8>,
}

impl<I: I2c, D: DelayNs> Se05xChip<I, D> {
    /// Wake the chip, select the applet, and create the P-256 curve object if the chip does
    /// not have it yet (once in a chip's life; Ed25519 needs none). `NoChip` when nothing
    /// answers at [`ADDRESS`].
    pub fn new(i2c: I, delay: D) -> Result<Self, ChipError> {
        let mut chip = Se05xChip {
            se: Se05X::new(Bus(i2c), ADDRESS, Wait(delay)),
            buf: vec![0u8; BUF],
        };
        chip.se.enable().map_err(err)?;
        let has_p256 = chip
            .se
            .run_command(&ReadEcCurveList {}, &mut chip.buf)
            .map_err(err)?
            .is_set(EcCurve::NistP256);
        if !has_p256 {
            chip.se.create_and_set_curve_params(&PRIME256V1_INITIALIZER).map_err(err)?;
        }
        Ok(chip)
    }

    fn read_object(&mut self, id: ObjectId) -> Result<Vec<u8>, ChipError> {
        let r = self
            .se
            .run_command(&ReadObject { object_id: id, offset: None, length: None, rsa_key_component: None }, &mut self.buf)
            .map_err(err)?;
        Ok(r.data.to_vec())
    }

    /// A binary object of any size, in chunks.
    fn read_file(&mut self, id: ObjectId) -> Result<Vec<u8>, ChipError> {
        let size = self.se.run_command(&ReadSize { object_id: id }, &mut self.buf).map_err(err)?.size.0;
        let size = u16::try_from(size).map_err(|_| ChipError::Transport)?;
        let mut out = Vec::with_capacity(size as usize);
        let mut at = 0u16;
        while at < size {
            let n = CHUNK.min(size - at);
            let r = self
                .se
                .run_command(
                    &ReadObject { object_id: id, offset: Some(Be(at)), length: Some(Be(n)), rsa_key_component: None },
                    &mut self.buf,
                )
                .map_err(err)?;
            if r.data.len() != n as usize {
                return Err(ChipError::Transport);
            }
            out.extend_from_slice(r.data);
            at += n;
        }
        Ok(out)
    }

    /// Delete the object if it is there, so a write makes it fresh with our policy.
    fn clear(&mut self, id: KeyId) -> Result<(), ChipError> {
        if self.exists(id)? {
            self.delete(id)?;
        }
        Ok(())
    }

    /// Bench: try the counter commands on a scratch object and report each answer, so the
    /// applet's expectations can be read off a boot log instead of guessed from the data
    /// sheet. Leaves nothing behind.
    pub fn probe_counter(&mut self) -> Vec<alloc::string::String> {
        use alloc::format;
        let id = KeyId::new(crate::ids::KeyKind::Epoch, 0xFF);
        let mut out = Vec::new();
        let _ = self.clear(id);
        let mut run = |me: &mut Self, what: &str, cmd: WriteCounter| {
            let r = me.se.run_command(&cmd, &mut me.buf).map_err(err);
            let read = me.read_object(oid(id));
            out.push(format!("{what}: {r:?}; read {read:?}"));
        };
        run(self, "size 8, value 0", WriteCounter { transient: false, policy: None, object_id: oid(id), data: Some(CounterSize::B8), value: Some(Be(0)) });
        let _ = self.clear(id);
        run(self, "size 8, no value", WriteCounter { transient: false, policy: None, object_id: oid(id), data: Some(CounterSize::B8), value: None });
        run(self, "then increment (no size, no value)", WriteCounter { transient: false, policy: None, object_id: oid(id), data: None, value: None });
        run(self, "then set value 5 (no size)", WriteCounter { transient: false, policy: None, object_id: oid(id), data: None, value: Some(Be(5)) });
        let _ = self.clear(id);
        run(self, "size 8, value 1", WriteCounter { transient: false, policy: None, object_id: oid(id), data: Some(CounterSize::B8), value: Some(Be(1)) });
        let _ = self.clear(id);
        run(self, "no size, value 0 on a fresh id", WriteCounter { transient: false, policy: None, object_id: oid(id), data: None, value: Some(Be(0)) });
        let _ = self.clear(id);
        run(self, "size 4, no value", WriteCounter { transient: false, policy: None, object_id: oid(id), data: Some(CounterSize::B4), value: None });
        run(self, "then increment", WriteCounter { transient: false, policy: None, object_id: oid(id), data: None, value: None });
        let _ = self.clear(id);
        out
    }

    /// The key's public key as the applet returns it: `04 || X || Y` for P-256, 32 bytes for
    /// Ed25519. An HMAC key or a counter has none.
    fn public_raw(&mut self, id: KeyId) -> Result<Vec<u8>, ChipError> {
        if !self.exists(id)? {
            return Err(ChipError::NoSuchKey);
        }
        // An HMAC key refuses the read; a counter reads as its 8-byte value.
        let raw = match self.read_object(oid(id)) {
            Err(ChipError::Status(_)) => return Err(ChipError::NoSuchKey),
            r => r?,
        };
        match raw.len() {
            65 | 32 => Ok(raw),
            _ => Err(ChipError::NoSuchKey),
        }
    }

    fn public_of_raw(raw: &[u8]) -> Result<Vec<u8>, ChipError> {
        match raw.len() {
            65 => compress(raw),
            32 if ED25519_REVERSED => Ok(reversed(raw)),
            32 => Ok(raw.to_vec()),
            _ => Err(ChipError::NoSuchKey),
        }
    }
}

impl<I: I2c, D: DelayNs> ChipOps for Se05xChip<I, D> {
    fn version(&mut self) -> Result<Version, ChipError> {
        let v = self.se.run_command(&GetVersion {}, &mut self.buf).map_err(err)?.version_info;
        Ok(Version { major: v.major, minor: v.minor, patch: v.patch })
    }

    fn random(&mut self, n: usize) -> Result<Vec<u8>, ChipError> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            let k = (n - out.len()).min(CHUNK as usize) as u16;
            let r = self.se.run_command(&GetRandom { length: Be(k) }, &mut self.buf).map_err(err)?;
            if r.data.len() != k as usize {
                return Err(ChipError::Transport);
            }
            out.extend_from_slice(r.data);
        }
        Ok(out)
    }

    fn generate(&mut self, id: KeyId, curve: Curve) -> Result<Vec<u8>, ChipError> {
        self.clear(id)?;
        let curve = match curve {
            Curve::P256 => EcCurve::NistP256,
            Curve::Ed25519 => EcCurve::IdEccEd25519,
        };
        self.se
            .run_command(
                &WriteEcKey {
                    transient: false,
                    is_auth: false,
                    key_type: Some(P1KeyType::KeyPair),
                    policy: Some(PolicySet(&SEAT_POLICY)),
                    max_attempts: None,
                    object_id: oid(id),
                    curve: Some(curve),
                    private_key: None,
                    public_key: None,
                },
                &mut self.buf,
            )
            .map_err(err)?;
        self.public(id)
    }

    fn public(&mut self, id: KeyId) -> Result<Vec<u8>, ChipError> {
        let raw = self.public_raw(id)?;
        Self::public_of_raw(&raw)
    }

    fn delete(&mut self, id: KeyId) -> Result<(), ChipError> {
        if !self.exists(id)? {
            return Err(ChipError::NoSuchKey);
        }
        self.se.run_command(&DeleteSecureObject { object_id: oid(id) }, &mut self.buf).map_err(err)?;
        Ok(())
    }

    fn exists(&mut self, id: KeyId) -> Result<bool, ChipError> {
        let r = self.se.run_command(&CheckObjectExists { object_id: oid(id) }, &mut self.buf).map_err(err)?;
        Ok(r.result.is_success())
    }

    fn ids(&mut self) -> Result<Vec<KeyId>, ChipError> {
        let mut out = Vec::new();
        let mut offset = 0u16;
        loop {
            let r = self
                .se
                .run_command(&ReadIdList { offset: Be(offset), filter: SecureObjectFilter::All }, &mut self.buf)
                .map_err(err)?;
            let more = r.more.is_more();
            let chunks = r.ids.chunks_exact(4);
            let n = chunks.len();
            out.extend(
                chunks
                    .map(|c| KeyId(u32::from_be_bytes([c[0], c[1], c[2], c[3]])))
                    .filter(|k| k.in_range()),
            );
            if !more || n == 0 {
                break;
            }
            offset = offset.saturating_add(n as u16);
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    fn sign_p256(&mut self, id: KeyId, digest: &[u8; 32]) -> Result<Signature64, ChipError> {
        let r = self
            .se
            .run_command(
                &EcdsaSign { key_id: oid(id), algo: EcDsaSignatureAlgo::Sha256, data: digest },
                &mut self.buf,
            )
            .map_err(err)?;
        der_to_raw(r.signature)
    }

    fn sign_ed25519(&mut self, id: KeyId, message: &[u8]) -> Result<Signature64, ChipError> {
        let r = self
            .se
            .run_command(&EddsaSign { key_id: oid(id), data: message }, &mut self.buf)
            .map_err(err)?;
        let sig: Signature64 = r.signature.try_into().map_err(|_| ChipError::Transport)?;
        if !ED25519_REVERSED {
            return Ok(sig);
        }
        let mut out = [0u8; 64];
        out[..32].copy_from_slice(&reversed(&sig[..32]));
        out[32..].copy_from_slice(&reversed(&sig[32..]));
        Ok(out)
    }

    /// The applet's attested read of the key's public part, signed by NXP's provisioned
    /// attestation key ([`ATTEST_KEY`]), with that key's certificate ([`ATTEST_CERT`]).
    ///
    /// Bench: the applet signs the TLV-encoded response (the public key, the object's
    /// attributes, the timestamp, the freshness and the chip id, each with its tag and
    /// length), not the bare concatenation `crates/objects/src/attestation.rs`'s
    /// `attested_payload` builds for the software chip, and `Attested` has no field for the
    /// attributes. Here `public` is the key as the applet returned it (`04 || X || Y`), `uid`
    /// the 18-byte chip id, `timestamp` the 12-byte timestamp and `signature` the DER
    /// signature; `attested_payload` must be matched to the real layout on the bench. The
    /// default session is used; if the attestation key's policy wants a user-id session the
    /// call fails with a status and that is the first thing to look at.
    fn attest(&mut self, id: KeyId, freshness: &[u8; 16]) -> Result<Attested, ChipError> {
        if !self.exists(id)? {
            return Err(ChipError::NoSuchKey);
        }
        let attested = {
            let r = self
                .se
                .run_command(
                    &ReadAttestObject {
                        object_id: oid(id),
                        offset: None,
                        length: None,
                        rsa_key_component: None,
                        attestation_object: ObjectId(ATTEST_KEY.to_be_bytes()),
                        attestation_algo: AttestationAlgo::ECdsaSha256,
                        freshness_random: freshness,
                    },
                    &mut self.buf,
                )
                .map_err(err)?;
            if r.freshness_random != freshness {
                return Err(ChipError::Transport);
            }
            Attested {
                public: r.data.ok_or(ChipError::NoSuchKey)?.to_vec(),
                uid: r.chip_unique_id.to_vec(),
                freshness: *freshness,
                timestamp: r.timestamp.to_vec(),
                signature: r.signature.to_vec(),
                attest_public: Vec::new(),
                attest_cert_der: Vec::new(),
            }
        };
        let cert = self.read_file(ObjectId(ATTEST_CERT.to_be_bytes()))?;
        let attest_public = cert_p256_key(&cert).ok_or(ChipError::Transport)?;
        Ok(Attested { attest_public, attest_cert_der: cert, ..attested })
    }

    fn put_hmac_key(&mut self, id: KeyId, key: &[u8; 32]) -> Result<(), ChipError> {
        self.clear(id)?;
        self.se
            .run_command(
                &WriteSymmKey {
                    transient: false,
                    is_auth: false,
                    key_type: SymmKeyType::Hmac,
                    policy: Some(PolicySet(&ROOT_POLICY)),
                    max_attempts: None,
                    object_id: oid(id),
                    kek_id: None,
                    value: key,
                },
                &mut self.buf,
            )
            .map_err(err)?;
        Ok(())
    }

    fn hmac(&mut self, id: KeyId, data: &[u8]) -> Result<[u8; 32], ChipError> {
        let r = self
            .se
            .run_command(
                &MacOneShotGenerate { key_id: oid(id), algo: MacAlgo::HmacSha256, data },
                &mut self.buf,
            )
            .map_err(err)?;
        r.tag.try_into().map_err(|_| ChipError::Transport)
    }

    /// An 8-byte counter, made fresh: the applet's counters only go up, so an old one is
    /// deleted first. The applet (7.2.0, on the bench 2026-10-29) refuses a `WriteCounter`
    /// that carries both a size and a value with 0x6A80, so the counter is created with its
    /// size alone, at zero, and a starting value other than zero is a second write.
    fn put_counter(&mut self, id: KeyId, value: u64) -> Result<(), ChipError> {
        self.clear(id)?;
        self.se
            .run_command(
                &WriteCounter {
                    transient: false,
                    policy: None,
                    object_id: oid(id),
                    data: Some(CounterSize::B8),
                    value: None,
                },
                &mut self.buf,
            )
            .map_err(err)?;
        if value != 0 {
            self.se
                .run_command(
                    &WriteCounter {
                        transient: false,
                        policy: None,
                        object_id: oid(id),
                        data: None,
                        value: Some(Be(value)),
                    },
                    &mut self.buf,
                )
                .map_err(err)?;
        }
        Ok(())
    }

    /// The driver has no increment command of its own: per the applet's APDU specification
    /// (AN12413), `WriteCounter` on an existing counter with neither size nor value adds
    /// one. Then the counter is read back. Confirmed on the bench, 2026-10-29: 0 became 1.
    fn counter_next(&mut self, id: KeyId) -> Result<u64, ChipError> {
        if !self.exists(id)? {
            return Err(ChipError::NoSuchKey);
        }
        self.se
            .run_command(
                &WriteCounter { transient: false, policy: None, object_id: oid(id), data: None, value: None },
                &mut self.buf,
            )
            .map_err(err)?;
        let raw = self.read_object(oid(id))?;
        if raw.is_empty() || raw.len() > 8 {
            return Err(ChipError::Transport);
        }
        let mut be = [0u8; 8];
        be[8 - raw.len()..].copy_from_slice(&raw);
        Ok(u64::from_be_bytes(be))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn der_signature_to_raw() {
        // r with a leading zero (high bit set), s short.
        let mut der = vec![0x30, 0x00, 0x02, 0x21, 0x00];
        der.extend_from_slice(&[0x80; 32]);
        der.extend_from_slice(&[0x02, 0x1f]);
        der.extend_from_slice(&[0x11; 31]);
        der[1] = (der.len() - 2) as u8;
        let raw = der_to_raw(&der).unwrap();
        assert_eq!(&raw[..32], &[0x80; 32]);
        assert_eq!(raw[32], 0);
        assert_eq!(&raw[33..], &[0x11; 31]);
        assert!(der_to_raw(&der[..der.len() - 1]).is_err());
        assert!(der_to_raw(&[]).is_err());
    }

    #[test]
    fn compresses_points() {
        let mut p = vec![0x04];
        p.extend_from_slice(&[7; 32]);
        p.extend_from_slice(&[0; 31]);
        p.push(3);
        let c = compress(&p).unwrap();
        assert_eq!(c[0], 0x03);
        assert_eq!(&c[1..], &[7; 32]);
        assert!(compress(&p[..64]).is_err());
    }
}
