//! The real chip: an NXP SE051 on I2C, through Nitrokey's `se05x` driver (LGPL-3.0-only,
//! used only from the firmware, behind [`ChipOps`] so it can be replaced; see
//! `crates/rp2350/README.md`).
//!
//! Object ids are the Cahoots ids as they are ([`KeyId`] is already the 32-bit id the applet
//! uses), so a seat key is object `0x4C41_01ss`, a root `0x4C41_02ss` and an epoch counter
//! `0x4C41_03ss`.
//!
//! The badge owns the chip through a user-id session: [`ChipOps::open_owner`] sends
//! `CreateSession` for the owner object and `VerifySessionUserId` with the owner secret, and
//! from then on every command goes through `ProcessSessionCmd` in that session until
//! [`ChipOps::close_session`]. Before the session opens, commands run in no session.
//!
//! A policy names identities by authentication object id: one `Policy` per identity the
//! [`ObjectPolicy`] names, `ObjectId::INVALID` (all zeros) for no session, the owner object's
//! id, the phone slot's id. The slot's entry carries `REQUIRE_SM` when the policy asks.
//!
//! What is not yet checked against a real chip is marked "bench" below: the Ed25519 byte
//! order, the counter increment, the attestation object ids and the attested payload.
//!
//! What a command in no session may make at an id that holds nothing (bench, SE051 applet
//! 7.2.0, 2026-10-31, `cahoots-host chip raw` from no session, each at an empty id in the
//! Cahoots range):
//!
//! - `WriteBinary`: `9000`, an object with every right for every session (the Task 5 bench).
//! - `WriteSymmKey` for an HMAC key (`80 01 05 00`): `9000`.
//! - `WriteSymmKey` for an AES key with `is_auth` (`80 41 03 00`): `9000`. The object it made
//!   could not then be deleted in no session (`6986`).
//! - `WriteSymmKey` for a plain AES key (`80 01 03 00`): `9000`.
//! - `WriteECKey` generating a P-256 key pair (`80 01 61 00`): `9000`.
//! - `WriteCounter`, 8 bytes (`80 01 08 00`): `9000`.
//!
//! So a tap can make any kind of object at an empty id. The badge never trusts one: it flags
//! its record before it empties an antenna-facing id (`cahoots_badge::chip::ensure_owned`),
//! deletes whatever stands at an id before it makes its own there, and a phone reads only the
//! circle slots the badge's pairing object names.

use crate::ids::KeyId;
use crate::ops::{Attested, ChipError, ChipOps, Curve, Identity, ObjectPolicy, Signature64, Version};
use alloc::vec;
use alloc::vec::Vec;
use embedded_hal::delay::DelayNs;
use embedded_hal::i2c::{ErrorKind, I2c, NoAcknowledgeSource};
use se05x::se05x::commands::{
    CheckObjectExists, CloseSession, CreateSession, DeleteSecureObject, EcdsaSign, EddsaSign,
    GetRandom, GetVersion, MacOneShotGenerate, ReadAttestObject, ReadEcCurveList, ReadIdList,
    ReadObject, VerifySessionUserId, WriteBinary, WriteCounter, WriteEcKey,
    WriteSymmKey, WriteUserId,
};
use se05x::se05x::constants::PRIME256V1_INITIALIZER;
use se05x::se05x::policies::{ObjectAccessRule, ObjectPolicyFlags, Policy, PolicySet};
use se05x::se05x::{
    AttestationAlgo, Be, CounterSize, EcCurve, EcDsaSignatureAlgo, Error as SeError, MacAlgo,
    ObjectId, P1KeyType, Se05X, SecureObjectFilter, SessionId, SymmKeyType,
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
/// The certificate and binary objects are read in chunks of this many bytes: well under the
/// applet's 889-byte APDU payload with the session wrapper and the tags around it.
const CHUNK: u16 = 512;
/// A binary object is written in chunks of this many bytes, so that every command, wrapped
/// in the owner session with a policy and its tags (66 bytes at most), fits one T=1 frame of
/// 254 bytes. A longer command goes out in chained frames, and the driver reads the chip's
/// acknowledgement of each frame once, with no retry while the chip is still busy and
/// refuses its address: on the bench (SE051, applet 7.2.0, 2026-10-31) about half the
/// commands over 254 bytes failed that way, and every shorter one passed.
const WRITE_CHUNK: usize = 160;

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
        SeError::Status(s) => match s.to_u16() {
            // The applet answers a call its policy refuses with 6986 (bench, 2026-10-31).
            0x6982 | 0x6986 => ChipError::NotAllowed,
            0x6A89 => ChipError::Exists,
            w => ChipError::Status(w),
        },
    }
}

