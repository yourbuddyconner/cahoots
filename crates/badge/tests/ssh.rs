//! Signing for ssh on the badge: the parser for what OpenSSH asks an agent to sign, the check
//! of the laptop's session binding, the screens, and the Serve path end to end.

mod common;

use cahoots_badge::chip::{ChipRecord, SlotEntry};
use cahoots_badge::screens;
use cahoots_badge::ssh::{self, HostCheck, Signed};
use cahoots_badge::{Badge, Press};
use cahoots_chip::{ChipOps, Curve, KeyId, KeyKind, SoftChip};
use cahoots_core::frame::{self, Frame};
use cahoots_core::seat::{SeatKey, SeatPublic};
use cahoots_custodian::wire::{
    ChipRequest, ChipResponse, ErrorCode, Request, Response, SshHost, KIND_REQUEST, KIND_RESPONSE,
};
use cahoots_custodian::CircleId;
use cahoots_hal::Link;
use common::{cable, FakeBoard, FakeLink};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use rand_core::OsRng;
use sha2::{Digest, Sha256, Sha512};
use ssh_encoding::Encode;
use ssh_key::{Algorithm, EcdsaCurve, HashAlg, PrivateKey, SshSig};

// ---- fixtures ----

const SESSION: &[u8] = &[0x5a; 32];

fn enc(x: &impl Encode) -> Vec<u8> {
    let mut v = Vec::new();
    x.encode(&mut v).unwrap();
    v
}

/// The lines of the screen before the last: the prompt, when the last is the page again.
fn prompt(badge: &mut Badge<FakeBoard>) -> Vec<String> {
    let screens = &badge.board().screens;
    screens[screens.len() - 2].clone()
}

/// The userauth request OpenSSH asks an agent to sign, with `method`, `alg` and `key`.
fn userauth_with(
    session_id: &[u8],
    user: &str,
    service: &str,
    method: &str,
    alg: &str,
    key: &[u8],
) -> Vec<u8> {
    let mut v = Vec::new();
    session_id.encode(&mut v).unwrap();
    50u8.encode(&mut v).unwrap();
    user.encode(&mut v).unwrap();
    service.encode(&mut v).unwrap();
    method.encode(&mut v).unwrap();
    1u8.encode(&mut v).unwrap();
    alg.encode(&mut v).unwrap();
    key.encode(&mut v).unwrap();
    v
}

fn userauth(session_id: &[u8], user: &str, key: &[u8]) -> Vec<u8> {
    userauth_with(
        session_id,
        user,
        "ssh-connection",
        "publickey",
        "ecdsa-sha2-nistp256",
        key,
    )
}

/// A seat key's public key as OpenSSH writes it, built with ssh-key from the SEC1 point.
fn seat_blob(public: &SeatPublic) -> Vec<u8> {
    let point = p256::PublicKey::from_sec1_bytes(&public.0).unwrap();
    let uncompressed = point.to_encoded_point(false);
    let key = ssh_key::public::EcdsaPublicKey::from_sec1_bytes(uncompressed.as_bytes()).unwrap();
    enc(&ssh_key::public::KeyData::from(key))
}

fn soft_seat() -> SeatKey {
    SeatKey::from_seed(&[7; 32]).unwrap()
}

/// The binding a laptop learns from `session-bind@openssh.com`, signed by a host key of `alg`.
fn bound(alg: Algorithm, session_id: &[u8], name: &str) -> SshHost {
    let key = PrivateKey::random(&mut OsRng, alg).unwrap();
    let sig: ssh_key::Signature = signature::Signer::sign(&key, session_id);
    SshHost {
        host_key: key.public_key().to_bytes().unwrap(),
        session_id: session_id.to_vec(),
        host_sig: enc(&sig),
        name: name.into(),
        user: "root".into(),
    }
}

fn p256_alg() -> Algorithm {
    Algorithm::Ecdsa {
        curve: EcdsaCurve::NistP256,
    }
}

