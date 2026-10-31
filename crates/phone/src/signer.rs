//! The `Signer` callback (Swift's Secure Enclave) and a software signer for tests.
//!
//! The ceremony signs through `cahoots_core::seat::SeatSigner`, which cannot fail. `Seat`
//! adapts a fallible callback to it: a refused signature is kept as the error and the
//! ceremony gets a signature that does not verify, which it refuses without changing anything.

use crate::PhoneError;
use cahoots_core::seat::{SeatKey, SeatPublic, SeatSigner, Signature};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Keys that never leave the phone. Swift makes them in the Secure Enclave with user-presence
/// access control, so a call may ask for Face ID or the passcode, and fails with
/// `PhoneError::Locked` when the person says no or the phone is locked.
#[uniffi::export(callback_interface)]
pub trait Signer: Send + Sync {
    /// Make the P-256 key named `label`, or return the one already made under it: the SEC1
    /// compressed public key, 33 bytes.
    fn make_key(&self, label: String) -> Result<Vec<u8>, PhoneError>;
    /// ECDSA P-256 over SHA-256 of `message` with the key named `label`: raw `r || s`, 64 bytes.
    fn sign(&self, label: String, message: Vec<u8>) -> Result<Vec<u8>, PhoneError>;
}

/// A `SeatSigner` over the callback for one key.
pub(crate) struct Seat<'a> {
    pub signer: &'a dyn Signer,
    pub label: String,
    pub public: SeatPublic,
    /// The first error the callback returned, if any.
    pub error: Option<PhoneError>,
}

impl SeatSigner for Seat<'_> {
    fn public(&mut self) -> SeatPublic {
        self.public
    }
    fn sign(&mut self, msg: &[u8]) -> Signature {
        match self.signer.sign(self.label.clone(), msg.to_vec()) {
            Ok(sig) if sig.len() == 64 => sig.try_into().expect("64 bytes"),
            Ok(_) => {
                self.error = Some(PhoneError::Signer {
                    message: "The key store returned a signature of the wrong length.".into(),
                });
                [0; 64]
            }
            Err(e) => {
                self.error = Some(e);
                [0; 64]
            }
        }
    }
}

/// A software stand-in for the Secure Enclave: deterministic ECDSA (RFC 6979) with keys in
/// memory. It records every `sign` call's label and can be locked. For tests only.
#[derive(Clone, Default)]
pub struct SoftSigner {
    inner: Arc<Mutex<SoftInner>>,
}

#[derive(Default)]
struct SoftInner {
    keys: HashMap<String, SeatKey>,
    signs: Vec<String>,
    locked: bool,
}

impl SoftSigner {
    pub fn new() -> Self {
        Self::default()
    }

    /// While locked, `sign` returns `PhoneError::Locked` and records nothing.
    pub fn set_locked(&self, locked: bool) {
        self.inner.lock().unwrap().locked = locked;
    }

    /// The label of every signature made, in order.
    pub fn sign_calls(&self) -> Vec<String> {
        self.inner.lock().unwrap().signs.clone()
    }

    /// The public key made under `label`, if any.
    pub fn public(&self, label: &str) -> Option<Vec<u8>> {
        let inner = self.inner.lock().unwrap();
        inner.keys.get(label).map(|k| k.public().0.to_vec())
    }
}

impl Signer for SoftSigner {
    fn make_key(&self, label: String) -> Result<Vec<u8>, PhoneError> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(k) = inner.keys.get(&label) {
            return Ok(k.public().0.to_vec());
        }
        let key = loop {
            let mut seed = zeroize::Zeroizing::new([0u8; 32]);
            getrandom::getrandom(seed.as_mut()).map_err(|e| PhoneError::Signer {
                message: e.to_string(),
            })?;
            // A seed that is not a valid scalar is drawn again.
            if let Ok(k) = SeatKey::from_seed(&seed) {
                break k;
            }
        };
        let public = key.public().0.to_vec();
        inner.keys.insert(label, key);
        Ok(public)
    }

    fn sign(&self, label: String, message: Vec<u8>) -> Result<Vec<u8>, PhoneError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.locked {
            return Err(PhoneError::Locked);
        }
        let sig = inner
            .keys
            .get(&label)
            .ok_or_else(|| PhoneError::Signer {
                message: format!("No key is named {label}."),
            })?
            .sign(&message);
        inner.signs.push(label);
        Ok(sig.to_vec())
    }
}
