//! A chip in software, for tests and the simulated badge.
//!
//! Keys live in a map. Attestation is signed by the fixed test CA in [`crate::testca`], so the
//! checks a verifier makes run in `cargo test` without hardware. Not secret: never use it to
//! guard a real key.
//!
//! It models what the real chip enforces: every object carries the policy it was made with,
//! the chip is in no session, the owner's user-id session or the phone slot's session, and a
//! call the policy does not allow the session is `NotAllowed` ([`ObjectPolicy::allows`]). A
//! slot session either did or did not set up secure messaging, and a policy with `require_sm`
//! refuses the slot without it. Creating at an id in use is `Exists`.
//!
//! Through [`ChipOps`] and the `_for_test` calls, SCP03 is modelled as "the host proved the
//! key". Through the APDU front end (`SoftChip::apdu`, in `soft/apdu.rs`, with the `std`
//! feature) a phone's tap runs the real thing: the SCP03 handshake against the slot's AES key
//! and every command in the session under C-MAC, C-ENC, R-MAC and R-ENC.

use crate::ids::KeyId;
use crate::ops::{
    Attested, ChipError, ChipOps, Curve, Identity, ObjectPolicy, Session, Signature64, Version,
};
use crate::testca;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use hmac::{Hmac, Mac};
use p256::ecdsa::signature::hazmat::PrehashSigner;
use p256::ecdsa::signature::Signer;
use rand_core::{CryptoRng, RngCore};
use sha2::{Digest, Sha256};

#[cfg(feature = "std")]
mod apdu;
#[cfg(feature = "std")]
mod scp03;

#[cfg(feature = "std")]
pub use apdu::APPLET_ID;

/// The chip's unique id: 17 bytes, the same on every soft chip.
pub const SOFT_UID: &[u8; 17] = b"softchip-00000000";

enum Object {
    P256(p256::ecdsa::SigningKey),
    Ed25519(ed25519_dalek::SigningKey),
    Hmac([u8; 32]),
    Counter(u64),
    Binary(Vec<u8>),
    UserId([u8; 16]),
    Aes([u8; 16]),
}

struct Entry {
    object: Object,
    policy: ObjectPolicy,
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
    objects: BTreeMap<u32, Entry>,
    session: Session,
    /// With `Some(n)`, the creating call after the next `n` fails.
    fail_create: Option<usize>,
    /// The status word the next failing creating call answers, `6A80` when not set.
    fail_status: Option<u16>,
    /// This many creating calls in a row fail, from the next.
    fail_streak: usize,
    /// The power is cut, as a panic, right after this object is deleted.
    cut_after_delete: Option<KeyId>,
    /// The next read of this binary object fails on the bus.
    fail_read: Option<KeyId>,
    rng: SoftRng,
    clock: u64,
    ca: p256::ecdsa::SigningKey,
    /// The applet was selected over the APDU front end (`SoftChip::apdu`).
    #[cfg(feature = "std")]
    selected: bool,
    /// The session a phone opened over the APDU front end, if any.
    #[cfg(feature = "std")]
    nfc: Option<apdu::NfcSession>,
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
            session: Session::None,
            fail_create: None,
            fail_status: None,
            fail_streak: 0,
            cut_after_delete: None,
            fail_read: None,
            rng: SoftRng { seed, counter: 0 },
            clock: 0,
            ca: p256::ecdsa::SigningKey::from_slice(&ca).expect("the test CA key is valid"),
            #[cfg(feature = "std")]
            selected: false,
            #[cfg(feature = "std")]
            nfc: None,
        }
    }
}

/// The first bytes of [`SoftChip::export`]'s output.
const EXPORT_MAGIC: &[u8; 9] = b"SOFTCHIP2";
/// The format before objects carried policies: imported with [`ObjectPolicy::any_session`],
/// the policy the badge gave every object then.
const EXPORT_MAGIC_V1: &[u8; 9] = b"SOFTCHIP1";

