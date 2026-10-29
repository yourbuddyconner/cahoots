//! `cahoots-host attest (--serial <port> | --sim <socket>) --circle <name> [--json]`: ask the
//! badge's chip to attest a circle's seat key, and check the answer.
//!
//! A fresh random value goes to the badge with the request; the chip signs the seat key, its
//! own id, that value and its timestamp. The answer is printed as JSON and checked by
//! `cahoots_objects::attestation::verify` against the roots built into this program
//! ([`cahoots_objects::attestation::NXP_ROOTS`]). Those roots are, for now, the software
//! chip's test CA, so a real chip fails the check until NXP's root is vendored; the output
//! says so.

use crate::badge_client::{describe_error, BadgeClient};
use cahoots_custodian::wire::{ChipRequest, ChipResponse, Request, Response};
use cahoots_objects::attestation::{self, AttestError, NXP_ROOTS};

/// What `attest` found: the JSON to print and whether the attestation verified.
pub struct Outcome {
    pub json: serde_json::Value,
    pub verified: bool,
    /// Lines for a person, when not printing JSON only.
    pub lines: Vec<String>,
}

/// Ask `badge` to attest the seat key of the circle named `circle` and check it.
pub fn attest(badge: &mut BadgeClient, circle: &str) -> Result<Outcome, String> {
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
    let result = attestation::verify(&attested, &freshness, NXP_ROOTS);
    let mut lines = Vec::new();
    let check = match &result {
        Ok(v) => {
            lines.push(format!(
                "verified: the seat key {} is held by chip {}",
                hex(&v.public),
                hex(&v.uid)
            ));
            serde_json::json!({ "verified": true, "public": hex(&v.public), "uid": hex(&v.uid) })
        }
        Err(e) => {
            lines.push(format!("not verified: {}", words(*e)));
            if *e == AttestError::Chain {
                lines.push(
                    "The roots built into this program are the software chip's test CA \
                     (crates/objects/roots/README.md). A real SE051 fails this check until \
                     NXP's attestation root is put in their place."
                        .into(),
                );
            }
            serde_json::json!({ "verified": false, "error": format!("{e:?}") })
        }
    };
    let json = serde_json::json!({
        "circle": circle,
        "circle_id": hex(&info.circle_id),
        "seat": info.seat,
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

fn words(e: AttestError) -> &'static str {
    match e {
        AttestError::Signature => "the chip's signature does not verify",
        AttestError::Chain => "the chip's certificate does not lead to a root this program trusts",
        AttestError::Freshness => "the chip did not echo the value sent, so the answer may be old",
        AttestError::Parse => "a certificate or key in the answer does not parse",
    }
}
