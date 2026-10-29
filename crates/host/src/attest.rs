//! `cahoots-host attest (--serial <port> | --sim <socket>) --circle <name> [--test-root] [--json]`:
//! ask the badge's chip to attest a circle's seat key, and check the answer.
//!
//! A fresh random value goes to the badge with the request; the chip signs the seat key, its
//! own id, that value and its timestamp. The answer is checked by
//! `cahoots_objects::attestation::verify` against NXP's roots
//! ([`cahoots_objects::attestation::NXP_ROOTS`]), and the attested key must be the seat key
//! the badge describes for that circle. NXP's root is not vendored yet, so a real chip cannot
//! pass. With `--sim` or `--test-root` the software chip's test root
//! ([`cahoots_objects::attestation::TEST_ROOTS`]) is also accepted, and a pass against it is
//! reported as a software chip, not hardware.

use crate::badge_client::{describe_error, BadgeClient};
use cahoots_chip::Attested;
use cahoots_custodian::wire::{ChipRequest, ChipResponse, Request, Response};
use cahoots_objects::attestation::{self, AttestError, NXP_ROOTS, TEST_ROOTS};

/// What `attest` found: the JSON to print and whether the attestation verified.
pub struct Outcome {
    pub json: serde_json::Value,
    pub verified: bool,
    /// Lines for a person, when not printing JSON only.
    pub lines: Vec<String>,
}

/// A good attestation of the seat key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pass {
    /// The seat key, SEC1 compressed.
    pub public: Vec<u8>,
    /// The chip's unique id.
    pub uid: Vec<u8>,
    /// True when NXP's root verified it; false when only the software chip's test root did.
    pub hardware: bool,
}

/// Why an attestation was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The badge says the seat key is not in a chip.
    NotInChip,
    /// The badge's description of the seat key does not parse as a P-256 key.
    SeatKey,
    /// The attestation verifies, but for a key that is not the circle's seat key.
    OtherKey,
    /// The attestation itself does not verify.
    Check(AttestError),
}

/// Check `attested` for `freshness` and tie it to the seat key the badge described.
///
/// NXP's roots are always tried. The software chip's test root is tried only when
/// `allow_test_root`, and a pass against it has `hardware: false`.
pub fn check(
    seat_public: &[u8],
    in_chip: bool,
    attested: &Attested,
    freshness: &[u8; 16],
    allow_test_root: bool,
) -> Result<Pass, Refusal> {
    if !in_chip {
        return Err(Refusal::NotInChip);
    }
    let seat = attestation::compressed(seat_public).ok_or(Refusal::SeatKey)?;
    let (verified, hardware) = match attestation::verify(attested, freshness, NXP_ROOTS) {
        Ok(v) => (v, true),
        Err(AttestError::Chain) if allow_test_root => (
            attestation::verify(attested, freshness, TEST_ROOTS).map_err(Refusal::Check)?,
            false,
        ),
        Err(e) => return Err(Refusal::Check(e)),
    };
    if verified.public != seat {
        return Err(Refusal::OtherKey);
    }
    Ok(Pass {
        public: verified.public,
        uid: verified.uid,
        hardware,
    })
}