fn oid(id: KeyId) -> ObjectId {
    ObjectId(id.0.to_be_bytes())
}

/// The applet's object id for an identity in a policy: all zeros for no session.
fn identity_oid(i: Identity) -> ObjectId {
    match i.auth_id() {
        Some(id) => oid(id),
        None => ObjectId::INVALID,
    }
}

/// The applet's policy set for `p`: one entry per identity it names, with the flags of every
/// right it names that identity in, and `REQUIRE_SM` on the slot's entry when asked.
fn policies(p: &ObjectPolicy) -> Vec<Policy> {
    let rights: [(&[Identity], ObjectPolicyFlags); 5] = [
        (&p.sign, ObjectPolicyFlags::ALLOW_SIGN),
        (&p.read, ObjectPolicyFlags::ALLOW_READ),
        (&p.write, ObjectPolicyFlags::ALLOW_WRITE),
        (&p.delete, ObjectPolicyFlags::ALLOW_DELETE),
        (&p.attest, ObjectPolicyFlags::ALLOW_ATTESTATION),
    ];
    let mut out = Vec::new();
    for who in [Identity::None, Identity::Owner, Identity::Slot] {
        let mut flags = ObjectPolicyFlags::empty();
        for (ids, flag) in &rights {
            if ids.contains(&who) {
                flags |= *flag;
            }
        }
        if flags.is_empty() {
            continue;
        }
        if who == Identity::Slot && p.require_sm {
            flags |= ObjectPolicyFlags::REQUIRE_SM;
        }
        out.push(Policy { object_id: identity_oid(who), access_rule: ObjectAccessRule::from_flags(flags) });
    }
    out
}

/// Run `cmd` in the open session, or in no session when none is open.
macro_rules! run {
    ($chip:expr, $cmd:expr) => {
        match $chip.session {
            Some(sid) => $chip.se.run_session_command(sid, &$cmd, &mut $chip.buf),
            None => $chip.se.run_command(&$cmd, &mut $chip.buf),
        }
        .map_err(err)
    };
}

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

/// A command APDU as bytes, sent as it is: the bench's pass-through (`Se05xChip::raw`, with
/// the `bench` feature) and ReadSize ([`Se05xChip::read_size`]).
struct Raw<'a>(&'a [u8]);

/// The answer's data, as it is.
struct RawAnswer<'a>(&'a [u8]);

impl<'a> se05x::se05x::Se05XResponse<'a> for RawAnswer<'a> {
    fn from_response(data: &'a [u8]) -> Result<Self, SeError> {
        Ok(RawAnswer(data))
    }
}

impl iso7816::command::DataSource for Raw<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }
}

impl<W: iso7816::command::Writer> iso7816::command::DataStream<W> for Raw<'_> {
    fn to_writer(&self, writer: &mut W) -> Result<(), W::Error> {
        writer.write_all(self.0)
    }
}

impl<W: iso7816::command::Writer> se05x::se05x::Se05XCommand<W> for Raw<'_> {
    type Response<'a> = RawAnswer<'a>;
}

/// The SE051 as a [`ChipOps`].
pub struct Se05xChip<I: I2c, D: DelayNs> {
    se: Se05X<Bus<I>, Wait<D>>,
    buf: Vec<u8>,
    /// The open session, if any.
    session: Option<SessionId>,
}