/// A policy as six bytes: for sign, read, write, delete and attest a bit mask of the
/// identities (1 no session, 2 owner, 4 slot), then `require_sm`.
fn policy_bytes(p: &ObjectPolicy) -> [u8; 6] {
    let mask = |ids: &[Identity]| {
        ids.iter().fold(0u8, |m, i| {
            m | match i {
                Identity::None => 1,
                Identity::Owner => 2,
                Identity::Slot => 4,
            }
        })
    };
    [mask(&p.sign), mask(&p.read), mask(&p.write), mask(&p.delete), mask(&p.attest), p.require_sm as u8]
}

fn policy_from(b: &[u8]) -> Option<ObjectPolicy> {
    let ids = |m: u8| -> Option<Vec<Identity>> {
        if m & !7 != 0 {
            return None;
        }
        let mut v = Vec::new();
        for (bit, i) in [(1, Identity::None), (2, Identity::Owner), (4, Identity::Slot)] {
            if m & bit != 0 {
                v.push(i);
            }
        }
        Some(v)
    };
    if b.len() != 6 || b[5] > 1 {
        return None;
    }
    Some(ObjectPolicy {
        sign: ids(b[0])?,
        read: ids(b[1])?,
        write: ids(b[2])?,
        delete: ids(b[3])?,
        attest: ids(b[4])?,
        require_sm: b[5] == 1,
    })
}

impl SoftChip {
    /// Everything the chip holds, so a simulated badge can keep its chip across restarts: the
    /// generator's seed and position, the clock, and every object with its secret and policy.
    /// The session is not kept: a chip starts in no session, as the real one does at power-on.
    ///
    /// `SOFTCHIP2 | seed 32 | counter u64 | clock u64 | count u32 | (id u32 | tag u8 | value |
    /// policy 6)*`, big-endian, objects in id order; the tag is 1 for P-256 (a 32-byte
    /// scalar), 2 for Ed25519 (a 32-byte seed), 3 for an HMAC key (32 bytes), 4 for a counter
    /// (u64), 5 for a binary object (u16 length and the bytes), 6 for a user id (16 bytes), 7
    /// for an AES key (16 bytes). The policy is six bytes, see `policy_bytes`.
    ///
    /// The output holds secret keys in the clear, as a software chip does in memory.
    pub fn export(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(EXPORT_MAGIC);
        out.extend_from_slice(&self.rng.seed);
        out.extend_from_slice(&self.rng.counter.to_be_bytes());
        out.extend_from_slice(&self.clock.to_be_bytes());
        out.extend_from_slice(&(self.objects.len() as u32).to_be_bytes());
        for (id, e) in &self.objects {
            out.extend_from_slice(&id.to_be_bytes());
            match &e.object {
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
                Object::Binary(b) => {
                    out.push(5);
                    out.extend_from_slice(&(b.len() as u16).to_be_bytes());
                    out.extend_from_slice(b);
                }
                Object::UserId(k) => {
                    out.push(6);
                    out.extend_from_slice(k);
                }
                Object::Aes(k) => {
                    out.push(7);
                    out.extend_from_slice(k);
                }
            }
            out.extend_from_slice(&policy_bytes(&e.policy));
        }
        out
    }

