//! `serve`: the one function a transport calls.
//!
//! It decodes a request, asks a [`Consent`] for permission if the request spends pad,
//! applies it to a [`Custodian`], and encodes the response. `Identify` and `Remaining`
//! never ask. The four spending requests always do, and a refusal spends nothing.
//!
//! `Identify` answers with the custodian's circles, the first circle's tier (0 when there
//! are none) and an empty name. A device that has a name wraps the response and fills it in.

use alloc::string::String;
use alloc::vec::Vec;

use crate::wire::{ErrorCode, Request, Response};
use crate::Custodian;

/// Permission to spend. The device decides how to ask: a button, a prompt, or nothing.
pub trait Consent {
    /// Return true to let a spending request go ahead.
    fn allow(&mut self, req: &Request) -> bool;
}

/// Consent that always says yes. For tests and for hosts that ask elsewhere.
pub struct AlwaysAllow;

impl Consent for AlwaysAllow {
    fn allow(&mut self, _: &Request) -> bool {
        true
    }
}

fn err(code: ErrorCode) -> Response {
    Response::Error { code }
}

/// Decode `request`, apply it to `c` and return the encoded response.
pub fn serve<C: Custodian, K: Consent>(c: &mut C, consent: &mut K, request: &[u8]) -> Vec<u8> {
    let req = match Request::decode(request) {
        Ok(r) => r,
        Err(code) => return err(code).encode(),
    };
    let spends = !matches!(req, Request::Identify | Request::Remaining { .. });
    if spends && !consent.allow(&req) {
        return err(ErrorCode::Denied).encode();
    }
    let resp = match req {
        Request::Identify => {
            let circles = c.circles();
            let tier = circles.first().map_or(0, |i| i.tier);
            Response::Identity {
                circles,
                tier,
                name: String::new(),
            }
        }
        Request::Psk { circle, context } => match c.psk(&circle, &context) {
            Ok((handle, key)) => Response::Psk { handle, key: *key },
            Err(e) => err(e.into()),
        },
        Request::PskFor { handle, context } => match c.psk_for(&handle, &context) {
            Ok(key) => Response::Key { key: *key },
            Err(e) => err(e.into()),
        },
        Request::Spend { circle, len } => match c.spend(&circle, len) {
            Ok((handle, bytes)) => Response::Bytes {
                handle,
                bytes: bytes.to_vec(),
            },
            Err(e) => err(e.into()),
        },
        Request::SpendFor { handle, len } => match c.spend_for(&handle, len) {
            Ok(bytes) => Response::Bytes {
                handle,
                bytes: bytes.to_vec(),
            },
            Err(e) => err(e.into()),
        },
        Request::Remaining { circle } => Response::Remaining {
            bytes: c.remaining(&circle),
            low: c.low_pad(&circle),
        },
    };
    resp.encode()
}
