//! Control messages (channels spec §5): the plaintext of a message of type control, a canonical
//! CBOR map `{0: kind, ...}`.
//!
//! | kind | name | fields |
//! |---|---|---|
//! | 0 | rotation | `1: epoch_id bstr 20, 2: [deposit id bstr 32, ...]` |
//! | 1 | policy change | `1: policy_id bstr 32` |
//! | 2 | revocation | `1: grant_id bstr 16, 2: reason tstr` |
//! | 3 | join | `1: grant_id bstr 16, 2: xwing_pk bstr, 3: name tstr 1 to 64 bytes` |
//! | 4 | leave | none |

use alloc::string::String;
use alloc::vec::Vec;
use minicbor::Decoder;

use crate::cbor::{array_len, canonical, encode_with, Enc, EncResult, Nested};
use crate::keys::EpochId;
use crate::ChannelError;

const M: ChannelError = ChannelError::Malformed;

/// The longest join name, in bytes.
const NAME_MAX: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Control {
    /// A new epoch, and the ids (`SHA-256` of [`crate::deposit::Deposit::encode`]) of the
    /// deposits made for it.
    Rotation {
        epoch_id: EpochId,
        deposits: Vec<[u8; 32]>,
    },
    PolicyChange {
        policy_id: [u8; 32],
    },
    Revocation {
        grant_id: [u8; 16],
        reason: String,
    },
    /// Posted by the issuer when it admits a member, so every member learns the key to seal to.
    Join {
        grant_id: [u8; 16],
        xwing_pk: Vec<u8>,
        name: String,
    },
    Leave,
}

impl Control {
    fn kind(&self) -> u8 {
        match self {
            Control::Rotation { .. } => 0,
            Control::PolicyChange { .. } => 1,
            Control::Revocation { .. } => 2,
            Control::Join { .. } => 3,
            Control::Leave => 4,
        }
    }

    /// The number of map entries a kind has, its own included.
    fn entries(kind: u8) -> Option<u64> {
        match kind {
            0 => Some(3),
            1 => Some(2),
            2 => Some(3),
            3 => Some(4),
            4 => Some(1),
            _ => None,
        }
    }

    /// The canonical CBOR map.
    pub fn encode(&self) -> Vec<u8> {
        encode_with(|e| self.write(e))
    }

    fn write(&self, e: &mut Enc) -> EncResult {
        let kind = self.kind();
        e.map(Control::entries(kind).unwrap_or(1))?;
        e.u8(0)?.u8(kind)?;
        match self {
            Control::Rotation { epoch_id, deposits } => {
                e.u8(1)?.bytes(&epoch_id.encode())?;
                e.u8(2)?.array(deposits.len() as u64)?;
                for d in deposits {
                    e.bytes(d)?;
                }
            }
            Control::PolicyChange { policy_id } => {
                e.u8(1)?.bytes(policy_id)?;
            }
            Control::Revocation { grant_id, reason } => {
                e.u8(1)?.bytes(grant_id)?;
                e.u8(2)?.str(reason)?;
            }
            Control::Join {
                grant_id,
                xwing_pk,
                name,
            } => {
                e.u8(1)?.bytes(grant_id)?;
                e.u8(2)?.bytes(xwing_pk)?;
                e.u8(3)?.str(name)?;
            }
            Control::Leave => {}
        }
        Ok(())
    }

    /// Parse a canonical control map. Refuses an unknown kind, a kind with the wrong fields, a
    /// join name that is empty or longer than 64 bytes, and trailing bytes.
    pub fn decode(bytes: &[u8]) -> Result<Control, ChannelError> {
        let mut d = Decoder::new(bytes);
        let n = match d.clone().map() {
            Ok(Some(n)) => n,
            Ok(None) => return Err(ChannelError::NotCanonical),
            Err(_) => return Err(M),
        };
        let mut r = Nested::open(&mut d, n)?;
        let kind = r.u8()?;
        if Control::entries(kind) != Some(n) {
            return Err(M);
        }
        let control = match kind {
            0 => {
                let epoch_id = EpochId::decode(r.bytes()?).ok_or(M)?;
                let d = r.key()?;
                let count = array_len(d)?;
                let mut deposits = Vec::new();
                for _ in 0..count {
                    deposits.push(d.bytes().map_err(|_| M)?.try_into().map_err(|_| M)?);
                }
                Control::Rotation { epoch_id, deposits }
            }
            1 => Control::PolicyChange {
                policy_id: r.bytes()?.try_into().map_err(|_| M)?,
            },
            2 => Control::Revocation {
                grant_id: r.bytes()?.try_into().map_err(|_| M)?,
                reason: r.string()?,
            },
            3 => {
                let grant_id = r.bytes()?.try_into().map_err(|_| M)?;
                let xwing_pk = r.bytes()?.to_vec();
                let name = r.string()?;
                if name.is_empty() || name.len() > NAME_MAX {
                    return Err(M);
                }
                Control::Join {
                    grant_id,
                    xwing_pk,
                    name,
                }
            }
            _ => Control::Leave,
        };
        r.finish()?;
        if d.position() != bytes.len() {
            return Err(M);
        }
        let encoded = control.encode();
        canonical(control, encoded, bytes)
    }
}