/// An RSA host key and a signature, written by hand: the badge does not check RSA.
fn rsa_bound(session_id: &[u8], name: &str) -> SshHost {
    let mut key = Vec::new();
    "ssh-rsa".encode(&mut key).unwrap();
    [1u8, 0, 1].as_slice().encode(&mut key).unwrap();
    let n: Vec<u8> = core::iter::once(0)
        .chain((0..256).map(|i| (i as u8) | 0x80))
        .collect();
    n.as_slice().encode(&mut key).unwrap();
    let mut sig = Vec::new();
    "rsa-sha2-256".encode(&mut sig).unwrap();
    [0x42u8; 256].as_slice().encode(&mut sig).unwrap();
    SshHost {
        host_key: key,
        session_id: session_id.to_vec(),
        host_sig: sig,
        name: name.into(),
        user: "git".into(),
    }
}

const COMMIT: &str = "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
author Ada Lovelace <ada@example.com> 1700000000 +0000\n\
committer Ada Lovelace <ada@example.com> 1700000000 +0000\n\
\n\
Fix the parser for long names\n\
\n\
A body that is not shown.\n";

fn sshsig(content: &[u8], alg: HashAlg) -> Vec<u8> {
    SshSig::signed_data("git", alg, content).unwrap()
}

// ---- the parser ----

#[test]
fn parses_userauth_request() {
    let blob = seat_blob(&soft_seat().public());
    let data = userauth(SESSION, "git", &blob);
    match ssh::parse(&data) {
        Ok(Signed::Login {
            user,
            session_id,
            key,
            host_key,
        }) => {
            assert_eq!(user, "git");
            assert_eq!(session_id, SESSION);
            assert_eq!(key, blob);
            assert_eq!(host_key, None);
        }
        other => panic!("not a login: {other:?}"),
    }
    // The badge writes the seat key in SSH form exactly as ssh-key does.
    assert_eq!(ssh::public_key_blob(&soft_seat().public()), blob);
}

#[test]
fn parses_a_host_bound_userauth_request() {
    let blob = seat_blob(&soft_seat().public());
    let host = bound(Algorithm::Ed25519, SESSION, "github.com");
    let mut data = userauth_with(
        SESSION,
        "git",
        "ssh-connection",
        "publickey-hostbound-v00@openssh.com",
        "ecdsa-sha2-nistp256",
        &blob,
    );
    host.host_key.as_slice().encode(&mut data).unwrap();
    match ssh::parse(&data) {
        Ok(Signed::Login { host_key, .. }) => assert_eq!(host_key, Some(host.host_key)),
        other => panic!("not a login: {other:?}"),
    }
}

#[test]
fn parses_sshsig_envelope() {
    for (alg, name) in [(HashAlg::Sha512, "sha512"), (HashAlg::Sha256, "sha256")] {
        let data = sshsig(COMMIT.as_bytes(), alg);
        let signed = ssh::parse(&data).unwrap();
        match &signed {
            Signed::Sig {
                namespace,
                hash_alg,
                hash,
            } => {
                assert_eq!(namespace, "git");
                assert_eq!(hash_alg, name);
                let want: Vec<u8> = match alg {
                    HashAlg::Sha512 => Sha512::digest(COMMIT).to_vec(),
                    _ => Sha256::digest(COMMIT).to_vec(),
                };
                assert_eq!(*hash, want);
            }
            other => panic!("not a signature envelope: {other:?}"),
        }
        assert!(ssh::check_content(&signed, COMMIT.as_bytes()));
    }
}