/// Ask `badge` to attest the seat key of the circle named `circle` and check it.
///
/// `allow_test_root` also accepts the software chip's test root (for simulated badges).
pub fn attest(
    badge: &mut BadgeClient,
    circle: &str,
    allow_test_root: bool,
) -> Result<Outcome, String> {
    let info = match badge
        .request(Request::Identify)
        .map_err(|e| e.to_string())?
    {
        Response::Identity { circles, .. } => circles
            .into_iter()
            .find(|c| c.name == circle)
            .ok_or_else(|| format!("the badge holds no circle named {circle}"))?,
        Response::Error { code } => return Err(describe_error(code).into()),
        other => return Err(format!("unexpected answer {other:?}")),
    };
    let (seat_public, in_chip) = match badge
        .request(Request::Chip {
            circle: info.circle_id,
            op: ChipRequest::Describe,
        })
        .map_err(|e| e.to_string())?
    {
        Response::Chip {
            result: ChipResponse::Description {
                public, in_chip, ..
            },
        } => (public, in_chip),
        Response::Error { code } => return Err(describe_error(code).into()),
        other => return Err(format!("unexpected answer {other:?}")),
    };
    if !in_chip {
        return Err(format!(
            "{circle}: the seat key is not in a chip, so there is nothing to attest"
        ));
    }
    let mut freshness = [0u8; 16];
    getrandom::fill(&mut freshness).map_err(|e| format!("random: {e}"))?;
    let req = Request::Chip {
        circle: info.circle_id,
        op: ChipRequest::Attest { freshness },
    };
    let attested = match badge.request(req).map_err(|e| e.to_string())? {
        Response::Chip {
            result: ChipResponse::Attested { attested },
        } => attested,
        Response::Error {
            code: cahoots_custodian::wire::ErrorCode::NoChip,
        } => {
            return Err(format!(
                "{circle}: the seat key is not in a chip, so there is nothing to attest"
            ))
        }
        Response::Error { code } => return Err(describe_error(code).into()),
        other => return Err(format!("unexpected answer {other:?}")),
    };
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let result = check(
        &seat_public,
        in_chip,
        &attested,
        &freshness,
        allow_test_root,
    );
    let mut lines = Vec::new();
    let check = match &result {
        Ok(p) if p.hardware => {
            lines.push(format!(
                "verified: the seat key {} is held by chip {}",
                hex(&p.public),
                hex(&p.uid)
            ));
            serde_json::json!({
                "verified": true,
                "hardware": true,
                "root": "nxp",
                "public": hex(&p.public),
                "uid": hex(&p.uid),
            })
        }
        Ok(p) => {
            lines.push(format!(
                "verified against the software chip's test root: the seat key {} is held by \
                 software chip {}, not hardware",
                hex(&p.public),
                hex(&p.uid)
            ));
            lines.push(
                "The test root's private key is in the source, so this shows only that the \
                 software agrees with itself. It says nothing about a secure element."
                    .into(),
            );
            serde_json::json!({
                "verified": true,
                "hardware": false,
                "root": "software chip test root",
                "public": hex(&p.public),
                "uid": hex(&p.uid),
            })
        }
        Err(r) => {
            lines.push(format!("not verified: {}", words(r)));
            if *r == Refusal::Check(AttestError::Chain) && NXP_ROOTS.is_empty() {
                lines.push(
                    "NXP's attestation root is not in this program yet \
                     (crates/objects/roots/README.md), so a real chip's attestation cannot \
                     pass this check."
                        .into(),
                );
            }
            serde_json::json!({ "verified": false, "hardware": false, "error": format!("{r:?}") })
        }
    };
    let json = serde_json::json!({
        "circle": circle,
        "circle_id": hex(&info.circle_id),
        "seat": info.seat,
        "seat_public": hex(&seat_public),
        "freshness": hex(&freshness),
        "attestation": attestation::to_json(&attested),
        "check": check,
    });
    Ok(Outcome {
        json,
        verified: result.is_ok(),
        lines,
    })
}

fn words(r: &Refusal) -> &'static str {
    match r {
        Refusal::NotInChip => "the badge says the seat key is not in a chip",
        Refusal::SeatKey => "the badge's seat key does not parse",
        Refusal::OtherKey => "the chip attested a key that is not this circle's seat key",
        Refusal::Check(AttestError::Signature) => "the chip's signature does not verify",
        Refusal::Check(AttestError::Chain) => {
            "the chip's certificate does not lead to a root this program trusts"
        }
        Refusal::Check(AttestError::Freshness) => {
            "the chip did not echo the value sent, so the answer may be old"
        }
        Refusal::Check(AttestError::Parse) => "a certificate or key in the answer does not parse",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cahoots_chip::ops::{ChipOps, Curve};
    use cahoots_chip::{KeyId, KeyKind, SoftChip};

    const FRESH: [u8; 16] = [9; 16];

    fn seat(chip: &mut SoftChip, slot: u8) -> (Vec<u8>, Attested) {
        let id = KeyId::new(KeyKind::Seat, slot);
        chip.generate(id, Curve::P256).unwrap();
        let att = chip.attest(id, &FRESH).unwrap();
        (att.public.clone(), att)
    }

    #[test]
    fn a_bundle_for_another_key_is_refused() {
        let mut chip = SoftChip::new();
        let (seat_a, _) = seat(&mut chip, 1);
        let (_, att_b) = seat(&mut chip, 2);
        assert_eq!(
            check(&seat_a, true, &att_b, &FRESH, true),
            Err(Refusal::OtherKey)
        );
    }

    #[test]
    fn a_test_root_pass_is_labelled_software() {
        let mut chip = SoftChip::new();
        let (seat_a, att) = seat(&mut chip, 1);
        let pass = check(&seat_a, true, &att, &FRESH, true).unwrap();
        assert!(!pass.hardware);
        assert_eq!(pass.public, seat_a);
    }

    #[test]
    fn the_seat_key_matches_in_either_form() {
        use p256::elliptic_curve::sec1::ToEncodedPoint;
        let mut chip = SoftChip::new();
        let (seat_a, att) = seat(&mut chip, 1);
        let wide = p256::PublicKey::from_sec1_bytes(&seat_a)
            .unwrap()
            .to_encoded_point(false);
        assert!(check(wide.as_bytes(), true, &att, &FRESH, true).is_ok());
    }

    #[test]
    fn without_the_test_root_a_software_chip_does_not_pass() {
        let mut chip = SoftChip::new();
        let (seat_a, att) = seat(&mut chip, 1);
        assert_eq!(
            check(&seat_a, true, &att, &FRESH, false),
            Err(Refusal::Check(AttestError::Chain))
        );
    }

    #[test]
    fn a_key_not_in_a_chip_is_refused() {
        let mut chip = SoftChip::new();
        let (seat_a, att) = seat(&mut chip, 1);
        assert_eq!(
            check(&seat_a, false, &att, &FRESH, true),
            Err(Refusal::NotInChip)
        );
    }
}
