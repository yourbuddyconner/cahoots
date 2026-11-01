//! The chip's operations and the values they return.

use crate::ids::KeyId;
use alloc::vec;
use alloc::vec::Vec;
use hmac::{Hmac, Mac};
use sha2::Sha256;

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
    /// The object's policy does not allow this in the open session (status word `6986`, the
    /// applet's answer on the bench, or `6982`).
    NotAllowed,
    /// An object already has this id (status word `6A89`). Creating calls never replace.
    Exists,
    /// A session could not be opened: the secret or key did not match.
    BadSession,
}

/// Who may touch an object: the session a command runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identity {
    /// No session: any command sent to the chip as it is, by the badge before its owner
    /// session opens, or by anything that taps the chip's antenna.
    None,
    /// The badge's owner session, opened with its user id ([`KeyId::OWNER`]).
    Owner,
    /// The phone slot's session ([`KeyId::SLOT`]), an SCP03 session over NFC.
    Slot,
}

impl Identity {
    /// The authentication object a policy names for this identity; `None` for no session.
    pub const fn auth_id(self) -> Option<KeyId> {
        match self {
            Identity::None => None,
            Identity::Owner => Some(KeyId::OWNER),
            Identity::Slot => Some(KeyId::SLOT),
        }
    }
}

/// The session the chip is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {
    None,
    Owner,
    /// The phone slot's session. `scp03` marks a session that set up SCP03 secure messaging,
    /// which a policy's [`ObjectPolicy::require_sm`] asks of the slot.
    Slot { scp03: bool },
}

impl Session {
    pub const fn identity(self) -> Identity {
        match self {
            Session::None => Identity::None,
            Session::Owner => Identity::Owner,
            Session::Slot { .. } => Identity::Slot,
        }
    }
}

/// An object's policy, fixed when the object is made: for each operation, the identities that
/// may ask for it.
///
/// The applet keeps one entry per identity the policy names anywhere, and `None` (no session,
/// authentication object id 0) is the entry for every session the policy does not name: a
/// session the policy names gets its own entry's rights and nothing from the `None` entry,
/// and a session it does not name gets the `None` entry's (bench, applet 7.2.0, 2026-10-31).
/// A session named nowhere, with no `None` entry, may do nothing with the object.
///
/// `require_sm` applies to the slot's own entry: when set, the slot's session must run SCP03
/// secure messaging for any of the slot's rights.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectPolicy {
    /// Sign with a key, or MAC with an HMAC key.
    pub sign: Vec<Identity>,
    /// Read a public key, a counter or a binary object.
    pub read: Vec<Identity>,
    /// Write a binary object or move a counter.
    pub write: Vec<Identity>,
    pub delete: Vec<Identity>,
    /// An attested read of the public key.
    pub attest: Vec<Identity>,
    pub require_sm: bool,
}

impl ObjectPolicy {
    /// A seat key: sign, read its public key, attest and delete for the owner, nothing for
    /// anyone else.
    pub fn owner_only() -> ObjectPolicy {
        ObjectPolicy {
            sign: vec![Identity::Owner],
            read: vec![Identity::Owner],
            write: Vec::new(),
            delete: vec![Identity::Owner],
            attest: vec![Identity::Owner],
            require_sm: false,
        }
    }

    /// A counter: read, move and delete for the owner, nothing for anyone else. The applet
    /// refuses a counter whose policy carries sign or attest rights (6A80, bench 2026-10-31).
    pub fn owner_counter() -> ObjectPolicy {
        ObjectPolicy {
            sign: Vec::new(),
            read: vec![Identity::Owner],
            write: vec![Identity::Owner],
            delete: vec![Identity::Owner],
            attest: Vec::new(),
            require_sm: false,
        }
    }

    /// A root or a hello object: MAC for the owner and for the slot under secure messaging,
    /// delete for the owner.
    pub fn owner_and_slot_mac() -> ObjectPolicy {
        ObjectPolicy {
            sign: vec![Identity::Owner, Identity::Slot],
            read: Vec::new(),
            write: Vec::new(),
            delete: vec![Identity::Owner],
            attest: Vec::new(),
            require_sm: true,
        }
    }

