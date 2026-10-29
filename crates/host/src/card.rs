//! Contact cards and grants a badge signs.
//!
//! ```text
//! cahoots-host card set (--serial <port> | --sim <socket>) --circle <name> --name <text>
//!     [--handle kind=value]... [--endpoint purpose=value]... [--log N] --out <file>
//! cahoots-host card fetch (--serial <port> | --sim <socket>) --circle <name> --state <dir>
//!     --in <file>... [--out <dir>]
//! cahoots-host card show --circle <name> --state <dir> <signed card file>
//! cahoots-host grant (--serial <port> | --sim <socket>) --circle <name> --to <key hex>
//!     --purpose <name>... --budget N --until N [--depth N] [--v2] [--out <file>]
//! ```
//!
//! `card set` builds this seat's card, asks the badge to sign it (the badge shows the name and
//! how many handles and endpoints), asks for the circle's card seal key (the badge shows "key
//! for seal/card"), seals the signed card with it and writes the sealed card to `--out`. Cards
//! travel as files for now: publishing them on the circle's station topic is not built.
//!
//! `card fetch` asks the badge for the seal key once, opens every sealed card named by
//! `--in`, checks each card's signature against the circle's manifest (kept in `--state`, the
//! directory of a badge or card of the circle, as `roster` reads it), and prints each as JSON.
//! A card that does not open or verify stops the command. With `--out`, each opened signed
//! card is written there as `<seat>.card`, for `card show`, which checks and prints one. The
//! laptop does not keep the seal key: each fetch asks the badge again.
//!
//! `grant` builds a root grant from this seat to the key `--to` (33-byte SEC1 hex), asks the
//! badge to sign it, and writes the signed grant (COSE_Sign1) to `--out` or as hex to stdout.
//! `--v2` writes the payload as a CBOR Web Token. `--depth` is how many more times the grantee
//! may delegate (0 if left out); `--until` is the last log position at which it holds.

use crate::badge_client::{describe_error, BadgeClient};
use cahoots_core::manifest::Manifest;
use cahoots_core::seat::SeatPublic;
use cahoots_custodian::wire::{ChipRequest, ChipResponse, Request, Response};
use cahoots_custodian::CircleInfo;
use cahoots_objects::card::{self, Card};
use cahoots_objects::grant::{Grant, Grantor};
use std::path::{Path, PathBuf};

/// The label of the card seal key: `HMAC-SHA256(root, "seal/card" || circle_id)`.
pub const SEAL_LABEL: &str = "seal/card";

/// The circle named `name` on the badge, with this badge's seat in it.
pub fn circle(badge: &mut BadgeClient, name: &str) -> Result<CircleInfo, String> {
    match badge
        .request(Request::Identify)
        .map_err(|e| e.to_string())?
    {
        Response::Identity { circles, .. } => circles
            .into_iter()
            .find(|c| c.name == name)
            .ok_or_else(|| format!("the badge holds no circle named {name}")),
        Response::Error { code } => Err(describe_error(code).into()),
        other => Err(format!("unexpected answer {other:?}")),
    }
}

fn chip(
    badge: &mut BadgeClient,
    info: &CircleInfo,
    op: ChipRequest,
) -> Result<ChipResponse, String> {
    let req = Request::Chip {
        circle: info.circle_id,
        op,
    };
    match badge.request(req).map_err(|e| e.to_string())? {
        Response::Chip { result } => Ok(result),
        Response::Error { code } => Err(describe_error(code).into()),
        other => Err(format!("unexpected answer {other:?}")),
    }
}

fn signature(r: ChipResponse) -> Result<Vec<u8>, String> {
    match r {
        ChipResponse::Signature { signature } => Ok(signature),
        other => Err(format!("unexpected answer {other:?}")),
    }
}

/// Ask the badge for the circle's card seal key.
pub fn seal_key(badge: &mut BadgeClient, info: &CircleInfo) -> Result<[u8; 32], String> {
    match chip(
        badge,
        info,
        ChipRequest::SealKey {
            label: SEAL_LABEL.into(),
        },
    )? {
        ChipResponse::Key { key } => Ok(key),
        other => Err(format!("unexpected answer {other:?}")),
    }
}

/// `kind=value` pairs.
fn pairs(list: &[String], what: &str) -> Result<Vec<(String, String)>, String> {
    list.iter()
        .map(|s| {
            s.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .filter(|(k, v)| !k.is_empty() && !v.is_empty())
                .ok_or_else(|| format!("{what} {s:?}: write it as name=value"))
        })
        .collect()
}

