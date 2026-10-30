//! Grants the badge consumes: issued on its screen with both buttons, honoured per use without
//! a press, counted in the chip (or the record, without one), refused when spent, expired or
//! shown another key, and swept at boot.

mod common;

use cahoots_badge::chip::{ChipRecord, SlotEntry};
use cahoots_badge::grants::{GrantRecord, GrantsRecord, ISSUE_SLOT};
use cahoots_badge::{Badge, Press};
use cahoots_chip::{ChipOps, Curve, KeyId, KeyKind, SoftChip};
use cahoots_core::frame::{self, Frame};
use cahoots_core::seat::{SeatKey, SeatPublic};
use cahoots_custodian::wire::{
    ChipRequest, ChipResponse, ErrorCode, Request, Response, KIND_REQUEST, KIND_RESPONSE,
};
use cahoots_custodian::CircleId;
use cahoots_hal::health::{HealthTests, Tested};
use cahoots_hal::Link;
use cahoots_objects::authz::{proof_sign, SignGrant};
use cahoots_objects::grant::key_id;
use common::{cable, FailingChip, FakeBoard, FakeLink, Xorshift};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use sha2::{Digest, Sha256};
use ssh_encoding::Encode;
use ssh_key::{Algorithm, EcdsaCurve, HashAlg, SshSig};

// ---- fixtures ----

const AGENT: &str = "claude on connerswann-mbp";
const AUTHOR: &str = "Conner Swann <me@connerswann.me>";
/// 29 Nov 2026, noon UTC.
const EXP: u64 = 1_795_953_600;
/// A month before.
const NOW: u64 = 1_793_361_600;
const SUBJECT: &str = "Fix the parser for long names";
/// A base the badge could have given: issue 1.
const BASE: u64 = 1 << 32;
const PROMPT_FOOTER: &str = "refuse  A+B  allow  refuse";

fn commit_by(author: &str, committer: &str) -> String {
    format!(
        "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
         author {author} 1700000000 +0000\n\
         committer {committer} 1700000000 +0000\n\
         \n\
         {SUBJECT}\n\
         \n\
         A body that is not shown.\n"
    )
}

fn commit() -> String {
    commit_by(AUTHOR, AUTHOR)
}

fn agent_key() -> SeatKey {
    SeatKey::from_seed(&[3; 32]).unwrap()
}

fn other_key() -> SeatKey {
    SeatKey::from_seed(&[4; 32]).unwrap()
}

fn enc(x: &impl Encode) -> Vec<u8> {
    let mut v = Vec::new();
    x.encode(&mut v).unwrap();
    v
}

fn seat_blob(public: &SeatPublic) -> Vec<u8> {
    let point = p256::PublicKey::from_sec1_bytes(&public.0).unwrap();
    let uncompressed = point.to_encoded_point(false);
    let key = ssh_key::public::EcdsaPublicKey::from_sec1_bytes(uncompressed.as_bytes()).unwrap();
    enc(&ssh_key::public::KeyData::from(key))
}

fn assert_verifies(public: &SeatPublic, data: &[u8], sig: &[u8]) {
    let parsed = ssh_key::Signature::try_from(sig).expect("an ssh signature");
    assert_eq!(
        parsed.algorithm(),
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256
        }
    );
    let key = ssh_key::PublicKey::from_bytes(&seat_blob(public)).unwrap();
    signature::Verifier::verify(&key, data, &parsed).expect("verifies with the seat key");
}

fn sshsig(content: &[u8]) -> Vec<u8> {
    SshSig::signed_data("git", HashAlg::Sha512, content).unwrap()
}

// ---- the Serve path, as tests/ssh.rs drives it ----

fn send(laptop: &mut FakeLink, seq: u32, req: &Request) {
    let payload = req.encode();
    laptop.send(&frame::encode(&Frame {
        kind: KIND_REQUEST,
        seq,
        payload: &payload,
    }));
}

fn responses(laptop: &mut FakeLink) -> Vec<(u32, Response)> {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let n = laptop.recv(&mut buf);
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let (f, used) = frame::decode_max(&bytes[at..], 1 << 16).expect("a whole frame");
        assert_eq!(f.kind, KIND_RESPONSE);
        out.push((f.seq, Response::decode(f.payload).expect("a response")));
        at += used;
    }
    out
}

fn press(badge: &mut Badge<FakeBoard>, gesture: Press) {
    badge.board().presses.push_back(gesture);
    badge.idle_step();
}

