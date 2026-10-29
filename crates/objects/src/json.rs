//! The JSON projection (spec §6.6), with the `std` feature. Informative only: nothing verifies
//! it, the signed CBOR is the object. Byte strings are lowercase hex; the grantee key is a JWK.

use alloc::string::String;
use alloc::vec::Vec;
use cahoots_core::seat::SeatPublic;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use serde_json::{json, Value};

use crate::{Approval, Fact, Grant, Receipt, VERSION};

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|b| [DIGITS[usize::from(b >> 4)], DIGITS[usize::from(b & 15)]])
        .map(char::from)
        .collect()
}

/// RFC 4648 §5 base64url without padding, as JWK coordinates are written.
fn b64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let mut b = [0u8; 3];
        b[..chunk.len()].copy_from_slice(chunk);
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..=chunk.len() {
            out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
        }
    }
    out
}

/// A P-256 public key as a JWK. A `SeatPublic` held in a decoded object is always a valid
/// point, so the decompression cannot fail for one; a hand-built invalid key yields nulls.
fn jwk(key: &SeatPublic) -> Value {
    let (x, y) = match p256::PublicKey::from_sec1_bytes(&key.0) {
        Ok(pk) => {
            let point = pk.to_encoded_point(false);
            (point.x().map(|x| b64url(x)), point.y().map(|y| b64url(y)))
        }
        Err(_) => (None, None),
    };
    json!({ "kty": "EC", "crv": "P-256", "x": x, "y": y })
}

fn opt_hex(h: &Option<[u8; 32]>) -> Value {
    h.as_ref().map_or(Value::Null, |h| Value::String(hex(h)))
}

impl Grant {
    /// `{"authorization_details": [{"type": "cahoots/purpose", "purpose", "circle", "budget"}],
    /// "cnf": {"jwk": ...}, "exp_log", "depth", "parent"}`, one authorization detail per purpose.
    pub fn to_json(&self) -> Value {
        let details: Vec<Value> = self
            .purposes
            .iter()
            .map(|p| {
                json!({
                    "type": "cahoots/purpose",
                    "purpose": p,
                    "circle": hex(&self.circle_id),
                    "budget": self.budget,
                })
            })
            .collect();
        json!({
            "authorization_details": details,
            "cnf": { "jwk": jwk(&self.grantee) },
            "exp_log": self.not_after,
            "depth": self.depth,
            "parent": opt_hex(&self.parent),
        })
    }
}

impl Approval {
    /// The table's fields, flat; `circle_id` as `"circle"`, `fields` as `[label, value]` pairs.
    pub fn to_json(&self) -> Value {
        json!({
            "version": VERSION,
            "circle": hex(&self.circle_id),
            "seat": self.seat,
            "profile": self.profile,
            "fields": self.fields.iter().map(|(l, v)| json!([l, v])).collect::<Vec<_>>(),
            "request_digest": hex(&self.request_digest),
            "policy_id": hex(&self.policy_id),
            "verdict": self.verdict,
            "assurance": self.assurance,
            "log": self.log,
            "nonce": hex(&self.nonce),
        })
    }
}

impl Fact {
    /// The table's fields, flat; `circle_id` as `"circle"`.
    pub fn to_json(&self) -> Value {
        json!({
            "version": VERSION,
            "circle": hex(&self.circle_id),
            "seat": self.seat,
            "module": self.module,
            "reading": hex(&self.reading),
            "counter": self.counter,
            "nonce": hex(&self.nonce),
            "log": self.log,
            "attested": self.attested,
            "evidence": hex(&self.evidence),
        })
    }
}

impl Receipt {
    /// The table's fields, flat; `circle_id` as `"circle"`.
    pub fn to_json(&self) -> Value {
        json!({
            "version": VERSION,
            "circle": hex(&self.circle_id),
            "seat": self.seat,
            "slice": self.slice,
            "offset": self.offset,
            "len": self.len,
            "purpose": self.purpose,
            "grant": opt_hex(&self.grant),
            "log": self.log,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64url_matches_rfc_4648_examples() {
        // RFC 4648 §10, unpadded, plus the two url-safe characters.
        let cases: [(&[u8], &str); 8] = [
            (b"", ""),
            (b"f", "Zg"),
            (b"fo", "Zm8"),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg"),
            (b"fooba", "Zm9vYmE"),
            (b"foobar", "Zm9vYmFy"),
            (&[0xfb, 0xff], "-_8"),
        ];
        for (input, want) in cases {
            assert_eq!(b64url(input), want);
        }
    }

    #[test]
    fn hex_is_lowercase() {
        assert_eq!(hex(&[0x00, 0xab, 0x7f]), "00ab7f");
    }
}