/// The card `card set` builds for this seat.
pub fn build(
    info: &CircleInfo,
    name: &str,
    handles: &[String],
    endpoints: &[String],
    log: u64,
) -> Result<Card, String> {
    let endpoints = pairs(endpoints, "--endpoint")?
        .into_iter()
        .map(|(p, v)| {
            card::purpose_from_name(&p)
                .map(|p| (p, v))
                .ok_or_else(|| {
                    format!("--endpoint {p}: the purposes are wireguard, tls, mls, wifi, meshtastic, other, ssh and chat")
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let c = Card {
        circle_id: info.circle_id,
        seat: info.seat,
        name: name.to_string(),
        handles: pairs(handles, "--handle")?,
        endpoints,
        log,
    };
    // The badge refuses what does not decode: say why here instead.
    c.encode().and_then(|b| Card::decode(&b)).map_err(|_| {
        format!(
            "the card is outside its limits: a name of 1 to {} bytes, at most {} handles and {} endpoints, kinds of at most {} bytes, values of at most {}, and {} bytes in all",
            card::MAX_NAME,
            card::MAX_ENTRIES,
            card::MAX_ENTRIES,
            card::MAX_KIND,
            card::MAX_VALUE,
            card::MAX_CARD_LEN
        )
    })
}

/// `card set`: sign the card on the badge, seal it, and write it to `out`.
pub fn set(badge: &mut BadgeClient, info: &CircleInfo, c: &Card, out: &Path) -> Result<(), String> {
    let bytes = c
        .encode()
        .map_err(|_| format!("the card is longer than {} bytes", card::MAX_CARD_LEN))?;
    let signed = signature(chip(badge, info, ChipRequest::Card { card: bytes })?)?;
    let key = seal_key(badge, info)?;
    let sealed = card::seal(&signed, &key).map_err(|e| format!("seal: {e:?}"))?;
    std::fs::write(out, sealed).map_err(|e| format!("{}: {e}", out.display()))
}

/// Check a signed card against the circle's manifest.
pub fn read_signed(signed: &[u8], manifest: &Manifest) -> Result<Card, String> {
    Card::verify(signed, manifest).map_err(|e| format!("the card does not verify: {e:?}"))
}

/// Open the sealed cards in `files` with `key` and check each: the signed bytes and the card.
pub fn open_all(
    files: &[PathBuf],
    key: &[u8; 32],
    manifest: &Manifest,
) -> Result<Vec<(Vec<u8>, Card)>, String> {
    files
        .iter()
        .map(|f| {
            let sealed = std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
            let signed = card::open(&sealed, key)
                .map_err(|_| format!("{}: not a card of this circle, or damaged", f.display()))?;
            let c = read_signed(&signed, manifest).map_err(|e| format!("{}: {e}", f.display()))?;
            Ok((signed, c))
        })
        .collect()
}

/// The root grant `grant` builds: from this seat to `to`, for `purposes`, with a random nonce.
pub fn build_grant(
    info: &CircleInfo,
    to: &str,
    purposes: &[String],
    budget: u32,
    until: u64,
    depth: u8,
) -> Result<Grant, String> {
    let key = (0..to.len())
        .step_by(2)
        .map(|i| {
            to.get(i..i + 2)
                .and_then(|b| u8::from_str_radix(b, 16).ok())
        })
        .collect::<Option<Vec<u8>>>()
        .and_then(|b| SeatPublic::parse(&b).ok())
        .ok_or("--to: a P-256 public key, 33 bytes as hex")?;
    let mut list = purposes
        .iter()
        .map(|p| {
            card::purpose_from_name(p)
                .filter(|n| *n <= 5)
                .ok_or_else(|| {
                    format!("--purpose {p}: one of wireguard, tls, mls, wifi, meshtastic, other")
                })
        })
        .collect::<Result<Vec<u8>, _>>()?;
    list.sort_unstable();
    list.dedup();
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|e| format!("random: {e}"))?;
    Ok(Grant {
        circle_id: info.circle_id,
        grantor: Grantor::Seat(info.seat),
        grantee: key,
        purposes: list,
        budget,
        not_after: until,
        depth,
        parent: None,
        nonce,
    })
}

/// Ask the badge to sign `g`, as version 2 when `v2`.
pub fn sign_grant(
    badge: &mut BadgeClient,
    info: &CircleInfo,
    g: &Grant,
    v2: bool,
) -> Result<Vec<u8>, String> {
    let payload = if v2 { g.encode_v2() } else { g.encode() };
    signature(chip(
        badge,
        info,
        ChipRequest::GrantSign { grant: payload },
    )?)
}