    /// A circle's public object: read by the owner and by the slot under secure messaging,
    /// written and deleted by the owner. No session may read it, so a stranger's tap learns
    /// nothing of a circle's manifest, card or paired phone.
    pub fn public_object() -> ObjectPolicy {
        ObjectPolicy {
            sign: Vec::new(),
            read: vec![Identity::Owner, Identity::Slot],
            write: vec![Identity::Owner],
            delete: vec![Identity::Owner],
            attest: Vec::new(),
            require_sm: true,
        }
    }

    /// The pairing object: read by any session, written and deleted by the owner. A phone
    /// reads it before it holds a key, and learns from it which circle slots the badge vouches
    /// for.
    pub fn pairing_object() -> ObjectPolicy {
        ObjectPolicy {
            sign: Vec::new(),
            read: vec![Identity::None, Identity::Owner, Identity::Slot],
            write: vec![Identity::Owner],
            delete: vec![Identity::Owner],
            attest: Vec::new(),
            require_sm: true,
        }
    }

    /// The inbox: written by any session, read, written and deleted by the owner.
    pub fn inbox() -> ObjectPolicy {
        ObjectPolicy {
            sign: Vec::new(),
            read: vec![Identity::Owner],
            write: vec![Identity::None, Identity::Owner, Identity::Slot],
            delete: vec![Identity::Owner],
            attest: Vec::new(),
            require_sm: true,
        }
    }

    /// The phone slot while no phone is paired: deleted by the owner, nothing else.
    pub fn placeholder_slot() -> ObjectPolicy {
        ObjectPolicy {
            sign: Vec::new(),
            read: Vec::new(),
            write: Vec::new(),
            delete: vec![Identity::Owner],
            attest: Vec::new(),
            require_sm: false,
        }
    }

    /// The owner object itself: its attributes read by any session, nothing else for anyone.
    /// Nobody may delete it, so a tap cannot remove it and make one of its own at its id. The
    /// applet does delete a user id through its own session when the policy allows that, so
    /// the owner session must not be named with a right to delete either.
    pub fn owner_object() -> ObjectPolicy {
        ObjectPolicy {
            sign: Vec::new(),
            read: vec![Identity::None],
            write: Vec::new(),
            delete: Vec::new(),
            attest: Vec::new(),
            require_sm: false,
        }
    }

    /// The policy every object made before the badge owned its chip carried: every right for
    /// every session. Only for reading old chips and for tests of the upgrade.
    pub fn any_session() -> ObjectPolicy {
        let all = vec![Identity::None, Identity::Owner, Identity::Slot];
        ObjectPolicy {
            sign: all.clone(),
            read: all.clone(),
            write: all.clone(),
            delete: all.clone(),
            attest: all,
            require_sm: false,
        }
    }

    /// True when the policy names `who` for any right: the applet keeps an entry for it.
    pub fn names(&self, who: Identity) -> bool {
        [&self.sign, &self.read, &self.write, &self.delete, &self.attest]
            .iter()
            .any(|ids| ids.contains(&who))
    }

    /// True when `session` may do what `rights` lists, as the applet decides it: by the
    /// session's own entry if the policy names it, by the `None` entry if not, and a slot
    /// named under `require_sm` must have set up secure messaging.
    pub fn allows(&self, rights: &[Identity], session: Session) -> bool {
        let who = session.identity();
        if !self.names(who) {
            return rights.contains(&Identity::None);
        }
        if !rights.contains(&who) {
            return false;
        }
        match session {
            Session::Slot { scp03 } => scp03 || !self.require_sm,
            _ => true,
        }
    }
}