    /// A chip from [`SoftChip::export`]'s output, or `None` if the bytes are not exactly that.
    /// The older `SOFTCHIP1` format is read too, each object given the policy objects had
    /// then ([`ObjectPolicy::any_session`]).
    pub fn import(bytes: &[u8]) -> Option<SoftChip> {
        let (mut r, v1) = match bytes.strip_prefix(EXPORT_MAGIC.as_slice()) {
            Some(r) => (r, false),
            None => (bytes.strip_prefix(EXPORT_MAGIC_V1.as_slice())?, true),
        };
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
                5 if !v1 => {
                    let n = u16::from_be_bytes(take(2)?.try_into().ok()?) as usize;
                    Object::Binary(take(n)?.to_vec())
                }
                6 if !v1 => Object::UserId(take(16)?.try_into().ok()?),
                7 if !v1 => Object::Aes(take(16)?.try_into().ok()?),
                _ => return None,
            };
            let policy = if v1 { ObjectPolicy::any_session() } else { policy_from(take(6)?)? };
            if chip.objects.insert(id, Entry { object, policy }).is_some() {
                return None;
            }
        }
        if !r.is_empty() {
            return None;
        }
        Some(chip)
    }

    /// The session the chip is in.
    pub fn session(&self) -> Session {
        self.session
    }

    /// Open the phone slot's session as a phone that proved the slot's key and set up SCP03
    /// secure messaging. `NoSuchKey` if there is no slot object.
    pub fn open_slot_with_sm_for_test(&mut self) -> Result<(), ChipError> {
        self.open_slot(true)
    }

    /// Open the phone slot's session without secure messaging: the session a policy with
    /// `require_sm` refuses.
    pub fn open_slot_without_sm_for_test(&mut self) -> Result<(), ChipError> {
        self.open_slot(false)
    }

    /// Open the phone slot's session as a phone holding `key` would, with SCP03 secure
    /// messaging: the session opens only if `key` is the slot's AES key. `BadSession` for any
    /// other key, and the chip is left in no session; `NoSuchKey` if there is no slot object.
    pub fn open_slot_with_key_for_test(&mut self, key: &[u8; 16]) -> Result<(), ChipError> {
        self.session = Session::None;
        match self.objects.get(&KeyId::SLOT.0) {
            Some(Entry { object: Object::Aes(k), .. }) if k == key => {
                self.session = Session::Slot { scp03: true };
                Ok(())
            }
            Some(Entry { object: Object::Aes(_), .. }) => Err(ChipError::BadSession),
            _ => Err(ChipError::NoSuchKey),
        }
    }

    /// The next creating call fails with `6A80`, as a write the applet refuses does.
    pub fn fail_next_create_for_test(&mut self) {
        self.fail_create = Some(0);
    }

    /// The next creating call fails with `sw`: `0x6A84` is the applet's answer when its
    /// memory is full (bench, SE051 applet 7.2.0).
    pub fn fail_next_create_with(&mut self, sw: u16) {
        self.fail_create = Some(0);
        self.fail_status = Some(sw);
    }

    /// The next `n` creating calls fail with `6A80`.
    pub fn fail_next_creates_for_test(&mut self, n: usize) {
        self.fail_streak = n;
    }

    /// The creating call after the next `n` fails with `6A80`.
    pub fn fail_create_after_for_test(&mut self, n: usize) {
        self.fail_create = Some(n);
    }

    /// Cut the power right after `id` is next deleted: the call panics, and the test catches
    /// it and turns the badge on again. What the chip and the board stored by then stays.
    pub fn cut_after_delete_for_test(&mut self, id: KeyId) {
        self.cut_after_delete = Some(id);
    }

    /// The next read of binary object `id` fails on the bus (`Transport`).
    pub fn fail_next_read_for_test(&mut self, id: KeyId) {
        self.fail_read = Some(id);
    }

    /// Back to no session, as a tap by anything that holds no key would be.
    pub fn no_session_for_test(&mut self) {
        self.session = Session::None;
    }

    fn open_slot(&mut self, scp03: bool) -> Result<(), ChipError> {
        match self.objects.get(&KeyId::SLOT.0) {
            Some(Entry { object: Object::Aes(_), .. }) => {
                self.session = Session::Slot { scp03 };
                Ok(())
            }
            _ => Err(ChipError::NoSuchKey),
        }
    }

    /// Make an object at `id`, which must be free.
    fn create(&mut self, id: KeyId, object: Object, policy: &ObjectPolicy) -> Result<(), ChipError> {
        if self.objects.contains_key(&id.0) {
            return Err(ChipError::Exists);
        }
        if self.fail_streak > 0 {
            self.fail_streak -= 1;
            return Err(ChipError::Status(0x6A80));
        }
        match self.fail_create {
            Some(0) => {
                self.fail_create = None;
                return Err(ChipError::Status(self.fail_status.take().unwrap_or(0x6A80)));
            }
            Some(n) => self.fail_create = Some(n - 1),
            None => {}
        }
        self.objects.insert(id.0, Entry { object, policy: policy.clone() });
        Ok(())
    }

    /// The object at `id`, if the session may do what `rights` picks from its policy.
    fn get(
        &mut self,
        id: KeyId,
        rights: fn(&ObjectPolicy) -> &[Identity],
    ) -> Result<&mut Object, ChipError> {
        let session = self.session;
        let e = self.objects.get_mut(&id.0).ok_or(ChipError::NoSuchKey)?;
        if !e.policy.allows(rights(&e.policy), session) {
            return Err(ChipError::NotAllowed);
        }
        Ok(&mut e.object)
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
        _ => Err(ChipError::NoSuchKey),
    }
}