impl<I: I2c, D: DelayNs> Se05xChip<I, D> {
    /// Wake the chip, select the applet, and create the P-256 curve object if the chip does
    /// not have it yet (once in a chip's life; Ed25519 needs none). `NoChip` when nothing
    /// answers at [`ADDRESS`].
    pub fn new(i2c: I, delay: D) -> Result<Self, ChipError> {
        let mut chip = Se05xChip {
            se: Se05X::new(Bus(i2c), ADDRESS, Wait(delay)),
            buf: vec![0u8; BUF],
            session: None,
        };
        chip.restart()?;
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

    /// Wake the chip again after its enable pin rose, in no session: the chip forgets its
    /// session when it loses power.
    pub fn restart(&mut self) -> Result<(), ChipError> {
        self.session = None;
        self.se.enable().map_err(err)?;
        Ok(())
    }

    /// Send `apdu` as it is, in no session, and return the answer: its data, then SW1 and
    /// SW2. For the bench's pass-through only: the laptop drives the applet as a phone would.
    /// An answer whose status is not `90 00` comes back as its status word alone, because the
    /// driver keeps no data with a failing status.
    #[cfg(feature = "bench")]
    pub fn raw(&mut self, apdu: &[u8]) -> Result<Vec<u8>, ChipError> {
        match self.se.run_command(&Raw(apdu), &mut self.buf) {
            Ok(RawAnswer(data)) => {
                let mut out = data.to_vec();
                out.extend_from_slice(&[0x90, 0x00]);
                Ok(out)
            }
            Err(SeError::Status(s)) => Ok(s.to_u16().to_be_bytes().to_vec()),
            Err(e) => Err(err(e)),
        }
    }

    /// True when a session is open.
    pub fn in_session(&self) -> bool {
        self.session.is_some()
    }

    fn read_object(&mut self, id: ObjectId) -> Result<Vec<u8>, ChipError> {
        let r = run!(self, ReadObject { object_id: id, offset: None, length: None, rsa_key_component: None })?;
        Ok(r.data.to_vec())
    }

    /// A binary object's size. The applet answers ReadSize with a 2-byte size (bench, SE051
    /// applet 7.2.0, 2026-10-31), which the driver's `ReadSize` refuses to parse (it wants 8
    /// bytes), so the command goes as bytes and the answer is read here.
    fn read_size(&mut self, id: ObjectId) -> Result<u16, ChipError> {
        let mut cmd = vec![0x80, 0x02, 0x00, 0x07, 0x06, 0x41, 0x04];
        cmd.extend_from_slice(&id.0);
        cmd.push(0x00);
        let a = run!(self, Raw(&cmd))?.0;
        match a {
            [0x41, n, size @ ..] if usize::from(*n) == size.len() && !size.is_empty() && size.len() <= 8 => {
                let v = size.iter().fold(0u64, |v, &b| v << 8 | u64::from(b));
                u16::try_from(v).map_err(|_| ChipError::Transport)
            }
            _ => Err(ChipError::Transport),
        }
    }

    /// A binary object of any size, in chunks.
    fn read_file(&mut self, id: ObjectId) -> Result<Vec<u8>, ChipError> {
        let size = self.read_size(id)?;
        let mut out = Vec::with_capacity(size as usize);
        let mut at = 0u16;
        while at < size {
            let n = CHUNK.min(size - at);
            let r = run!(self, ReadObject { object_id: id, offset: Some(Be(at)), length: Some(Be(n)), rsa_key_component: None })?;
            if r.data.len() != n as usize {
                return Err(ChipError::Transport);
            }
            out.extend_from_slice(r.data);
            at += n;
        }
        Ok(out)
    }

    /// `Exists` if an object has `id`: a creating call never replaces one, so a policy is
    /// only ever the one the object was made with.
    fn fresh(&mut self, id: KeyId) -> Result<(), ChipError> {
        if self.exists(id)? {
            return Err(ChipError::Exists);
        }
        Ok(())
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
        let v = run!(self, GetVersion {})?.version_info;
        Ok(Version { major: v.major, minor: v.minor, patch: v.patch })
    }

    fn random(&mut self, n: usize) -> Result<Vec<u8>, ChipError> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            let k = (n - out.len()).min(CHUNK as usize) as u16;
            let r = run!(self, GetRandom { length: Be(k) })?;
            if r.data.len() != k as usize {
                return Err(ChipError::Transport);
            }
            out.extend_from_slice(r.data);
        }
        Ok(out)
    }

    fn generate(&mut self, id: KeyId, curve: Curve, policy: &ObjectPolicy) -> Result<Vec<u8>, ChipError> {
        self.fresh(id)?;
        let policy = policies(policy);
        let curve = match curve {
            Curve::P256 => EcCurve::NistP256,
            Curve::Ed25519 => EcCurve::IdEccEd25519,
        };
        run!(self, WriteEcKey {
                    transient: false,
                    is_auth: false,
                    key_type: Some(P1KeyType::KeyPair),
                    policy: Some(PolicySet(&policy)),
                    max_attempts: None,
                    object_id: oid(id),
                    curve: Some(curve),
                    private_key: None,
                    public_key: None,
                })?;
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
        run!(self, DeleteSecureObject { object_id: oid(id) })?;
        Ok(())
    }

    fn exists(&mut self, id: KeyId) -> Result<bool, ChipError> {
        let r = run!(self, CheckObjectExists { object_id: oid(id) })?;
        Ok(r.result.is_success())
    }

    fn ids(&mut self) -> Result<Vec<KeyId>, ChipError> {
        let mut out = Vec::new();
        let mut offset = 0u16;
        loop {
            let r = run!(self, ReadIdList { offset: Be(offset), filter: SecureObjectFilter::All })?;
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
        let r = run!(self, EcdsaSign { key_id: oid(id), algo: EcDsaSignatureAlgo::Sha256, data: digest })?;
        der_to_raw(r.signature)
    }

    fn sign_ed25519(&mut self, id: KeyId, message: &[u8]) -> Result<Signature64, ChipError> {
        let r = run!(self, EddsaSign { key_id: oid(id), data: message })?;
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
    /// signature; `attested_payload` must be matched to the real layout on the bench. It runs
    /// in the open session, the owner's on the badge; if NXP's attestation key refuses that
    /// session the call fails with a status and that is the first thing to look at.
    fn attest(&mut self, id: KeyId, freshness: &[u8; 16]) -> Result<Attested, ChipError> {
        if !self.exists(id)? {
            return Err(ChipError::NoSuchKey);
        }
        let attested = {
            let r = run!(self, ReadAttestObject {
                        object_id: oid(id),
                        offset: None,
                        length: None,
                        rsa_key_component: None,
                        attestation_object: ObjectId(ATTEST_KEY.to_be_bytes()),
                        attestation_algo: AttestationAlgo::ECdsaSha256,
                        freshness_random: freshness,
                    })?;
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

    fn put_hmac_key(&mut self, id: KeyId, key: &[u8; 32], policy: &ObjectPolicy) -> Result<(), ChipError> {
        self.fresh(id)?;
        let policy = policies(policy);
        run!(self, WriteSymmKey {
                    transient: false,
                    is_auth: false,
                    key_type: SymmKeyType::Hmac,
                    policy: Some(PolicySet(&policy)),
                    max_attempts: None,
                    object_id: oid(id),
                    kek_id: None,
                    value: key,
                })?;
        Ok(())
    }

    fn hmac(&mut self, id: KeyId, data: &[u8]) -> Result<[u8; 32], ChipError> {
        let r = run!(self, MacOneShotGenerate { key_id: oid(id), algo: MacAlgo::HmacSha256, data })?;
        r.tag.try_into().map_err(|_| ChipError::Transport)
    }

    /// An 8-byte counter at a free id. The applet (7.2.0, on the bench 2026-10-29) refuses a `WriteCounter`
    /// that carries both a size and a value with 0x6A80, so the counter is created with its
    /// size alone, at zero, and a starting value other than zero is a second write.
    fn put_counter(&mut self, id: KeyId, value: u64, policy: &ObjectPolicy) -> Result<(), ChipError> {
        self.fresh(id)?;
        let policy = policies(policy);
        run!(self, WriteCounter {
                    transient: false,
                    policy: Some(PolicySet(&policy)),
                    object_id: oid(id),
                    data: Some(CounterSize::B8),
                    value: None,
                })?;
        if value != 0 {
            run!(self, WriteCounter {
                        transient: false,
                        policy: None,
                        object_id: oid(id),
                        data: None,
                        value: Some(Be(value)),
                    })?;
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
        run!(self, WriteCounter { transient: false, policy: None, object_id: oid(id), data: None, value: None })?;
        let raw = self.read_object(oid(id))?;
        if raw.is_empty() || raw.len() > 8 {
            return Err(ChipError::Transport);
        }
        let mut be = [0u8; 8];
        be[8 - raw.len()..].copy_from_slice(&raw);
        Ok(u64::from_be_bytes(be))
    }

    fn counter_get(&mut self, id: KeyId) -> Result<u64, ChipError> {
        if !self.exists(id)? {
            return Err(ChipError::NoSuchKey);
        }
        let raw = self.read_object(oid(id))?;
        let be: [u8; 8] = raw.as_slice().try_into().map_err(|_| ChipError::Transport)?;
        Ok(u64::from_be_bytes(be))
    }

    /// `CreateSession` for the owner object, then `VerifySessionUserId` with the secret inside
    /// it. A session already open is closed first. `NoSuchKey` if the chip has no owner object;
    /// `BadSession` if the session cannot be opened or the secret is refused.
    fn open_owner(&mut self, secret: &[u8; 16]) -> Result<(), ChipError> {
        self.close_session()?;
        if !self.exists(KeyId::OWNER)? {
            return Err(ChipError::NoSuchKey);
        }
        let sid = self
            .se
            .run_command(&CreateSession { object_id: oid(KeyId::OWNER) }, &mut self.buf)
            .map_err(err)?
            .session_id;
        let verified = self
            .se
            .run_session_command(sid, &VerifySessionUserId { user_id: secret }, &mut self.buf)
            .map(|_| ());
        if let Err(e) = verified {
            let _ = self.se.run_session_command(sid, &CloseSession {}, &mut self.buf);
            // A status word from VerifySessionUserId is the applet refusing the secret, so
            // `BadSession`. A bus or framing error is not: it may pass at the next try.
            return Err(match e {
                SeError::Status(_) => ChipError::BadSession,
                e => err(e),
            });
        }
        self.session = Some(sid);
        Ok(())
    }

    fn close_session(&mut self) -> Result<(), ChipError> {
        if let Some(sid) = self.session.take() {
            self.se.run_session_command(sid, &CloseSession {}, &mut self.buf).map_err(err)?;
        }
        Ok(())
    }

    /// `WriteUserId` without `MaxAttempts`. The applet (7.2.0) refuses the tag as the driver
    /// encodes it, one byte, with 6A80 for 0 and for 255 (bench, 2026-10-31). Without it, twelve
    /// wrong secrets in a row did not stop the right one opening the session, on the bench.
    fn put_user_id(&mut self, id: KeyId, secret: &[u8; 16], policy: &ObjectPolicy) -> Result<(), ChipError> {
        self.fresh(id)?;
        let policy = policies(policy);
        run!(
            self,
            WriteUserId {
                policy: Some(PolicySet(&policy)),
                max_attempts: None,
                object_id: oid(id),
                data: secret,
            }
        )?;
        Ok(())
    }

    /// `WriteSymmKey` with `is_auth` set: an AES key that opens an SCP03 session.
    fn put_aes_auth(&mut self, id: KeyId, key: &[u8; 16], policy: &ObjectPolicy) -> Result<(), ChipError> {
        self.fresh(id)?;
        let policy = policies(policy);
        run!(
            self,
            WriteSymmKey {
                transient: false,
                is_auth: true,
                key_type: SymmKeyType::Aes,
                policy: Some(PolicySet(&policy)),
                max_attempts: None,
                object_id: oid(id),
                kek_id: None,
                value: key,
            }
        )?;
        Ok(())
    }

    /// `WriteBinary` in chunks: the first makes the object with its length and policy, the
    /// rest write at their offsets.
    fn write_binary(&mut self, id: KeyId, data: &[u8], policy: &ObjectPolicy) -> Result<(), ChipError> {
        let len = u16::try_from(data.len()).map_err(|_| ChipError::Status(0x6A80))?;
        if len == 0 {
            return Err(ChipError::Status(0x6A80));
        }
        self.fresh(id)?;
        let policy = policies(policy);
        let first = &data[..data.len().min(WRITE_CHUNK)];
        run!(
            self,
            WriteBinary {
                transient: false,
                policy: Some(PolicySet(&policy)),
                object_id: oid(id),
                offset: Some(Be(0)),
                file_length: Some(Be(len)),
                data: Some(first),
            }
        )?;
        self.write_chunks(id, data, first.len())
    }

    fn rewrite_binary(&mut self, id: KeyId, data: &[u8]) -> Result<(), ChipError> {
        if !self.exists(id)? {
            return Err(ChipError::NoSuchKey);
        }
        self.write_chunks(id, data, 0)
    }

    fn read_binary(&mut self, id: KeyId) -> Result<Vec<u8>, ChipError> {
        if !self.exists(id)? {
            return Err(ChipError::NoSuchKey);
        }
        self.read_file(oid(id))
    }
}

impl<I: I2c, D: DelayNs> Se05xChip<I, D> {
    /// Write `data` from byte `from` on into the existing binary object, a chunk at a time.
    fn write_chunks(&mut self, id: KeyId, data: &[u8], from: usize) -> Result<(), ChipError> {
        let mut at = from;
        while at < data.len() {
            let end = data.len().min(at + WRITE_CHUNK);
            run!(
                self,
                WriteBinary {
                    transient: false,
                    policy: None,
                    object_id: oid(id),
                    offset: Some(Be(at as u16)),
                    file_length: None,
                    data: Some(&data[at..end]),
                }
            )?;
            at = end;
        }
        Ok(())
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

    fn rule(f: ObjectPolicyFlags) -> ObjectAccessRule {
        ObjectAccessRule::from_flags(f)
    }

    #[test]
    fn policies_name_each_identity_once() {
        use ObjectPolicyFlags as F;
        let owner = oid(KeyId::OWNER);
        let slot = oid(KeyId::SLOT);
        assert_eq!(
            policies(&ObjectPolicy::owner_only()),
            vec![Policy {
                object_id: owner,
                access_rule: rule(F::ALLOW_SIGN | F::ALLOW_READ | F::ALLOW_DELETE | F::ALLOW_ATTESTATION),
            }]
        );
        assert_eq!(
            policies(&ObjectPolicy::owner_counter()),
            vec![Policy {
                object_id: owner,
                access_rule: rule(F::ALLOW_READ | F::ALLOW_WRITE | F::ALLOW_DELETE),
            }]
        );
        assert_eq!(
            policies(&ObjectPolicy::owner_and_slot_mac()),
            vec![
                Policy { object_id: owner, access_rule: rule(F::ALLOW_SIGN | F::ALLOW_DELETE) },
                Policy { object_id: slot, access_rule: rule(F::ALLOW_SIGN | F::REQUIRE_SM) },
            ]
        );
        assert_eq!(
            policies(&ObjectPolicy::inbox()),
            vec![
                Policy { object_id: ObjectId::INVALID, access_rule: rule(F::ALLOW_WRITE) },
                Policy {
                    object_id: owner,
                    access_rule: rule(F::ALLOW_READ | F::ALLOW_WRITE | F::ALLOW_DELETE),
                },
                Policy { object_id: slot, access_rule: rule(F::ALLOW_WRITE | F::REQUIRE_SM) },
            ]
        );
        assert_eq!(
            policies(&ObjectPolicy::placeholder_slot()),
            vec![Policy { object_id: owner, access_rule: rule(F::ALLOW_DELETE) }]
        );
        assert_eq!(
            policies(&ObjectPolicy::owner_object()),
            vec![Policy { object_id: ObjectId::INVALID, access_rule: rule(F::ALLOW_READ) }]
        );
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