/// The lines of the screen before the last: the prompt, when the last is the page again.
fn prompt(badge: &mut Badge<FakeBoard>) -> Vec<String> {
    let screens = &badge.board().screens;
    screens[screens.len() - 2].clone()
}

/// A board with one circle whose seat key is in a SoftChip in slot 0 (`chip`) or in software.
fn board(chip: bool) -> (FakeBoard, CircleId, SeatPublic) {
    let (mut board, _, _) = FakeBoard::with_circle();
    let id = board.circle.as_ref().unwrap().circle_id;
    let public = if chip {
        let mut c = SoftChip::with_seed([9; 32]);
        let p = c
            .generate(KeyId::new(KeyKind::Seat, 0), Curve::P256)
            .unwrap();
        let mut record = ChipRecord::default();
        record.insert(SlotEntry {
            slot: 0,
            circle_id: id,
            has_seat: true,
            has_root: false,
            has_epoch: false,
        });
        board.chip_record = Some(record.encode());
        board.chip = Some(c);
        let circle = board.circle.as_mut().unwrap();
        circle.chip_slot = Some(0);
        circle.seat_key = [0; 32];
        SeatPublic::parse(&p).unwrap()
    } else {
        SeatKey::from_seed(&[7; 32]).unwrap().public()
    };
    (board, id, public)
}

/// A badge serving its down link with the laptop allowed, the laptop end, the circle id, and
/// the seat public key.
fn serving(chip: bool) -> (Badge<FakeBoard>, FakeLink, CircleId, SeatPublic) {
    let (mut board, id, public) = board(chip);
    let (mut laptop, down) = cable();
    board.links = Some((down, FakeLink::open()));
    let mut badge = Badge::new(board);
    badge.boot();
    badge.idle_step();
    send(&mut laptop, 900, &Request::Identify);
    badge.idle_step();
    assert!(
        badge.board().shows("a laptop"),
        "{:?}",
        badge.board().last()
    );
    press(&mut badge, Press::Both);
    let got = responses(&mut laptop);
    assert!(
        matches!(got.as_slice(), [(900, Response::Identity { .. })]),
        "{got:?}"
    );
    (badge, laptop, id, public)
}

fn chip_req(id: CircleId, op: ChipRequest) -> Request {
    Request::Chip { circle: id, op }
}

/// Send `op`, step so the prompt is drawn, answer with `gesture`, and return the response.
fn ask(
    badge: &mut Badge<FakeBoard>,
    laptop: &mut FakeLink,
    id: CircleId,
    op: ChipRequest,
    gesture: Press,
) -> Response {
    send(laptop, 7, &chip_req(id, op));
    badge.idle_step();
    assert!(responses(laptop).is_empty(), "answered before asking");
    press(badge, gesture);
    match responses(laptop).as_slice() {
        [(7, r)] => r.clone(),
        other => panic!("not one answer: {other:?}"),
    }
}

/// Send `op` and step once: the answer must come back without a screen.
fn ask_now(
    badge: &mut Badge<FakeBoard>,
    laptop: &mut FakeLink,
    id: CircleId,
    op: ChipRequest,
) -> Response {
    let before = badge.board().screens.len();
    send(laptop, 8, &chip_req(id, op));
    badge.idle_step();
    assert_eq!(badge.board().screens.len(), before, "a screen was drawn");
    match responses(laptop).as_slice() {
        [(8, r)] => r.clone(),
        other => panic!("not one answer: {other:?}"),
    }
}

/// Send `op` and step once: the answer must come back on that step, and no screen drawn
/// meanwhile may ask anything (the badge may show what it signed).
fn ask_unprompted(
    badge: &mut Badge<FakeBoard>,
    laptop: &mut FakeLink,
    id: CircleId,
    op: ChipRequest,
) -> Response {
    let before = badge.board().screens.len();
    send(laptop, 9, &chip_req(id, op));
    badge.idle_step();
    let asked = badge.board().screens[before..]
        .iter()
        .any(|s| s.iter().any(|l| l.contains("A+B")));
    assert!(
        !asked,
        "a prompt was drawn: {:?}",
        badge.board().screens[before..].to_vec()
    );
    match responses(laptop).as_slice() {
        [(9, r)] => r.clone(),
        other => panic!("not one answer: {other:?}"),
    }
}

fn grant_request(budget: u32, exp: Option<u64>, standing: bool, time: u64) -> ChipRequest {
    ChipRequest::GrantRequest {
        action: "git-commit".into(),
        author: AUTHOR.into(),
        location: None,
        agent: AGENT.into(),
        budget,
        exp,
        standing,
        cnf: agent_key().public().0,
        time,
    }
}