fn sign_rights(p: &ObjectPolicy) -> &[Identity] {
    &p.sign
}
fn read_rights(p: &ObjectPolicy) -> &[Identity] {
    &p.read
}
fn write_rights(p: &ObjectPolicy) -> &[Identity] {
    &p.write
}
fn attest_rights(p: &ObjectPolicy) -> &[Identity] {
    &p.attest
}

impl ChipOps for SoftChip {
    fn open_owner(&mut self, secret: &[u8; 16]) -> Result<(), ChipError> {
        self.session = Session::None;
        // The badge takes the chip over I2C: a phone's session over NFC is gone.
        #[cfg(feature = "std")]
        {
            self.nfc = None;
        }
        match self.objects.get(&KeyId::OWNER.0) {
            Some(Entry { object: Object::UserId(k), .. }) if k == secret => {
                self.session = Session::Owner;
                Ok(())
            }
            Some(_) => Err(ChipError::BadSession),
            None => Err(ChipError::NoSuchKey),
        }
    }

    fn close_session(&mut self) -> Result<(), ChipError> {
        self.session = Session::None;
        Ok(())
    }

    fn version(&mut self) -> Result<Version, ChipError> {
        Ok(Version { major: 7, minor: 2, patch: 0 })
    }

    fn random(&mut self, n: usize) -> Result<Vec<u8>, ChipError> {
        let mut v = alloc::vec![0u8; n];
        self.rng.fill_bytes(&mut v);
        Ok(v)
    }

    fn generate(&mut self, id: KeyId, curve: Curve, policy: &ObjectPolicy) -> Result<Vec<u8>, ChipError> {
        if self.objects.contains_key(&id.0) {
            return Err(ChipError::Exists);
        }
        let object = match curve {
            Curve::P256 => Object::P256(p256::ecdsa::SigningKey::random(&mut self.rng)),
            Curve::Ed25519 => Object::Ed25519(ed25519_dalek::SigningKey::generate(&mut self.rng)),
        };
        let public = public_of(&object)?;
        self.create(id, object, policy)?;
        Ok(public)
    }

    fn public(&mut self, id: KeyId) -> Result<Vec<u8>, ChipError> {
        public_of(self.get(id, read_rights)?)
    }

    fn delete(&mut self, id: KeyId) -> Result<(), ChipError> {
        let session = self.session;
        let e = self.objects.get(&id.0).ok_or(ChipError::NoSuchKey)?;
        if !e.policy.allows(&e.policy.delete, session) {
            return Err(ChipError::NotAllowed);
        }
        // The applet deletes an authentication object even through the session it opened, if
        // the policy allows it (bench, 2026-10-31); that session is then dead.
        let own = match session {
            Session::Owner => id == KeyId::OWNER,
            Session::Slot { .. } => id == KeyId::SLOT,
            Session::None => false,
        };
        self.objects.remove(&id.0);
        if own {
            self.session = Session::None;
        }
        if self.cut_after_delete == Some(id) {
            self.cut_after_delete = None;
            panic!("the power was cut after deleting {:08x}", id.0);
        }
        Ok(())
    }

    fn exists(&mut self, id: KeyId) -> Result<bool, ChipError> {
        Ok(self.objects.contains_key(&id.0))
    }

    fn ids(&mut self) -> Result<Vec<KeyId>, ChipError> {
        Ok(self.objects.keys().map(|&k| KeyId(k)).filter(|k| k.in_range()).collect())
    }