#[test]
fn rejects_garbage_with_unparseable() {
    let blob = seat_blob(&soft_seat().public());
    let good = userauth(SESSION, "git", &blob);
    let env = sshsig(COMMIT.as_bytes(), HashAlg::Sha512);
    let mut bad: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"hello".to_vec(),
        vec![0xff; 64],
        // Lengths that run past the end, or overflow.
        vec![0xff, 0xff, 0xff, 0xff, 1, 2, 3],
        userauth_with(
            SESSION,
            "git",
            "ssh-userauth",
            "publickey",
            "ecdsa-sha2-nistp256",
            &blob,
        ),
        userauth_with(
            SESSION,
            "git",
            "ssh-connection",
            "password",
            "ecdsa-sha2-nistp256",
            &blob,
        ),
        userauth_with(
            SESSION,
            "git",
            "ssh-connection",
            "publickey",
            "ssh-ed25519",
            &blob,
        ),
        // A user name that is not UTF-8.
        {
            let mut v = Vec::new();
            SESSION.encode(&mut v).unwrap();
            50u8.encode(&mut v).unwrap();
            [0xffu8, 0xfe].as_slice().encode(&mut v).unwrap();
            v.extend_from_slice(&good[4 + SESSION.len() + 1 + 4 + 3..]);
            v
        },
        // Trailing bytes after a whole request.
        [good.clone(), vec![0]].concat(),
        [env.clone(), vec![0]].concat(),
        // A request with the "has signature" flag clear.
        {
            let mut v = good.clone();
            let at = 4 + SESSION.len() + 1 + 7 + 18 + 13;
            assert_eq!(v[at], 1);
            v[at] = 0;
            v
        },
        // An unknown hash algorithm, and a hash of the wrong length.
        {
            let mut v = b"SSHSIG".to_vec();
            "git".encode(&mut v).unwrap();
            "".encode(&mut v).unwrap();
            "md5".encode(&mut v).unwrap();
            [0u8; 16].as_slice().encode(&mut v).unwrap();
            v
        },
        {
            let mut v = b"SSHSIG".to_vec();
            "git".encode(&mut v).unwrap();
            "".encode(&mut v).unwrap();
            "sha512".encode(&mut v).unwrap();
            [0u8; 32].as_slice().encode(&mut v).unwrap();
            v
        },
        // An empty namespace.
        {
            let mut v = b"SSHSIG".to_vec();
            "".encode(&mut v).unwrap();
            "".encode(&mut v).unwrap();
            "sha512".encode(&mut v).unwrap();
            [0u8; 64].as_slice().encode(&mut v).unwrap();
            v
        },
    ];
    // Every proper prefix of a good request.
    for n in 0..good.len() {
        bad.push(good[..n].to_vec());
    }
    for n in 0..env.len() {
        bad.push(env[..n].to_vec());
    }
    for b in &bad {
        assert!(ssh::parse(b).is_err(), "parsed {b:02x?}");
    }
    // Nothing panics, whatever the bytes.
    let mut x: u64 = 0x1234_5678_9abc_def0;
    for len in 0..400 {
        let bytes: Vec<u8> = (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect();
        let _ = ssh::parse(&bytes);
        let mut flipped = good.clone();
        flipped[len % good.len()] ^= (x as u8) | 1;
        let _ = ssh::parse(&flipped);
    }
}

// ---- the session binding ----

#[test]
fn bound_host_verified_for_ed25519_and_p256() {
    for alg in [Algorithm::Ed25519, p256_alg()] {
        let host = bound(alg.clone(), SESSION, "github.com");
        assert_eq!(
            ssh::check_host(&host, SESSION),
            HostCheck::Verified("github.com".into()),
            "{alg:?}"
        );
    }
}

#[test]
fn mismatched_session_id_is_host_not_named() {
    for alg in [Algorithm::Ed25519, p256_alg()] {
        // The binding is honest, but for another session.
        let host = bound(alg.clone(), &[0x11; 32], "github.com");
        assert_eq!(ssh::check_host(&host, SESSION), HostCheck::None, "{alg:?}");
        // The session id matches, but the signature is over something else.
        let mut forged = bound(alg.clone(), &[0x11; 32], "github.com");
        forged.session_id = SESSION.to_vec();
        assert_eq!(
            ssh::check_host(&forged, SESSION),
            HostCheck::None,
            "{alg:?}"
        );
        // Garbage in place of the host key.
        let mut junk = bound(alg, SESSION, "github.com");
        junk.host_key = vec![0xff; 40];
        assert_eq!(ssh::check_host(&junk, SESSION), HostCheck::None);
    }
    let shown = screens::ssh_login("git", &HostCheck::None, "fakes");
    assert!(shown.lines.iter().any(|l| l == "host not named"));
    assert!(!shown.lines.iter().any(|l| l.contains("github.com")));
}

#[test]
fn rsa_host_is_host_not_named() {
    let host = rsa_bound(SESSION, "github.com");
    assert_eq!(ssh::check_host(&host, SESSION), HostCheck::None);
}