/// Ask for `op`, allow it with Both, and return the signed grant and its slot.
fn issue(
    badge: &mut Badge<FakeBoard>,
    laptop: &mut FakeLink,
    id: CircleId,
    op: ChipRequest,
) -> (Vec<u8>, u8) {
    match ask(badge, laptop, id, op, Press::Both) {
        Response::Chip {
            result: ChipResponse::Grant { grant, slot },
        } => (grant, slot),
        other => panic!("not a grant: {other:?}"),
    }
}

fn nonce(badge: &mut Badge<FakeBoard>, laptop: &mut FakeLink, id: CircleId) -> [u8; 16] {
    match ask_now(badge, laptop, id, ChipRequest::Nonce) {
        Response::Chip {
            result: ChipResponse::Nonce { nonce },
        } => nonce,
        other => panic!("not a nonce: {other:?}"),
    }
}

/// A request to sign `commit` under `grant`, proved by `key` over `nonce`.
fn under_grant(
    grant: &[u8],
    nonce: &[u8; 16],
    commit: &str,
    key: &SeatKey,
    time: u64,
) -> ChipRequest {
    let data = sshsig(commit.as_bytes());
    let hash: [u8; 32] = Sha256::digest(&data).into();
    ChipRequest::SshSignUnderGrant {
        proof: proof_sign(key, &hash, nonce),
        data,
        content: commit.as_bytes().to_vec(),
        grant: grant.to_vec(),
        location: None,
        time,
    }
}

/// Fetch a nonce, then sign the usual commit under `grant` with the agent's key at `time`.
fn use_grant(
    badge: &mut Badge<FakeBoard>,
    laptop: &mut FakeLink,
    id: CircleId,
    grant: &[u8],
    time: u64,
) -> Response {
    let n = nonce(badge, laptop, id);
    let op = under_grant(grant, &n, &commit(), &agent_key(), time);
    ask_unprompted(badge, laptop, id, op)
}

fn remaining_of(r: &Response) -> u32 {
    match r {
        Response::Chip {
            result: ChipResponse::GrantSignature { remaining, .. },
        } => *remaining,
        other => panic!("not a signature under a grant: {other:?}"),
    }
}

/// The chip counter of `slot` as it reads, its base included.
fn raw_counter(badge: &mut Badge<FakeBoard>, slot: u8) -> Option<u64> {
    badge
        .board()
        .chip
        .as_mut()
        .unwrap()
        .counter_get(KeyId::new(KeyKind::Grant, slot))
        .ok()
}

/// The uses the chip counted in `slot`: its counter less the base of the grant the record
/// holds there (or the raw value when the record holds none).
fn counter(badge: &mut Badge<FakeBoard>, slot: u8) -> Option<u64> {
    let base = record(badge)
        .grants
        .iter()
        .find(|g| g.slot == slot)
        .map_or(0, |g| g.base);
    raw_counter(badge, slot).map(|n| n - base)
}

fn record(badge: &mut Badge<FakeBoard>) -> GrantsRecord {
    match &badge.board().grants {
        Some(bytes) => GrantsRecord::decode(bytes).expect("a grants record"),
        None => GrantsRecord::default(),
    }
}

fn denied() -> Response {
    Response::Error {
        code: ErrorCode::Denied,
    }
}

fn unparseable() -> Response {
    Response::Error {
        code: ErrorCode::Unparseable,
    }
}

// ---- issuing ----

#[test]
fn a_grant_is_issued_after_both_and_the_counter_starts_at_zero() {
    let (mut badge, mut laptop, id, public) = serving(true);
    let (grant, slot) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(EXP), false, NOW),
    );
    assert_eq!(
        prompt(&mut badge),
        [
            "let an agent sign",
            "claude on connerswann-mbp (laptop)",
            "git commits as",
            "Conner Swann <me@connerswann.me>",
            "50 of them",
            "until 29 Nov 2026 (laptop)",
            "fakes",
            PROMPT_FOOTER,
        ]
    );
    assert_eq!(slot, 0);
    let g = SignGrant::verify(&grant, &public, &id, 1).expect("signed by the seat");
    assert_eq!(g.agent, AGENT);
    assert_eq!(g.author, AUTHOR);
    assert_eq!(g.budget, 50);
    assert_eq!(g.exp, Some(EXP));
    assert_eq!(g.grantee, agent_key().public());
    let r = record(&mut badge);
    let base = r.grants[0].base;
    assert_ne!(base, 0);
    assert_eq!(raw_counter(&mut badge, 0), Some(base));
    assert!(
        raw_counter(&mut badge, ISSUE_SLOT).is_some(),
        "the issue counter exists"
    );
    assert_eq!(r.grants.len(), 1);
    assert_eq!(r.grants[0].cti, g.cti);
    assert_eq!(r.grants[0].key_id, key_id(&agent_key().public()));
    assert_eq!(r.known_time, NOW);
}