/// The badge's owner secret from the board's identity:
/// `HKDF-SHA256(ikm = board_id, salt = "cahoots/owner/v1", info = empty)`, 16 bytes. On the
/// RP2350 `board_id` is the device id and the wafer id the boot ROM reports, each 4 bytes
/// little-endian, then the 128-bit RANDID from OTP, the factory's random number that the boot
/// ROM's `get_sys_info` and PICOBOOT's GET_INFO do not report. A reflash recovers it, so
/// nothing has to be stored.
///
/// The risk: anyone who learns the board id opens the owner session over NFC for good. Over
/// USB, BOOTSEL's PICOBOOT interface reads the whole of OTP page 0, RANDID included, and a USB
/// host can reboot the badge into BOOTSEL. So a badge that has been plugged into a USB host
/// that is not trusted is retired, as one whose board id has leaked is.
pub fn owner_secret(board_id: &[u8]) -> [u8; 16] {
    let mut extract = <Hmac<Sha256> as Mac>::new_from_slice(b"cahoots/owner/v1")
        .expect("HMAC takes a key of any length");
    extract.update(board_id);
    let prk = extract.finalize().into_bytes();
    let mut expand =
        <Hmac<Sha256> as Mac>::new_from_slice(&prk).expect("HMAC takes a key of any length");
    expand.update(&[1]);
    let okm = expand.finalize().into_bytes();
    let mut out = [0u8; 16];
    out.copy_from_slice(&okm[..16]);
    out
}

/// The owner secret of the software badges: the host's simulator and the tests. Not secret.
pub const TEST_OWNER_SECRET: [u8; 16] = *b"cahoots-test-own";

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
///
/// Every call runs in the session the chip is in ([`ChipOps::open_owner`] opens the badge's,
/// [`ChipOps::close_session`] ends it) and is checked against the object's policy: a call the
/// policy does not allow is `NotAllowed`. A creating call takes the new object's policy and
/// never replaces an object: an id in use is `Exists`.
pub trait ChipOps {
    /// Open the owner session with the owner object's secret. `NoSuchKey` if the chip has no
    /// owner object, `BadSession` if the secret is wrong.
    fn open_owner(&mut self, secret: &[u8; 16]) -> Result<(), ChipError>;
    /// End the open session, if any.
    fn close_session(&mut self) -> Result<(), ChipError>;
    fn version(&mut self) -> Result<Version, ChipError>;
    /// `n` random bytes from the chip.
    fn random(&mut self, n: usize) -> Result<Vec<u8>, ChipError>;
    /// Makes a key and returns its public key (SEC1 compressed for P-256).
    fn generate(&mut self, id: KeyId, curve: Curve, policy: &ObjectPolicy) -> Result<Vec<u8>, ChipError>;
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
    fn put_hmac_key(&mut self, id: KeyId, key: &[u8; 32], policy: &ObjectPolicy) -> Result<(), ChipError>;
    fn hmac(&mut self, id: KeyId, data: &[u8]) -> Result<[u8; 32], ChipError>;
    fn put_counter(&mut self, id: KeyId, value: u64, policy: &ObjectPolicy) -> Result<(), ChipError>;
    /// Adds one and returns the new value.
    fn counter_next(&mut self, id: KeyId) -> Result<u64, ChipError>;
    /// Reads a counter without moving it. `NoSuchKey` if there is none.
    fn counter_get(&mut self, id: KeyId) -> Result<u64, ChipError>;
    /// Make a user-id authentication object holding `secret`. No limit on attempts is set.
    fn put_user_id(&mut self, id: KeyId, secret: &[u8; 16], policy: &ObjectPolicy) -> Result<(), ChipError>;
    /// Make an AES-128 authentication object: a session opened against it runs SCP03 with
    /// `key` as its static keys.
    fn put_aes_auth(&mut self, id: KeyId, key: &[u8; 16], policy: &ObjectPolicy) -> Result<(), ChipError>;
    /// Make a binary object holding `data`, its length fixed at `data.len()`.
    fn write_binary(&mut self, id: KeyId, data: &[u8], policy: &ObjectPolicy) -> Result<(), ChipError>;
    /// Write `data` over an existing binary object from its start. Data longer than the
    /// object is refused; bytes past `data.len()` are left as they were.
    fn rewrite_binary(&mut self, id: KeyId, data: &[u8]) -> Result<(), ChipError>;
    /// A binary object's whole contents.
    fn read_binary(&mut self, id: KeyId) -> Result<Vec<u8>, ChipError>;
}