#[test]
fn content_hash_mismatch_refused() {
    let data = sshsig(COMMIT.as_bytes(), HashAlg::Sha512);
    let signed = ssh::parse(&data).unwrap();
    assert!(ssh::check_content(&signed, COMMIT.as_bytes()));
    assert!(!ssh::check_content(
        &signed,
        b"tree 00\nauthor Mallory <m@x> 1 +0000\n\nother\n"
    ));
    assert!(!ssh::check_content(&signed, b""));
    // A login is never matched with content.
    let login = ssh::parse(&userauth(SESSION, "git", &seat_blob(&soft_seat().public()))).unwrap();
    assert!(!ssh::check_content(&login, COMMIT.as_bytes()));
}

// ---- the screens ----

#[test]
fn login_screen_lines() {
    let s = screens::ssh_login("git", &HostCheck::Verified("github.com".into()), "fakes");
    assert_eq!(
        s.lines,
        [
            "ssh login",
            "as git",
            "to github.com (laptop)",
            "fakes",
            "refuse  A+B  allow  refuse"
        ]
    );
    let s = screens::ssh_login("git", &HostCheck::None, "fakes");
    assert_eq!(
        s.lines,
        [
            "ssh login",
            "as git",
            "host not named",
            "fakes",
            "refuse  A+B  allow  refuse"
        ]
    );
    // Long values are cut to fit; the mark stays.
    let long = "x".repeat(100);
    let s = screens::ssh_login(&long, &HostCheck::Verified(long.clone()), &long);
    // The user and the circle get one row each, the host two.
    for (l, w) in s.lines[1..4].iter().zip([21, 48, 24]) {
        assert!(l.chars().count() <= w, "too wide: {l}");
    }
    assert!(s.lines[1].starts_with("as xxx") && s.lines[1].ends_with("..."));
    assert!(s.lines[2].starts_with("to xxx") && s.lines[2].ends_with("... (laptop)"));
}

#[test]
fn git_screen_lines() {
    let s = screens::git_signature("Ada Lovelace", "Fix the parser", true, "fakes");
    assert_eq!(
        s.lines,
        [
            "sign a git commit",
            "Ada Lovelace",
            "Fix the parser",
            "fakes",
            "refuse  A+B  allow  refuse"
        ]
    );
    let s = screens::git_signature("Ada Lovelace", "Fix the parser", false, "fakes");
    assert_eq!(
        s.lines,
        [
            "sign a git commit",
            "Ada Lovelace (laptop)",
            "Fix the parser (laptop)",
            "fakes",
            "refuse  A+B  allow  refuse"
        ]
    );
    let s = screens::git_signature(&"a".repeat(60), &"s".repeat(60), false, "fakes");
    assert!(s.lines[1].ends_with("... (laptop)") && s.lines[1].chars().count() <= 48);
    assert!(s.lines[2].ends_with("... (laptop)") && s.lines[2].chars().count() <= 48);
}

// ---- the Serve path ----

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

/// A badge serving its down link, the laptop end, the circle id, and the seat public key. With
/// `chip`, the seat key is in a SoftChip in slot 0.
fn serving(chip: bool) -> (Badge<FakeBoard>, FakeLink, CircleId, SeatPublic) {
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
        soft_seat().public()
    };
    let (laptop, down) = cable();
    board.links = Some((down, FakeLink::open()));
    let mut badge = Badge::new(board);
    badge.boot();
    badge.idle_step();
    press(&mut badge, Press::Right);
    press(&mut badge, Press::Right);
    press(&mut badge, Press::Both);
    assert!(badge.board().shows("serving"));
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

/// Send `op` and step once: the answer must come back without a prompt.
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

fn signature_of(r: Response) -> Vec<u8> {
    match r {
        Response::Chip {
            result: ChipResponse::Signature { signature },
        } => signature,
        other => panic!("not a signature: {other:?}"),
    }
}

/// Check an SSH-wire signature over `data` with the seat key, both with ssh-key and as raw
/// P-256 over SHA-256(data).
fn assert_verifies(public: &SeatPublic, data: &[u8], sig: &[u8]) {
    let parsed = ssh_key::Signature::try_from(sig).expect("an ssh signature");
    assert_eq!(parsed.algorithm(), p256_alg());
    let key = ssh_key::PublicKey::from_bytes(&seat_blob(public)).unwrap();
    signature::Verifier::verify(&key, data, &parsed).expect("verifies with the seat key");
}