#[test]
fn left_refuses_a_grant() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let r = ask(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(EXP), false, NOW),
        Press::Left,
    );
    assert_eq!(r, denied());
    assert_eq!(counter(&mut badge, 0), None);
    assert!(record(&mut badge).grants.is_empty());
}

#[test]
fn standing_with_an_expiry_is_refused() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let r = ask_now(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(EXP), true, NOW),
    );
    assert_eq!(r, denied());
    // So are no expiry without standing, another action, and budgets out of range.
    let r = ask_now(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, None, false, NOW),
    );
    assert_eq!(r, denied());
    for budget in [0, 1001] {
        let r = ask_now(
            &mut badge,
            &mut laptop,
            id,
            grant_request(budget, Some(EXP), false, NOW),
        );
        assert_eq!(r, denied());
    }
    let mut op = grant_request(50, Some(EXP), false, NOW);
    if let ChipRequest::GrantRequest { action, .. } = &mut op {
        *action = "ssh-login".into();
    }
    assert_eq!(ask_now(&mut badge, &mut laptop, id, op), denied());
    assert!(record(&mut badge).grants.is_empty());
}

#[test]
fn a_ninth_grant_is_denied() {
    let (mut badge, mut laptop, id, _) = serving(true);
    for i in 0..8u8 {
        let (_, slot) = issue(
            &mut badge,
            &mut laptop,
            id,
            grant_request(5, Some(EXP), false, NOW),
        );
        assert_eq!(slot, i);
    }
    let r = ask_now(
        &mut badge,
        &mut laptop,
        id,
        grant_request(5, Some(EXP), false, NOW),
    );
    assert_eq!(r, denied());
    assert_eq!(record(&mut badge).grants.len(), 8);
}

// ---- using ----

#[test]
fn a_use_signs_without_a_prompt_and_counts() {
    let (mut badge, mut laptop, id, public) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(EXP), false, NOW),
    );
    let n = nonce(&mut badge, &mut laptop, id);
    let op = under_grant(&grant, &n, &commit(), &agent_key(), NOW + 60);
    let data = sshsig(commit().as_bytes());
    let r = ask_unprompted(&mut badge, &mut laptop, id, op);
    match &r {
        Response::Chip {
            result:
                ChipResponse::GrantSignature {
                    signature,
                    remaining,
                },
        } => {
            assert_eq!(*remaining, 49);
            assert_verifies(&public, &data, signature);
        }
        other => panic!("not a signature under a grant: {other:?}"),
    }
    assert_eq!(counter(&mut badge, 0), Some(1));
    assert_eq!(
        badge.board().last(),
        [
            "signed for claude on connerswann-mbp",
            SUBJECT,
            "49 left of 50"
        ]
    );
    let cti = SignGrant::verify(&grant, &public, &id, 1).unwrap().cti;
    let short: String = cti[..4].iter().map(|b| format!("{b:02x}")).collect();
    let line = format!("grant {short}: signed git commit \"{SUBJECT}\", 49 left");
    assert!(badge.board().log.contains(&line), "{:?}", badge.board().log);
    // A fresh nonce for the next use.
    assert_ne!(nonce(&mut badge, &mut laptop, id), n);
}

#[test]
fn a_replayed_proof_is_refused() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(EXP), false, NOW),
    );
    let n = nonce(&mut badge, &mut laptop, id);
    let op = under_grant(&grant, &n, &commit(), &agent_key(), NOW);
    let r = ask_unprompted(&mut badge, &mut laptop, id, op.clone());
    assert_eq!(remaining_of(&r), 49);
    let r = ask_unprompted(&mut badge, &mut laptop, id, op);
    assert_eq!(r, unparseable());
    assert_eq!(counter(&mut badge, 0), Some(1));
}

#[test]
fn committer_may_differ_from_author() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(EXP), false, NOW),
    );
    let n = nonce(&mut badge, &mut laptop, id);
    let text = commit_by(AUTHOR, "GitHub <noreply@github.com>");
    let op = under_grant(&grant, &n, &text, &agent_key(), NOW);
    let r = ask_unprompted(&mut badge, &mut laptop, id, op);
    assert_eq!(remaining_of(&r), 49);
}