    fn sign_p256(&mut self, id: KeyId, digest: &[u8; 32]) -> Result<Signature64, ChipError> {
        match self.get(id, sign_rights)? {
            Object::P256(k) => {
                let sig: p256::ecdsa::Signature =
                    k.sign_prehash(digest).map_err(|_| ChipError::Status(0x6A80))?;
                Ok(sig.to_bytes().into())
            }
            _ => Err(ChipError::NoSuchKey),
        }
    }

    fn sign_ed25519(&mut self, id: KeyId, message: &[u8]) -> Result<Signature64, ChipError> {
        match self.get(id, sign_rights)? {
            Object::Ed25519(k) => Ok(k.sign(message).to_bytes()),
            _ => Err(ChipError::NoSuchKey),
        }
    }

    fn attest(&mut self, id: KeyId, freshness: &[u8; 16]) -> Result<Attested, ChipError> {
        let public = public_of(self.get(id, attest_rights)?)?;
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

    fn put_hmac_key(&mut self, id: KeyId, key: &[u8; 32], policy: &ObjectPolicy) -> Result<(), ChipError> {
        self.create(id, Object::Hmac(*key), policy)
    }

    fn hmac(&mut self, id: KeyId, data: &[u8]) -> Result<[u8; 32], ChipError> {
        match self.get(id, sign_rights)? {
            Object::Hmac(k) => {
                let mut m = <Hmac<Sha256> as Mac>::new_from_slice(k).map_err(|_| ChipError::Status(0x6A80))?;
                m.update(data);
                Ok(m.finalize().into_bytes().into())
            }
            _ => Err(ChipError::NoSuchKey),
        }
    }

    fn put_counter(&mut self, id: KeyId, value: u64, policy: &ObjectPolicy) -> Result<(), ChipError> {
        // The applet refuses a counter whose policy gives sign or attest rights.
        if !policy.sign.is_empty() || !policy.attest.is_empty() {
            return Err(ChipError::Status(0x6A80));
        }
        self.create(id, Object::Counter(value), policy)
    }

    fn counter_next(&mut self, id: KeyId) -> Result<u64, ChipError> {
        match self.get(id, write_rights)? {
            Object::Counter(c) => {
                *c = c.checked_add(1).ok_or(ChipError::Status(0x6985))?;
                Ok(*c)
            }
            _ => Err(ChipError::NoSuchKey),
        }
    }

    fn counter_get(&mut self, id: KeyId) -> Result<u64, ChipError> {
        match self.get(id, read_rights)? {
            Object::Counter(c) => Ok(*c),
            _ => Err(ChipError::NoSuchKey),
        }
    }

    fn put_user_id(&mut self, id: KeyId, secret: &[u8; 16], policy: &ObjectPolicy) -> Result<(), ChipError> {
        self.create(id, Object::UserId(*secret), policy)
    }

    fn put_aes_auth(&mut self, id: KeyId, key: &[u8; 16], policy: &ObjectPolicy) -> Result<(), ChipError> {
        self.create(id, Object::Aes(*key), policy)
    }

    fn write_binary(&mut self, id: KeyId, data: &[u8], policy: &ObjectPolicy) -> Result<(), ChipError> {
        if data.is_empty() || data.len() > u16::MAX as usize {
            return Err(ChipError::Status(0x6A80));
        }
        self.create(id, Object::Binary(data.to_vec()), policy)
    }

    fn rewrite_binary(&mut self, id: KeyId, data: &[u8]) -> Result<(), ChipError> {
        match self.get(id, write_rights)? {
            Object::Binary(b) if data.len() <= b.len() => {
                b[..data.len()].copy_from_slice(data);
                Ok(())
            }
            Object::Binary(_) => Err(ChipError::Status(0x6A80)),
            _ => Err(ChipError::NoSuchKey),
        }
    }

    fn read_binary(&mut self, id: KeyId) -> Result<Vec<u8>, ChipError> {
        if self.fail_read == Some(id) {
            self.fail_read = None;
            return Err(ChipError::Transport);
        }
        match self.get(id, read_rights)? {
            Object::Binary(b) => Ok(b.clone()),
            _ => Err(ChipError::NoSuchKey),
        }
    }
}
