//! Rebuilds `crates/vectors/objects.json` from its inputs and compares every derived field.
#![cfg(feature = "std")]

use cahoots_core::manifest::Manifest;
use cahoots_core::seat::{SeatKey, SeatPublic};
use cahoots_objects::grant::{key_id, verify_chain, Grant, Grantor};
use cahoots_objects::{fact_nonce, Approval, Fact, Receipt};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn vectors() -> Value {
    serde_json::from_str(include_str!("../../vectors/objects.json")).unwrap()
}

fn unhex(v: &Value) -> Vec<u8> {
    hex::decode(v.as_str().unwrap()).unwrap()
}

fn arr<const N: usize>(v: &Value) -> [u8; N] {
    unhex(v).try_into().unwrap()
}

fn uint(v: &Value) -> u64 {
    v.as_u64().unwrap()
}

fn key(v: &Value) -> SeatKey {
    SeatKey::from_seed(&arr(v)).unwrap()
}

fn public(v: &Value) -> SeatPublic {
    SeatPublic::parse(&unhex(v)).unwrap()
}

fn opt_digest(v: &Value) -> Option<[u8; 32]> {
    (!v.is_null()).then(|| arr(v))
}

fn grant(i: &Value) -> Grant {
    Grant {
        circle_id: arr(&i["circle_id"]),
        grantor: match &i["grantor"] {
            Value::Number(n) => Grantor::Seat(n.as_u64().unwrap() as u8),
            k => Grantor::Key(public(k)),
        },
        grantee: public(&i["grantee"]),
        purposes: i["purposes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| uint(p) as u8)
            .collect(),
        budget: uint(&i["budget"]) as u32,
        not_after: uint(&i["not_after"]),
        depth: uint(&i["depth"]) as u8,
        parent: opt_digest(&i["parent"]),
        nonce: arr(&i["nonce"]),
    }
}

/// The payload, the signature and the JSON projection match the file.
fn check(v: &Value, payload: Vec<u8>, signed: &[u8], json: Value) {
    assert_eq!(hex::encode(payload), v["payload"].as_str().unwrap());
    assert_eq!(hex::encode(signed), v["signed"].as_str().unwrap());
    assert_eq!(json, v["json"]);
}

fn manifest(v: &Value) -> Manifest {
    let m = Manifest::decode(&unhex(&v["manifest"])).unwrap();
    assert_eq!(hex::encode(m.circle_id), v["circle_id"].as_str().unwrap());
    for (i, seed) in v["seat_seeds"].as_array().unwrap().iter().enumerate() {
        let pk = key(seed).public();
        assert_eq!(&pk, m.seat_key(i as u8).unwrap());
        assert_eq!(pk, public(&v["seat_publics"][i]));
    }
    m
}

#[test]
fn the_grants_are_reproduced() {
    let v = vectors();
    let m = manifest(&v);
    let seat = uint(&v["seat"]) as u8;
    let seat_key = key(&v["seat_seeds"][usize::from(seat)]);
    let delegate = key(&v["delegate_seed"]);
    assert_eq!(delegate.public(), public(&v["delegate_public"]));
    assert_eq!(key(&v["leaf_seed"]).public(), public(&v["leaf_public"]));

    let root = grant(&v["root_grant"]["inputs"]);
    let root_signed = root.sign(&seat_key, &[seat]);
    check(
        &v["root_grant"],
        root.encode(),
        &root_signed,
        root.to_json(),
    );

    let delegated = grant(&v["delegated_grant"]["inputs"]);
    assert_eq!(
        delegated.parent,
        Some(Sha256::digest(&root_signed).into()),
        "the parent is the hash of the root's signed bytes"
    );
    let delegated_signed = delegated.sign(&delegate, &key_id(&delegate.public()));
    check(
        &v["delegated_grant"],
        delegated.encode(),
        &delegated_signed,
        delegated.to_json(),
    );
    assert_eq!(
        verify_chain(&[&root_signed, &delegated_signed], &m).unwrap(),
        delegated
    );
}

#[test]
fn the_jwk_is_the_uncompressed_grantee_key() {
    let v = vectors();
    let grantee = unhex(&v["root_grant"]["inputs"]["grantee"]);
    let jwk = &v["root_grant"]["json"]["cnf"]["jwk"];
    let b64 = |s: &Value| {
        // base64url without padding, decoded by hand to keep the dependency list short.
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut bits = 0u32;
        let mut n = 0;
        let mut out = Vec::new();
        for c in s.as_str().unwrap().bytes() {
            bits = bits << 6 | alphabet.iter().position(|&a| a == c).unwrap() as u32;
            n += 6;
            if n >= 8 {
                n -= 8;
                out.push((bits >> n) as u8);
            }
        }
        out
    };
    let x = b64(&jwk["x"]);
    let y = b64(&jwk["y"]);
    assert_eq!(x.len(), 32);
    assert_eq!(y.len(), 32);
    // Recompress: the prefix carries the parity of y.
    let mut compressed = vec![if y[31] & 1 == 0 { 0x02 } else { 0x03 }];
    compressed.extend_from_slice(&x);
    assert_eq!(compressed, grantee);
}

#[test]
fn the_approval_is_reproduced() {
    let v = vectors();
    let m = manifest(&v);
    let i = &v["approval"]["inputs"];
    let a = Approval {
        circle_id: arr(&i["circle_id"]),
        seat: uint(&i["seat"]) as u8,
        profile: i["profile"].as_str().unwrap().into(),
        fields: i["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| (f[0].as_str().unwrap().into(), f[1].as_str().unwrap().into()))
            .collect(),
        request_digest: arr(&i["request_digest"]),
        policy_id: unhex(&i["policy_id"]),
        verdict: i["verdict"].as_bool().unwrap(),
        assurance: uint(&i["assurance"]) as u8,
        log: uint(&i["log"]),
        nonce: arr(&i["nonce"]),
    };
    let signed = a.sign(&key(&v["seat_seeds"][usize::from(a.seat)]), &[a.seat]);
    check(&v["approval"], a.encode(), &signed, a.to_json());
    assert_eq!(Approval::verify(&signed, &m).unwrap(), a);
}

#[test]
fn the_fact_is_reproduced() {
    let v = vectors();
    let m = manifest(&v);
    let i = &v["fact"]["inputs"];
    let f = Fact {
        circle_id: arr(&i["circle_id"]),
        seat: uint(&i["seat"]) as u8,
        module: i["module"].as_str().unwrap().into(),
        reading: unhex(&i["reading"]),
        counter: uint(&i["counter"]),
        nonce: arr(&i["nonce"]),
        log: uint(&i["log"]),
        attested: i["attested"].as_bool().unwrap(),
        evidence: unhex(&i["evidence"]),
    };
    assert_eq!(f.nonce, fact_nonce(&f.circle_id, f.log));
    let signed = f.sign(&key(&v["seat_seeds"][usize::from(f.seat)]), &[f.seat]);
    check(&v["fact"], f.encode(), &signed, f.to_json());
    assert_eq!(Fact::verify(&signed, &m).unwrap(), f);
}

#[test]
fn the_receipt_is_reproduced() {
    let v = vectors();
    let m = manifest(&v);
    let i = &v["receipt"]["inputs"];
    let r = Receipt {
        circle_id: arr(&i["circle_id"]),
        seat: uint(&i["seat"]) as u8,
        slice: uint(&i["slice"]) as u32,
        offset: uint(&i["offset"]) as u32,
        len: uint(&i["len"]) as u32,
        purpose: uint(&i["purpose"]) as u8,
        grant: opt_digest(&i["grant"]),
        log: uint(&i["log"]),
    };
    assert_eq!(
        r.grant,
        Some(Sha256::digest(unhex(&v["delegated_grant"]["signed"])).into())
    );
    let signed = r.sign(&key(&v["seat_seeds"][usize::from(r.seat)]), &[r.seat]);
    check(&v["receipt"], r.encode(), &signed, r.to_json());
    assert_eq!(Receipt::verify(&signed, &m).unwrap(), r);
}

#[test]
fn the_v2_grants_are_reproduced() {
    use cahoots_objects::cwt::Claims;
    let v = vectors();
    let m = manifest(&v);
    let seat = uint(&v["seat"]) as u8;
    let seat_key = key(&v["seat_seeds"][usize::from(seat)]);
    let delegate = key(&v["delegate_seed"]);

    let root = grant(&v["grant_v2_root"]["inputs"]);
    assert_eq!(root, grant(&v["root_grant"]["inputs"]), "the same root");
    let root_signed = root.sign_v2(&seat_key, &[seat]);
    check(
        &v["grant_v2_root"],
        root.encode_v2(),
        &root_signed,
        root.to_v2().to_json(),
    );
    let delegated = grant(&v["grant_v2_delegated"]["inputs"]);
    assert_eq!(delegated.parent, Some(Sha256::digest(&root_signed).into()));
    let delegated_signed = delegated.sign_v2(&delegate, &key_id(&delegate.public()));
    check(
        &v["grant_v2_delegated"],
        delegated.encode_v2(),
        &delegated_signed,
        delegated.to_v2().to_json(),
    );
    assert_eq!(
        verify_chain(&[&root_signed, &delegated_signed], &m).unwrap(),
        delegated
    );
    let claims = Claims::decode(&unhex(&v["grant_v2_root"]["payload"])).unwrap();
    assert_eq!(Grant::from_v2(&claims).unwrap(), root);
}

#[test]
fn the_card_is_reproduced() {
    use cahoots_objects::card::{self, Card};
    let v = vectors();
    let m = manifest(&v);
    let i = &v["card"]["inputs"];
    let c = Card {
        circle_id: arr(&i["circle_id"]),
        seat: uint(&i["seat"]) as u8,
        name: i["name"].as_str().unwrap().into(),
        handles: i["handles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| (h[0].as_str().unwrap().into(), h[1].as_str().unwrap().into()))
            .collect(),
        endpoints: i["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (uint(&e[0]) as u8, e[1].as_str().unwrap().into()))
            .collect(),
        log: uint(&i["log"]),
    };
    let signed = c
        .sign(&key(&v["seat_seeds"][usize::from(c.seat)]), &[c.seat])
        .unwrap();
    check(&v["card"], c.encode().unwrap(), &signed, c.to_json());
    assert_eq!(Card::verify(&signed, &m).unwrap(), c);

    let s = &v["card"]["seal"];
    let root: [u8; 32] = arr(&s["root"]);
    let seal_key = cahoots_core::root::seal_key(&root, b"seal/card", &c.circle_id);
    assert_eq!(seal_key.to_vec(), unhex(&s["key"]));
    let sealed = card::seal_with_nonce(&signed, &seal_key, arr(&s["nonce"]));
    assert_eq!(hex::encode(&sealed), s["sealed"].as_str().unwrap());
    assert_eq!(card::open(&sealed, &seal_key).unwrap(), signed);
}

#[test]
fn the_sign_grant_and_its_proof_are_reproduced() {
    use cahoots_objects::authz::{proof_sign, proof_verify, Action, SignGrant};
    use cahoots_objects::cwt::Claims;
    let v = vectors();
    let m = manifest(&v);
    let seat = uint(&v["seat"]) as u8;
    let seat_key = key(&v["seat_seeds"][usize::from(seat)]);
    let g = &v["sign_grant"];
    let i = &g["inputs"];
    let agent = key(&g["agent_seed"]);
    let want = SignGrant {
        circle_id: arr(&i["circle_id"]),
        seat: uint(&i["seat"]) as u8,
        grantee: public(&i["grantee"]),
        cti: arr(&i["cti"]),
        actions: vec![Action::GitCommit],
        author: i["author"].as_str().unwrap().into(),
        locations: i["locations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap().into())
            .collect(),
        agent: i["agent"].as_str().unwrap().into(),
        budget: uint(&i["budget"]) as u32,
        exp: Some(uint(&i["exp"])),
        base: uint(&i["base"]),
    };
    assert_eq!(want.grantee, agent.public());
    assert_eq!(want.exp, Some(1_764_374_400));

    let payload = unhex(&g["payload"]);
    assert_eq!(want.claims().encode(), payload);
    assert_eq!(SignGrant::from_claims(&Claims::decode(&payload).unwrap()).unwrap(), want);
    let signed = want.sign(&seat_key, seat);
    assert_eq!(hex::encode(&signed), g["signed"].as_str().unwrap());
    let got = SignGrant::verify(&signed, m.seat_key(seat).unwrap(), &m.circle_id, seat).unwrap();
    assert_eq!(got, want);

    let hash: [u8; 32] = arr(&g["proof"]["data_hash"]);
    let nonce: [u8; 16] = arr(&g["proof"]["nonce"]);
    let proof = unhex(&g["proof"]["signed"]);
    assert_eq!(proof_sign(&agent, &hash, &nonce), proof);
    assert!(proof_verify(&proof, &got.grantee, &hash, &nonce));
    let mut other = nonce;
    other[0] ^= 1;
    assert!(!proof_verify(&proof, &got.grantee, &hash, &other));
}