#[test]
fn another_author_falls_back_to_the_prompt() {
    let (mut badge, mut laptop, id, public) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(EXP), false, NOW),
    );
    let n = nonce(&mut badge, &mut laptop, id);
    let text = commit_by("Ada Lovelace <ada@example.com>", AUTHOR);
    let op = under_grant(&grant, &n, &text, &agent_key(), NOW);
    let r = ask(&mut badge, &mut laptop, id, op, Press::Both);
    assert_eq!(
        prompt(&mut badge),
        [
            "sign a git commit",
            "Ada Lovelace",
            SUBJECT,
            "fakes",
            PROMPT_FOOTER
        ]
    );
    // Allowed on the screen, it is an ordinary signature: nothing is counted.
    match r {
        Response::Chip {
            result: ChipResponse::Signature { signature },
        } => assert_verifies(&public, &sshsig(text.as_bytes()), &signature),
        other => panic!("not a signature: {other:?}"),
    }
    assert_eq!(counter(&mut badge, 0), Some(0));
    assert!(badge.board().log.iter().any(|l| l.contains("author")));
}

#[test]
fn another_key_is_refused_without_a_prompt() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(EXP), false, NOW),
    );
    let n = nonce(&mut badge, &mut laptop, id);
    let op = under_grant(&grant, &n, &commit(), &other_key(), NOW);
    let r = ask_unprompted(&mut badge, &mut laptop, id, op);
    assert_eq!(r, unparseable());
    assert_eq!(counter(&mut badge, 0), Some(0));
}

#[test]
fn a_spent_grant_is_refused_and_removed() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(1, Some(EXP), false, NOW),
    );
    let r = use_grant(&mut badge, &mut laptop, id, &grant, NOW);
    assert_eq!(remaining_of(&r), 0);
    let r = use_grant(&mut badge, &mut laptop, id, &grant, NOW);
    assert_eq!(r, denied());
    assert_eq!(counter(&mut badge, 0), None);
    assert!(record(&mut badge).grants.is_empty());
    assert!(badge.board().log.iter().any(|l| l.contains("spent")));
}

#[test]
fn an_expired_grant_is_refused_and_removed() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(100), false, 50),
    );
    let r = use_grant(&mut badge, &mut laptop, id, &grant, 200);
    assert_eq!(r, denied());
    assert_eq!(counter(&mut badge, 0), None);
    assert!(record(&mut badge).grants.is_empty());
    assert!(badge.board().log.iter().any(|l| l.contains("expired")));
}

#[test]
fn an_earlier_laptop_time_does_not_lower_known_time() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(800), false, 1000),
    );
    let r = use_grant(&mut badge, &mut laptop, id, &grant, 500);
    assert_eq!(r, denied());
    assert_eq!(counter(&mut badge, 0), None);
}

#[test]
fn the_badge_keeps_time_with_its_uptime() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(1000), false, 900),
    );
    // A laptop that says it is still 900 cannot hold the time back: the badge counted 200 s.
    badge.board().uptime += 200;
    let r = use_grant(&mut badge, &mut laptop, id, &grant, 900);
    assert_eq!(r, denied());
}

#[test]
fn the_stored_time_is_written_when_an_hour_behind() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(EXP), false, NOW),
    );
    assert_eq!(record(&mut badge).known_time, NOW);
    // Within the hour: the flash is not written for the time alone.
    let r = use_grant(&mut badge, &mut laptop, id, &grant, NOW + 600);
    assert_eq!(remaining_of(&r), 49);
    assert_eq!(record(&mut badge).known_time, NOW);
    let r = use_grant(&mut badge, &mut laptop, id, &grant, NOW + 7200);
    assert_eq!(remaining_of(&r), 48);
    assert_eq!(record(&mut badge).known_time, NOW + 7200);
}

#[test]
fn a_standing_grant_never_expires() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, None, true, NOW),
    );
    assert!(prompt(&mut badge).contains(&String::from("until revoked")));
    let r = use_grant(&mut badge, &mut laptop, id, &grant, u64::MAX / 2);
    assert_eq!(remaining_of(&r), 49);
}