#[test]
fn serve_ssh_sign_end_to_end() {
    let (mut badge, mut laptop, id, public) = serving(true);
    let data = userauth(SESSION, "git", &seat_blob(&public));
    let host = bound(Algorithm::Ed25519, SESSION, "github.com");
    let op = ChipRequest::SshSign {
        data: data.clone(),
        host: Some(host.clone()),
        content: None,
    };
    let r = ask(&mut badge, &mut laptop, id, op.clone(), Press::Both);
    let prompt = prompt(&mut badge);
    assert_eq!(
        prompt,
        [
            "ssh login",
            "as git",
            "to github.com (laptop)",
            "fakes",
            "refuse  A+B  allow  refuse"
        ]
    );
    assert_verifies(&public, &data, &signature_of(r));

    // A single press refuses.
    for gesture in [Press::Left, Press::Right] {
        let r = ask(&mut badge, &mut laptop, id, op.clone(), gesture);
        assert_eq!(
            r,
            Response::Error {
                code: ErrorCode::Denied
            }
        );
    }
}

#[test]
fn serve_signs_a_git_commit_whose_content_matches() {
    let (mut badge, mut laptop, id, public) = serving(true);
    let data = sshsig(COMMIT.as_bytes(), HashAlg::Sha512);
    let op = ChipRequest::SshSign {
        data: data.clone(),
        host: None,
        content: Some(COMMIT.as_bytes().to_vec()),
    };
    let r = ask(&mut badge, &mut laptop, id, op, Press::Both);
    let prompt = prompt(&mut badge);
    assert_eq!(
        prompt,
        [
            "sign a git commit",
            "Ada Lovelace",
            "Fix the parser for long names",
            "fakes",
            "refuse  A+B  allow  refuse"
        ]
    );
    let sig = signature_of(r);
    assert_verifies(&public, &data, &sig);
    // The signature git would write verifies as an SSHSIG over the commit.
    let key = ssh_key::PublicKey::from_bytes(&seat_blob(&public)).unwrap();
    let sshsig = SshSig::new(
        key.key_data().clone(),
        "git",
        HashAlg::Sha512,
        ssh_key::Signature::try_from(sig.as_slice()).unwrap(),
    )
    .unwrap();
    key.verify("git", COMMIT.as_bytes(), &sshsig).unwrap();
}

#[test]
fn serve_refuses_git_without_content_without_asking() {
    // The badge cannot describe a commit it was not sent: no screen, no signature.
    let (mut badge, mut laptop, id, _) = serving(true);
    let op = ChipRequest::SshSign {
        data: sshsig(COMMIT.as_bytes(), HashAlg::Sha512),
        host: None,
        content: None,
    };
    let r = ask_now(&mut badge, &mut laptop, id, op);
    assert_eq!(r, Response::Error { code: ErrorCode::Unparseable });
}

#[test]
fn commit_text_with_two_authors_is_refused() {
    // git shows the first author; a second one must not let the badge show another name.
    let two = "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
author Ada Lovelace <ada@example.com> 1700000000 +0000\n\
author Mallory <m@example.com> 1700000000 +0000\n\
committer Ada Lovelace <ada@example.com> 1700000000 +0000\n\
\n\
Fix the parser\n";
    assert_eq!(ssh::describe_commit(two.as_bytes()), None);
    assert!(ssh::describe_commit(COMMIT.as_bytes()).is_some());
    let (mut badge, mut laptop, id, _) = serving(true);
    let op = ChipRequest::SshSign {
        data: sshsig(two.as_bytes(), HashAlg::Sha512),
        host: None,
        content: Some(two.as_bytes().to_vec()),
    };
    let r = ask_now(&mut badge, &mut laptop, id, op);
    assert_eq!(r, Response::Error { code: ErrorCode::Unparseable });
}

