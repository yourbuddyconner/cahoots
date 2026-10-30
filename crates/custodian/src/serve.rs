//! `serve`: the one function a transport calls.
//!
//! It decodes a request, asks a [`Consent`] for permission if the request spends pad,
//! applies it to a [`Custodian`], and encodes the response. `Identify` and `Remaining`
//! never ask. The four spending requests always do, and a refusal spends nothing. Of the
//! `Chip` family, `Describe`, `Attest`, `Nonce` and `SshSignUnderGrant` never ask (public
//! material, or the badge decides for itself); the other signing and key-releasing
//! operations always do.
//!
//! `Identify` answers with the custodian's circles, the first circle's tier (0 when there
//! are none), an empty name and whether it has a chip. A device that has a name wraps the
//! response and fills it in.

use alloc::string::String;
use alloc::vec::Vec;

use crate::wire::{
    ChipRequest, ChipResponse, ErrorCode, GrantRequestFields, Request, Response, UnderGrantFields,
};
use crate::{ChipApps, Custodian};

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
pub fn serve<C: Custodian + ChipApps, K: Consent>(
    c: &mut C,
    consent: &mut K,
    request: &[u8],
) -> Vec<u8> {
    let req = match Request::decode(request) {
        Ok(r) => r,
        Err(code) => return err(code).encode(),
    };
    // Only a request to sign for ssh may be long; it can carry the text of a commit.
    if !req.length_ok(request.len()) {
        return err(ErrorCode::Malformed).encode();
    }
    let spends = match &req {
        Request::Identify | Request::Remaining { .. } => false,
        Request::Chip { op, .. } => op.needs_consent(),
        _ => true,
    };
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
                chip: c.has_chip(),
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
        Request::Chip { circle, op } => {
            let result = match op {
                ChipRequest::Describe => c.describe(&circle).map(|i| ChipResponse::Description {
                    public: i.public,
                    curve: i.curve,
                    in_chip: i.in_chip,
                    can_attest: i.can_attest,
                }),
                ChipRequest::Attest { freshness } => c
                    .attest(&circle, &freshness)
                    .map(|attested| ChipResponse::Attested { attested }),
                ChipRequest::SshSign {
                    data,
                    host,
                    content,
                } => c
                    .ssh_sign(&circle, &data, host.as_ref(), content.as_deref())
                    .map(|signature| ChipResponse::Signature { signature }),
                ChipRequest::Card { card } => c
                    .card_sign(&circle, &card)
                    .map(|signature| ChipResponse::Signature { signature }),
                ChipRequest::GrantSign { grant } => c
                    .grant_sign(&circle, &grant)
                    .map(|signature| ChipResponse::Signature { signature }),
                ChipRequest::SealKey { label } => c
                    .seal_key(&circle, &label)
                    .map(|key| ChipResponse::Key { key }),
                ChipRequest::Nonce => c.nonce(&circle).map(|nonce| ChipResponse::Nonce { nonce }),
                ChipRequest::GrantRequest {
                    action,
                    author,
                    location,
                    agent,
                    budget,
                    exp,
                    standing,
                    cnf,
                    time,
                } => c
                    .grant_request(
                        &circle,
                        &GrantRequestFields {
                            action,
                            author,
                            location,
                            agent,
                            budget,
                            exp,
                            standing,
                            cnf,
                            time,
                        },
                    )
                    .map(|(grant, slot)| ChipResponse::Grant { grant, slot }),
                ChipRequest::SshSignUnderGrant {
                    data,
                    content,
                    grant,
                    proof,
                    location,
                    time,
                } => c
                    .ssh_sign_under_grant(
                        &circle,
                        &UnderGrantFields {
                            data,
                            content,
                            grant,
                            proof,
                            location,
                            time,
                        },
                    )
                    .map(|(signature, remaining)| ChipResponse::GrantSignature {
                        signature,
                        remaining,
                    }),
            };
            match result {
                Ok(result) => Response::Chip { result },
                Err(code) => err(code),
            }
        }
    };
    resp.encode()
}