#[test]
fn a_restored_record_does_not_refund_the_chip_counter() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(2, Some(EXP), false, NOW),
    );
    let r = use_grant(&mut badge, &mut laptop, id, &grant, NOW);
    assert_eq!(remaining_of(&r), 1);
    let image = badge.board().grants.clone();
    let r = use_grant(&mut badge, &mut laptop, id, &grant, NOW);
    assert_eq!(remaining_of(&r), 0);
    badge.board().grants = image.clone();
    // The flash says one use; the chip says two, and the chip is what counts.
    assert_eq!(counter(&mut badge, 0), Some(2));
    let r = use_grant(&mut badge, &mut laptop, id, &grant, NOW);
    assert_eq!(r, denied());
    // Spent: the counter and the record are gone. A record restored again has no counter,
    // and a grant without its counter is refused.
    assert_eq!(counter(&mut badge, 0), None);
    badge.board().grants = image.clone();
    let r = use_grant(&mut badge, &mut laptop, id, &grant, NOW);
    assert_eq!(r, denied());

    // A later grant in the same slot: an image holding the earlier grant's record, presented
    // with the earlier grant and its key, neither spends nor resets the later one's counter.
    badge.board().grants = None;
    let (_, slot) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(2, Some(EXP), false, NOW),
    );
    assert_eq!(slot, 0);
    let later = raw_counter(&mut badge, 0).unwrap();
    let later_record = badge.board().grants.clone();
    badge.board().grants = image;
    let r = use_grant(&mut badge, &mut laptop, id, &grant, NOW);
    assert_eq!(r, denied());
    assert_eq!(raw_counter(&mut badge, 0), Some(later));
    // The later grant still has its whole budget.
    badge.board().grants = later_record;
    assert_eq!(counter(&mut badge, 0), Some(0));
}

#[test]
fn a_badge_without_a_chip_counts_in_its_record() {
    let (mut badge, mut laptop, id, public) = serving(false);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(EXP), false, NOW),
    );
    SignGrant::verify(&grant, &public, &id, 1).expect("signed by the software seat key");
    for left in [49, 48] {
        let r = use_grant(&mut badge, &mut laptop, id, &grant, NOW);
        assert_eq!(remaining_of(&r), left);
    }
    assert_eq!(record(&mut badge).grants[0].used_soft, 2);
}

// ---- the sweep at boot ----

fn grant_record(slot: u8, id: CircleId) -> GrantRecord {
    GrantRecord {
        slot,
        cti: [slot; 16],
        circle_id: id,
        seat: 1,
        key_id: key_id(&agent_key().public()),
        agent: AGENT.into(),
        action: "git-commit".into(),
        author: AUTHOR.into(),
        locations: Vec::new(),
        budget: 10,
        exp: Some(EXP),
        revoked: false,
        used_soft: 0,
        base: BASE,
    }
}

#[test]
fn sweep_deletes_orphans() {
    let (mut board, id, _) = board(true);
    let chip = board.chip.as_mut().unwrap();
    // Slot 5: a counter with no record. Slot 6: a record with no counter. Slot 1: both,
    // which stays. Slot 2: both, expired by the stored time.
    chip.put_counter(KeyId::new(KeyKind::Grant, 5), 0).unwrap();
    chip.put_counter(KeyId::new(KeyKind::Grant, ISSUE_SLOT), 7)
        .unwrap();
    chip.put_counter(KeyId::new(KeyKind::Grant, 1), BASE + 3).unwrap();
    chip.put_counter(KeyId::new(KeyKind::Grant, 2), BASE).unwrap();
    let mut expired = grant_record(2, id);
    expired.exp = Some(NOW - 1);
    let stored = GrantsRecord {
        known_time: NOW,
        grants: vec![grant_record(1, id), expired, grant_record(6, id)],
    };
    board.grants = Some(stored.encode());
    let mut badge = Badge::new(board);
    badge.boot();
    assert_eq!(counter(&mut badge, 5), None);
    assert_eq!(counter(&mut badge, 2), None);
    assert_eq!(counter(&mut badge, 1), Some(3));
    // The issue counter is not a grant's, and stays.
    assert_eq!(raw_counter(&mut badge, ISSUE_SLOT), Some(7));
    let left = record(&mut badge);
    assert_eq!(left.grants, vec![grant_record(1, id)]);
    let log = badge.board().log.join("\n");
    assert!(log.contains("slot 5"), "{log}");
    assert!(log.contains("slot 6"), "{log}");
    assert!(log.contains("slot 2"), "{log}");
    // The seat key the chip record knows is left alone.
    assert!(badge
        .board()
        .chip
        .as_mut()
        .unwrap()
        .exists(KeyId::new(KeyKind::Seat, 0))
        .unwrap());
}

// ---- the record ----