#[test]
fn serve_refuses_what_it_cannot_describe_without_asking() {
    let (mut badge, mut laptop, id, public) = serving(true);
    let blob = seat_blob(&public);
    let mismatched = ChipRequest::SshSign {
        data: sshsig(COMMIT.as_bytes(), HashAlg::Sha512),
        host: None,
        content: Some(b"tree 00\nauthor Mallory <m@x> 1 +0000\n\nother\n".to_vec()),
    };
    let garbage = ChipRequest::SshSign {
        data: b"sign this hash please".to_vec(),
        host: None,
        content: None,
    };
    // Someone else's key in the request.
    let other_key = ChipRequest::SshSign {
        data: userauth(
            SESSION,
            "git",
            &seat_blob(&SeatKey::from_seed(&[3; 32]).unwrap().public()),
        ),
        host: None,
        content: None,
    };
    let other_namespace = ChipRequest::SshSign {
        data: SshSig::signed_data("file", HashAlg::Sha512, b"x").unwrap(),
        host: None,
        content: None,
    };
    for op in [mismatched, garbage, other_key, other_namespace] {
        let r = ask_now(&mut badge, &mut laptop, id, op);
        assert_eq!(
            r,
            Response::Error {
                code: ErrorCode::Unparseable
            }
        );
    }
    // The good request still works after them.
    let data = userauth(SESSION, "git", &blob);
    let op = ChipRequest::SshSign {
        data: data.clone(),
        host: None,
        content: None,
    };
    assert_verifies(
        &public,
        &data,
        &signature_of(ask(&mut badge, &mut laptop, id, op, Press::Both)),
    );
}

#[test]
fn serve_mismatched_binding_shows_host_not_named() {
    let (mut badge, mut laptop, id, public) = serving(true);
    let data = userauth(SESSION, "git", &seat_blob(&public));
    let op = ChipRequest::SshSign {
        data,
        host: Some(bound(p256_alg(), &[0x11; 32], "github.com")),
        content: None,
    };
    ask(&mut badge, &mut laptop, id, op, Press::Left);
    let prompt = prompt(&mut badge);
    assert_eq!(prompt[2], "host not named");
}

#[test]
fn a_badge_without_a_chip_signs_with_its_software_key() {
    let (mut badge, mut laptop, id, public) = serving(false);
    let data = userauth(SESSION, "git", &seat_blob(&public));
    let op = ChipRequest::SshSign {
        data: data.clone(),
        host: None,
        content: None,
    };
    let sig = signature_of(ask(&mut badge, &mut laptop, id, op, Press::Both));
    assert_verifies(&public, &data, &sig);

    match ask_now(&mut badge, &mut laptop, id, ChipRequest::Describe) {
        Response::Chip {
            result:
                ChipResponse::Description {
                    public: p,
                    in_chip,
                    can_attest,
                    ..
                },
        } => {
            assert_eq!(p, public.0.to_vec());
            assert!(!in_chip && !can_attest);
        }
        other => panic!("not a description: {other:?}"),
    }
    let r = ask_now(
        &mut badge,
        &mut laptop,
        id,
        ChipRequest::Attest { freshness: [1; 16] },
    );
    assert_eq!(
        r,
        Response::Error {
            code: ErrorCode::NoChip
        }
    );
}

#[test]
fn a_badge_with_a_chip_describes_and_attests_its_seat_key() {
    let (mut badge, mut laptop, id, public) = serving(true);
    match ask_now(&mut badge, &mut laptop, id, ChipRequest::Describe) {
        Response::Chip {
            result:
                ChipResponse::Description {
                    public: p,
                    in_chip,
                    can_attest,
                    curve,
                },
        } => {
            assert_eq!(p, public.0.to_vec());
            assert!(in_chip && can_attest);
            assert_eq!(curve, cahoots_badge::apps::CURVE_P256);
        }
        other => panic!("not a description: {other:?}"),
    }
    match ask_now(
        &mut badge,
        &mut laptop,
        id,
        ChipRequest::Attest { freshness: [1; 16] },
    ) {
        Response::Chip {
            result: ChipResponse::Attested { attested },
        } => assert_eq!(attested.freshness, [1; 16]),
        other => panic!("not an attestation: {other:?}"),
    }
}

#[test]
fn other_chip_requests_are_answered_without_a_prompt() {
    let (mut badge, mut laptop, id, _) = serving(true);
    for op in [
        ChipRequest::Card { card: vec![1] },
        ChipRequest::GrantSign { grant: vec![1] },
        ChipRequest::SealKey {
            label: "card".into(),
        },
    ] {
        let r = ask_now(&mut badge, &mut laptop, id, op);
        assert_eq!(
            r,
            Response::Error {
                code: ErrorCode::NoChip
            }
        );
    }
    // A circle the badge does not hold.
    for op in [
        ChipRequest::Describe,
        ChipRequest::SshSign {
            data: vec![],
            host: None,
            content: None,
        },
    ] {
        let r = ask_now(&mut badge, &mut laptop, [0xee; 16], op);
        assert_eq!(
            r,
            Response::Error {
                code: ErrorCode::UnknownCircle
            }
        );
    }
}