#[test]
fn the_record_round_trips_and_refuses_what_is_not_canonical() {
    let id = [0x11; 16];
    let r = GrantsRecord {
        known_time: NOW,
        grants: vec![grant_record(0, id), grant_record(3, id)],
    };
    let bytes = r.encode();
    assert_eq!(GrantsRecord::decode(&bytes), Ok(r.clone()));
    assert_eq!(r.next_free_slot(), Some(1));
    // The same slot twice is refused.
    let twice = GrantsRecord {
        known_time: NOW,
        grants: vec![grant_record(0, id), grant_record(0, id)],
    };
    assert!(GrantsRecord::decode(&twice.encode()).is_err());
    assert!(GrantsRecord::decode(&[bytes.as_slice(), &[0]].concat()).is_err());
}

// ---- review fixes ----

fn busy() -> Response {
    Response::Error {
        code: ErrorCode::Busy,
    }
}

#[test]
fn a_field_that_would_not_show_whole_is_refused_before_any_screen() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let long_author = "Conner Swann <a-very-long-address-for-a-test@connerswann.me>";
    assert_eq!(long_author.len(), 60);
    let cases = [
        (AGENT, long_author),
        ("claude\non connerswann-mbp", AUTHOR),
        (AGENT, "Zo\u{eb} Swann <me@connerswann.me>"),
        // 40 characters: with " (laptop)" one more than two rows hold.
        ("an agent with a name of forty characters", AUTHOR),
    ];
    for (agent, author) in cases {
        let mut op = grant_request(50, Some(EXP), false, NOW);
        if let ChipRequest::GrantRequest {
            agent: a,
            author: b,
            ..
        } = &mut op
        {
            *a = agent.into();
            *b = author.into();
        }
        assert_eq!(
            ask_now(&mut badge, &mut laptop, id, op),
            denied(),
            "{agent:?} {author:?}"
        );
    }
    assert!(record(&mut badge).grants.is_empty());
    assert!(badge.board().log.iter().any(|l| l.contains("show whole")));
}

#[test]
fn a_failed_random_source_gives_no_nonce_and_no_use() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(EXP), false, NOW),
    );
    let n = nonce(&mut badge, &mut laptop, id);
    // From here the source gives only zeros, which fail its health tests.
    badge.board().entropy = Tested::new(Xorshift(0), HealthTests::for_min_entropy(6));
    let op = under_grant(&grant, &n, &commit(), &agent_key(), NOW);
    assert_eq!(
        remaining_of(&ask_unprompted(&mut badge, &mut laptop, id, op.clone())),
        49
    );
    // The spent nonce could not be replaced: none is given, and no use is accepted.
    assert_eq!(
        ask_now(&mut badge, &mut laptop, id, ChipRequest::Nonce),
        busy()
    );
    assert_eq!(ask_unprompted(&mut badge, &mut laptop, id, op), busy());
    let zeros = under_grant(&grant, &[0; 16], &commit(), &agent_key(), NOW);
    assert_eq!(ask_unprompted(&mut badge, &mut laptop, id, zeros), busy());
    assert_eq!(counter(&mut badge, 0), Some(1));
    // A source that recovers gives a nonce again.
    badge.board().entropy = Tested::new(Xorshift(77), HealthTests::for_min_entropy(6));
    let r = use_grant(&mut badge, &mut laptop, id, &grant, NOW);
    assert_eq!(remaining_of(&r), 48);
}

#[test]
fn a_record_with_a_base_the_badge_does_not_give_is_refused() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(2, Some(EXP), false, NOW),
    );
    let r = use_grant(&mut badge, &mut laptop, id, &grant, NOW);
    assert_eq!(remaining_of(&r), 1);
    let honest = record(&mut badge);
    let raw = raw_counter(&mut badge, 0).unwrap();
    // A crafted record that moves its base up to the counter to win the budget back, and one
    // with no base at all, are both refused, and the counter does not move.
    for base in [raw, 0] {
        let mut crafted = honest.clone();
        crafted.grants[0].base = base;
        badge.board().grants = Some(crafted.encode());
        let r = use_grant(&mut badge, &mut laptop, id, &grant, NOW);
        assert_eq!(r, denied(), "base {base:#x}");
        assert_eq!(raw_counter(&mut badge, 0), Some(raw));
    }
    assert!(badge.board().log.iter().any(|l| l.contains("base")));
}