/// A chip that signs with the wrong key: every signature is over a different digest.
struct Lying(SoftChip);

impl ChipOps for Lying {
    fn version(&mut self) -> Result<cahoots_chip::Version, cahoots_chip::ChipError> {
        self.0.version()
    }
    fn random(&mut self, n: usize) -> Result<Vec<u8>, cahoots_chip::ChipError> {
        self.0.random(n)
    }
    fn generate(&mut self, id: KeyId, curve: Curve) -> Result<Vec<u8>, cahoots_chip::ChipError> {
        self.0.generate(id, curve)
    }
    fn public(&mut self, id: KeyId) -> Result<Vec<u8>, cahoots_chip::ChipError> {
        self.0.public(id)
    }
    fn delete(&mut self, id: KeyId) -> Result<(), cahoots_chip::ChipError> {
        self.0.delete(id)
    }
    fn exists(&mut self, id: KeyId) -> Result<bool, cahoots_chip::ChipError> {
        self.0.exists(id)
    }
    fn ids(&mut self) -> Result<Vec<KeyId>, cahoots_chip::ChipError> {
        self.0.ids()
    }
    fn sign_p256(&mut self, id: KeyId, _: &[u8; 32]) -> Result<[u8; 64], cahoots_chip::ChipError> {
        self.0.sign_p256(id, &[0x77; 32])
    }
    fn sign_ed25519(&mut self, id: KeyId, m: &[u8]) -> Result<[u8; 64], cahoots_chip::ChipError> {
        self.0.sign_ed25519(id, m)
    }
    fn attest(
        &mut self,
        id: KeyId,
        f: &[u8; 16],
    ) -> Result<cahoots_chip::Attested, cahoots_chip::ChipError> {
        self.0.attest(id, f)
    }
    fn put_hmac_key(&mut self, id: KeyId, key: &[u8; 32]) -> Result<(), cahoots_chip::ChipError> {
        self.0.put_hmac_key(id, key)
    }
    fn hmac(&mut self, id: KeyId, data: &[u8]) -> Result<[u8; 32], cahoots_chip::ChipError> {
        self.0.hmac(id, data)
    }
    fn put_counter(&mut self, id: KeyId, value: u64) -> Result<(), cahoots_chip::ChipError> {
        self.0.put_counter(id, value)
    }
    fn counter_next(&mut self, id: KeyId) -> Result<u64, cahoots_chip::ChipError> {
        self.0.counter_next(id)
    }
}

#[test]
fn a_signature_that_does_not_verify_is_never_returned() {
    use cahoots_badge::apps::BadgeApps;
    use cahoots_custodian::ChipApps;
    let (board, _, _) = FakeBoard::with_circle();
    let mut circle = board.circle.unwrap();
    circle.chip_slot = Some(0);
    circle.seat_key = [0; 32];
    let mut chip = Lying(SoftChip::with_seed([5; 32]));
    let p = chip
        .generate(KeyId::new(KeyKind::Seat, 0), Curve::P256)
        .unwrap();
    let data = userauth(SESSION, "git", &seat_blob(&SeatPublic::parse(&p).unwrap()));
    let mut apps = BadgeApps::new(core::slice::from_ref(&circle), Some(&mut chip));
    let r = apps.ssh_sign(&circle.circle_id, &data, None, None);
    assert_eq!(r, Err(ErrorCode::ChipError));
}

#[test]
fn a_seat_key_missing_from_the_chip_is_a_chip_error_before_asking() {
    let (mut badge, mut laptop, id, public) = serving(true);
    let chip = badge.board().chip.as_mut().unwrap();
    chip.delete(KeyId::new(KeyKind::Seat, 0)).unwrap();
    let op = ChipRequest::SshSign {
        data: userauth(SESSION, "git", &seat_blob(&public)),
        host: None,
        content: None,
    };
    let r = ask_now(&mut badge, &mut laptop, id, op);
    assert_eq!(
        r,
        Response::Error {
            code: ErrorCode::ChipError
        }
    );
}