#[test]
fn the_signed_budget_counts_not_the_record() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(1, Some(EXP), false, NOW),
    );
    let mut r = record(&mut badge);
    r.grants[0].budget = 1000;
    badge.board().grants = Some(r.encode());
    let used = use_grant(&mut badge, &mut laptop, id, &grant, NOW);
    assert_eq!(remaining_of(&used), 0);
    let r = use_grant(&mut badge, &mut laptop, id, &grant, NOW);
    assert_eq!(r, denied());
}

#[test]
fn a_proof_is_not_accepted_twice_even_when_signing_fails() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let (grant, _) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(EXP), false, NOW),
    );
    let base = record(&mut badge).grants[0].base;
    let n = nonce(&mut badge, &mut laptop, id);
    let chip = badge.board().chip.take().unwrap();
    badge.board().failing = Some(FailingChip {
        inner: chip,
        fail_sign: true,
    });
    let op = under_grant(&grant, &n, &commit(), &agent_key(), NOW);
    let r = ask_unprompted(&mut badge, &mut laptop, id, op.clone());
    assert_eq!(
        r,
        Response::Error {
            code: ErrorCode::ChipError
        }
    );
    // The use was counted before the signature failed; the same proof cannot count again.
    badge.board().failing.as_mut().unwrap().fail_sign = false;
    let r = ask_unprompted(&mut badge, &mut laptop, id, op);
    assert_eq!(r, unparseable());
    let failing = badge.board().failing.as_mut().unwrap();
    assert_eq!(
        failing.inner.counter_get(KeyId::new(KeyKind::Grant, 0)),
        Ok(base + 1)
    );
}

// ---- the grants page ----

#[test]
fn revoking_from_the_page_ends_the_grant() {
    let (mut badge, mut laptop, id, public) = serving(true);
    let (grant, slot) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(EXP), false, NOW),
    );
    press(&mut badge, Press::Right);
    press(&mut badge, Press::Right);
    assert!(badge.board().shows("3/5"), "{:?}", badge.board().last());
    assert!(badge.board().shows("grants"));
    assert!(badge.board().shows("> claude on conner..."), "{:?}", badge.board().last());
    assert!(badge.board().shows("0 of 50 \u{b7} until 29 Nov"));
    assert!(badge.board().shows("prev  A+B  revoke  next"));
    // Left on the prompt goes back and revokes nothing.
    press(&mut badge, Press::Both);
    assert!(badge.board().shows("revoke this grant?"));
    assert!(badge.board().shows("it stops at once"));
    assert!(badge.board().shows("back  A+B  revoke  back"));
    press(&mut badge, Press::Left);
    assert!(badge.board().shows("3/5"));
    assert!(!record(&mut badge).grants[0].revoked);
    assert_eq!(counter(&mut badge, slot), Some(0));
    // Both twice revokes.
    press(&mut badge, Press::Both);
    press(&mut badge, Press::Both);
    let cti = SignGrant::verify(&grant, &public, &id, 1).unwrap().cti;
    let short: String = cti[..4].iter().map(|b| format!("{b:02x}")).collect();
    let line = format!("grant {short}: revoked");
    assert!(badge.board().log.contains(&line), "{:?}", badge.board().log);
    assert_eq!(raw_counter(&mut badge, slot), None);
    assert!(record(&mut badge).grants[0].revoked);
    assert!(
        badge.board().shows("no grants"),
        "{:?}",
        badge.board().last()
    );
    assert_eq!(
        use_grant(&mut badge, &mut laptop, id, &grant, NOW + 60),
        denied()
    );
}

#[test]
fn a_request_over_the_revoke_question_cancels_it() {
    let (mut badge, mut laptop, id, _) = serving(true);
    let (_, slot) = issue(
        &mut badge,
        &mut laptop,
        id,
        grant_request(50, Some(EXP), false, NOW),
    );
    press(&mut badge, Press::Right);
    press(&mut badge, Press::Right);
    press(&mut badge, Press::Both);
    assert!(badge.board().shows("revoke this grant?"));
    send(
        &mut laptop,
        31,
        &Request::Psk {
            circle: id,
            context: b"wg".to_vec(),
        },
    );
    badge.idle_step();
    assert!(badge.board().shows("spend 32 bytes"));
    press(&mut badge, Press::Both);
    assert!(matches!(
        responses(&mut laptop).as_slice(),
        [(31, Response::Psk { .. })]
    ));
    assert!(badge.board().shows("3/5"));
    // Both now is the page's: it asks again and revokes nothing.
    press(&mut badge, Press::Both);
    assert!(badge.board().shows("revoke this grant?"));
    assert!(!record(&mut badge).grants[0].revoked);
    assert_eq!(counter(&mut badge, slot), Some(0));
}
