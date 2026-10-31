//! The `gate` feature: a gate token or a proof by a key with a read grant to read a channel; a
//! verified post grant, signature, budget and quorum to post; verified policies and grants.
#![cfg(feature = "gate")]

use cahoots_channel::grant::{
    issue_delegated, issue_root, sign_delegated, sign_root, verify_chain,
};
use cahoots_channel::keys::{kid_of, EpochId};
use cahoots_channel::message::{self, body_digest, Header, MessageType};
use cahoots_channel::object::channel_id_for;
use cahoots_channel::quorum::{approval_for, PROFILE_GOVERN, PROFILE_POST};
use cahoots_channel::{policy, Action, ChannelId, ChannelObject, Policy, Preset};
use cahoots_core::cose;
use cahoots_core::manifest::{Manifest, SealedManifest, Seat, MANIFEST_AAD};
use cahoots_core::params::CircleParams;
use cahoots_core::seat::SeatKey;
use cahoots_objects::assurance;
use cahoots_objects::authz::proof_sign;
use cahoots_station::limits::Limits;
use cahoots_station::server;
use reqwest::{Client, RequestBuilder, Response};
use serde_json::Value;
use sha2::{Digest, Sha256};

const CIRCLE: [u8; 16] = [7; 16];
const NONCE: [u8; 16] = [9; 16];

/// The deterministic filler `crates/vectors` uses.
fn fill(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 56) as u8
        })
        .collect()
}

fn seat_key(i: u8) -> SeatKey {
    SeatKey::from_seed(&fill(350 + u64::from(i), 32).try_into().unwrap()).unwrap()
}

fn member_key(i: u64) -> SeatKey {
    SeatKey::from_seed(&fill(900 + i, 32).try_into().unwrap()).unwrap()
}

fn manifest() -> Manifest {
    let params = CircleParams::new(3, 192, "channel", false).unwrap();
    Manifest {
        circle_id: CIRCLE,
        seats: (0..3)
            .map(|i| Seat {
                seat: i,
                key: seat_key(i).public(),
                tier: 0,
                evidence: Vec::new(),
            })
            .collect(),
        floor: Manifest::default_floor(&params),
        params,
        genesis: 0,
    }
}

fn sealed_manifest() -> Vec<u8> {
    let body = manifest().encode();
    let attestations = (0..3u8)
        .map(|i| cose::sign1(&seat_key(i), &[i], &body, MANIFEST_AAD, true))
        .collect();
    SealedManifest { body, attestations }.encode()
}

fn channel_id() -> ChannelId {
    channel_id_for(&CIRCLE, &NONCE)
}

fn cbor_array(items: &[&[u8]]) -> Vec<u8> {
    let mut e = minicbor::Encoder::new(Vec::new());
    e.array(items.len() as u64).unwrap();
    for i in items {
        e.bytes(i).unwrap();
    }
    e.into_writer()
}

/// `[signed_policy, [approval, ...]]`.
fn policy_body(signed: &[u8], approvals: &[Vec<u8>]) -> Vec<u8> {
    let mut e = minicbor::Encoder::new(Vec::new());
    e.array(2).unwrap().bytes(signed).unwrap();
    e.array(approvals.len() as u64).unwrap();
    for a in approvals {
        e.bytes(a).unwrap();
    }
    e.into_writer()
}

fn preset(p: Preset, log: u64, nonce: u8) -> Policy {
    Policy::preset(p, CIRCLE, channel_id(), [0x33; 32], 3, log, [nonce; 16])
}

/// A COSE_Sign1 by `seat` over the canonical map `{0: channel, 1: second, 2: reason?}`.
fn seat_statement(seat: u8, domain: &[u8], second: &[u8], reason: Option<&str>) -> Vec<u8> {
    let mut e = minicbor::Encoder::new(Vec::new());
    e.map(if reason.is_some() { 3 } else { 2 }).unwrap();
    e.u8(0).unwrap().bytes(&channel_id()).unwrap();
    e.u8(1).unwrap().bytes(second).unwrap();
    if let Some(r) = reason {
        e.u8(2).unwrap().str(r).unwrap();
    }
    cose::sign1(&seat_key(seat), &[seat], &e.into_writer(), domain, false)
}

async fn start_at(path: &std::path::Path) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let limits = Limits {
        posts_per_minute: None,
        reads_per_minute: None,
        trust_forwarded: false,
        channel_posts_per_minute: None,
    };
    let app = server::app(server::State::open_with(path, limits, None).unwrap())
        .into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn send(r: RequestBuilder) -> (u16, Value) {
    let r: Response = r.send().await.unwrap();
    let status = r.status().as_u16();
    let v = r.json().await.unwrap_or(Value::Null);
    (status, v)
}

struct Station {
    base: String,
    c: Client,
    dir: tempfile::TempDir,
}

impl Station {
    fn ch(&self) -> String {
        format!("{}/v1/channels/{}", self.base, hex::encode(channel_id()))
    }

    fn path(&self, rest: &str) -> String {
        format!("/v1/channels/{}{rest}", hex::encode(channel_id()))
    }

    async fn post_policy(&self, signed: &[u8], approvals: &[Vec<u8>]) -> (u16, Value) {
        send(
            self.c
                .post(format!("{}/policy", self.ch()))
                .body(policy_body(signed, approvals)),
        )
        .await
    }

    async fn post_grant(&self, links: &[&[u8]]) -> (u16, Value) {
        send(
            self.c
                .post(format!("{}/grants", self.ch()))
                .body(cbor_array(links)),
        )
        .await
    }

    async fn post_message(&self, wire: Vec<u8>) -> (u16, Value) {
        send(self.c.post(format!("{}/messages", self.ch())).body(wire)).await
    }

    async fn nonce(&self) -> [u8; 16] {
        let (s, v) = send(self.c.get(format!("{}/v1/nonce", self.base))).await;
        assert_eq!(s, 200);
        hex::decode(v["nonce"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap()
    }

    /// GET `path_and_query` with a fresh proof by `key`.
    async fn get_with_proof(&self, path_and_query: &str, key: &SeatKey) -> (u16, Value) {
        let nonce = self.nonce().await;
        let r = proved(
            self.c.get(format!("{}{path_and_query}", self.base)),
            "GET",
            path_and_query,
            key,
            &nonce,
        );
        send(r).await
    }

    async fn add_gate(&self, token: &[u8; 32]) {
        let signed = seat_statement(1, b"cahoots/gate/v1", token, None);
        let (s, v) = send(self.c.post(format!("{}/gates", self.ch())).body(signed)).await;
        assert_eq!(s, 201, "{v}");
    }
}

/// The headers of a read by key: the proof over `SHA-256(method " " path "?" query)` and the kid.
fn proved(
    r: RequestBuilder,
    method: &str,
    path_and_query: &str,
    key: &SeatKey,
    nonce: &[u8; 16],
) -> RequestBuilder {
    let signed = if path_and_query.contains('?') {
        format!("{method} {path_and_query}")
    } else {
        format!("{method} {path_and_query}?")
    };
    let hash: [u8; 32] = Sha256::digest(signed.as_bytes()).into();
    let proof = proof_sign(key, &hash, nonce);
    use base64::Engine;
    r.header(
        "Cahoots-Proof",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(proof),
    )
    .header("Cahoots-Kid", hex::encode(kid_of(&key.public())))
}

/// A station with the manifest, the channel object and a first policy `p`, signed by seat 0.
async fn setup(p: Preset) -> Station {
    let dir = tempfile::tempdir().unwrap();
    let base = start_at(&dir.path().join("station.db")).await;
    let c = Client::new();
    let url = format!("{base}/v1/circles/{}/manifest", hex::encode(CIRCLE));
    assert_eq!(send(c.put(url).body(sealed_manifest())).await.0, 201);
    let object = ChannelObject {
        circle_id: CIRCLE,
        channel_id: channel_id(),
        name: "general".into(),
        created_log: 5,
        policy_id: [0x11; 32],
        nonce: NONCE,
    };
    let (s, _) = send(
        c.post(format!("{base}/v1/channels"))
            .body(object.sign(&seat_key(0), 0)),
    )
    .await;
    assert_eq!(s, 201);
    let st = Station { base, c, dir };
    let (s, v) = st
        .post_policy(&preset(p, 6, 4).sign(&seat_key(0), 0), &[])
        .await;
    assert_eq!(s, 201, "{v}");
    st
}

/// A root grant from seat 1 to `member_key(member)`.
fn root_grant(member: u64, actions: &[Action], budget: u32, not_after: u64, cti: u8) -> Vec<u8> {
    let claims = issue_root(
        &CIRCLE,
        &channel_id(),
        1,
        &member_key(member).public(),
        actions,
        budget,
        not_after,
        1,
        [cti; 16],
    );
    sign_root(&claims, &seat_key(1), 1)
}

/// A message by `key`, sealed under a key the relay never sees.
fn message_by(key: &SeatKey, counter: u32, t: MessageType, approvals: &[Vec<u8>]) -> Vec<u8> {
    let plaintext = b"hello";
    let header = Header {
        channel_id: channel_id(),
        epoch_id: EpochId {
            channel_id: channel_id(),
            epoch: 1,
        },
        kid: kid_of(&key.public()),
        counter,
        len: plaintext.len() as u32,
        msg_type: t,
    };
    message::seal_sign(&header, &[0x5a; 32], plaintext, approvals, key)
}

fn text_by(key: &SeatKey, counter: u32) -> Vec<u8> {
    message_by(key, counter, MessageType::Text, &[])
}

fn reason(v: &(u16, Value)) -> (u16, Option<&str>, Option<&str>) {
    (v.0, v.1["error"].as_str(), v.1["reason"].as_str())
}

/// A version 2 (chat) body for the circle, with one byte of plaintext.
fn chat() -> Vec<u8> {
    let mut m = vec![2u8];
    m.extend_from_slice(&CIRCLE);
    m.extend_from_slice(&[1, 2, 3, 4, 5, 0]);
    m.extend_from_slice(&0u32.to_le_bytes());
    m.extend_from_slice(&1u32.to_le_bytes());
    m.push(0);
    m.extend_from_slice(&[0u8; 17]);
    m
}

#[tokio::test]
async fn read_needs_a_gate_token_or_a_proof() {
    let st = setup(Preset::GroupChat).await;
    assert_eq!(
        st.post_grant(&[&root_grant(0, &[Action::Read], 0, 100, 1)])
            .await
            .0,
        201
    );
    let kid = hex::encode([0u8; 32]);
    let gated = [
        "/messages".to_string(),
        "/messages?after=0".to_string(),
        "/head".to_string(),
        "/deposits".to_string(),
        format!("/grants?kid={kid}"),
        format!("/grants/{}", hex::encode([1u8; 16])),
        "/requests".to_string(),
    ];
    for p in &gated {
        let (s, v) = send(st.c.get(format!("{}{p}", st.ch()))).await;
        assert_eq!((s, v["error"].as_str()), (401, Some("gate")), "{p}");
    }
    let token = [0x77u8; 32];
    let wrong = [0x78u8; 32];
    for p in &gated {
        let (s, _) = send(
            st.c.get(format!("{}{p}", st.ch()))
                .header("Cahoots-Gate", hex::encode(token)),
        )
        .await;
        assert_eq!(s, 401, "{p}");
    }
    st.add_gate(&token).await;
    for p in &gated[..6] {
        let (s, v) = send(
            st.c.get(format!("{}{p}", st.ch()))
                .header("Cahoots-Gate", hex::encode(token)),
        )
        .await;
        assert_eq!(s, 200, "{p}: {v}");
        let (s, _) = send(
            st.c.get(format!("{}{p}", st.ch()))
                .header("Cahoots-Gate", hex::encode(wrong)),
        )
        .await;
        assert_eq!(s, 401, "{p}");
    }
    // The join queue is for keys that admit; a gate token does not open it.
    let (s, _) = send(
        st.c.get(format!("{}/requests", st.ch()))
            .header("Cahoots-Gate", hex::encode(token)),
    )
    .await;
    assert_eq!(s, 401);
    // Anyone may ask to join.
    let (s, _) = send(
        st.c.post(format!("{}/requests", st.ch()))
            .body(b"let me in".to_vec()),
    )
    .await;
    assert_eq!(s, 201);

    // A token for this channel does not open another stored channel.
    let other_object = ChannelObject {
        circle_id: CIRCLE,
        channel_id: channel_id_for(&CIRCLE, &[10; 16]),
        name: "other".into(),
        created_log: 5,
        policy_id: [0x11; 32],
        nonce: [10; 16],
    };
    let (s, _) = send(
        st.c.post(format!("{}/v1/channels", st.base))
            .body(other_object.sign(&seat_key(0), 0)),
    )
    .await;
    assert_eq!(s, 201);
    let other = format!(
        "{}/v1/channels/{}",
        st.base,
        hex::encode(other_object.channel_id)
    );
    for p in &gated {
        let (s, v) = send(
            st.c.get(format!("{other}{p}"))
                .header("Cahoots-Gate", hex::encode(token)),
        )
        .await;
        assert_eq!((s, v["error"].as_str()), (401, Some("gate")), "{p}");
    }

    // An unknown channel is 404.
    let unknown = format!("{}/v1/channels/{}", st.base, hex::encode([0x42u8; 16]));
    for p in &gated {
        let (s, v) = send(
            st.c.get(format!("{unknown}{p}"))
                .header("Cahoots-Gate", hex::encode(token)),
        )
        .await;
        assert_eq!((s, v["error"].as_str()), (404, Some("channel")), "{p}");
    }
}

#[tokio::test]
async fn read_with_a_valid_proof_succeeds() {
    let st = setup(Preset::GroupChat).await;
    let reader = member_key(0);
    let (s, _) = st
        .post_grant(&[&root_grant(0, &[Action::Read], 0, 100, 1)])
        .await;
    assert_eq!(s, 201);

    let path = st.path("/messages?after=0");
    let (s, v) = st.get_with_proof(&path, &reader).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["entries"], serde_json::json!([]));
    let (s, _) = st.get_with_proof(&st.path("/head"), &reader).await;
    assert_eq!(s, 200);
    let kid = hex::encode(kid_of(&reader.public()));
    let (s, v) = st
        .get_with_proof(&st.path(&format!("/grants?kid={kid}")), &reader)
        .await;
    assert_eq!(s, 200);
    assert_eq!(v["grants"].as_array().unwrap().len(), 1);

    // A nonce is used once.
    let nonce = st.nonce().await;
    let get = || {
        proved(
            st.c.get(format!("{}{path}", st.base)),
            "GET",
            &path,
            &reader,
            &nonce,
        )
    };
    assert_eq!(send(get()).await.0, 200);
    assert_eq!(send(get()).await.0, 401);
    // A nonce the relay never gave.
    let r = proved(
        st.c.get(format!("{}{path}", st.base)),
        "GET",
        &path,
        &reader,
        &[3; 16],
    );
    assert_eq!(send(r).await.0, 401);
    // A proof over another request.
    let nonce = st.nonce().await;
    let r = proved(
        st.c.get(format!("{}{path}", st.base)),
        "GET",
        &st.path("/messages?after=1"),
        &reader,
        &nonce,
    );
    assert_eq!(send(r).await.0, 401);
    // A key with no grant, and a key whose grant does not carry read.
    let (s, v) = st.get_with_proof(&path, &member_key(5)).await;
    assert_eq!((s, v["error"].as_str()), (401, Some("gate")));
    let (s, _) = st
        .post_grant(&[&root_grant(1, &[Action::Post], 0, 100, 2)])
        .await;
    assert_eq!(s, 201);
    assert_eq!(st.get_with_proof(&path, &member_key(1)).await.0, 401);
    // An expired read grant.
    let (s, _) = st
        .post_grant(&[&root_grant(2, &[Action::Read], 0, 0, 3)])
        .await;
    assert_eq!(s, 201);
    assert_eq!(st.get_with_proof(&path, &member_key(2)).await.0, 401);
}

#[tokio::test]
async fn read_of_requests_needs_admit() {
    let st = setup(Preset::GroupChat).await;
    let (s, _) = send(
        st.c.post(format!("{}/requests", st.ch()))
            .body(b"let me in".to_vec()),
    )
    .await;
    assert_eq!(s, 201);
    assert_eq!(
        st.post_grant(&[&root_grant(0, &[Action::Read], 0, 100, 1)])
            .await
            .0,
        201
    );
    assert_eq!(
        st.post_grant(&[&root_grant(1, &[Action::Read, Action::Admit], 0, 100, 2)])
            .await
            .0,
        201
    );
    let path = st.path("/requests");
    let (s, v) = st.get_with_proof(&path, &member_key(0)).await;
    assert_eq!((s, v["error"].as_str()), (401, Some("gate")));
    let (s, v) = st.get_with_proof(&path, &member_key(1)).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["requests"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn post_without_a_grant_is_403_missing() {
    let st = setup(Preset::GroupChat).await;
    let r = st.post_message(text_by(&member_key(0), 0)).await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("missing")));
    // A message signed by another key than its kid's grantee.
    assert_eq!(
        st.post_grant(&[&root_grant(0, &[Action::Read, Action::Post], 0, 100, 1)])
            .await
            .0,
        201
    );
    let mut forged = text_by(&member_key(1), 0);
    forged[37..69].copy_from_slice(&kid_of(&member_key(0).public()));
    let r = st.post_message(forged).await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("signature")));
    // Nothing was stored.
    let (_, v) = send(st.c.get(format!("{}/v1/head", st.base))).await;
    assert_eq!(v["seq"], Value::Null);
}

#[tokio::test]
async fn post_with_a_read_only_grant_is_403_action() {
    let st = setup(Preset::GroupChat).await;
    assert_eq!(
        st.post_grant(&[&root_grant(0, &[Action::Read], 0, 100, 1)])
            .await
            .0,
        201
    );
    let r = st.post_message(text_by(&member_key(0), 0)).await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("action")));
}

#[tokio::test]
async fn post_after_expiry_is_403_expired() {
    let st = setup(Preset::GroupChat).await;
    assert_eq!(
        st.post_grant(&[&root_grant(0, &[Action::Read, Action::Post], 0, 2, 1)])
            .await
            .0,
        201
    );
    // A circle entry at seq 0, so the channel's two entries are at seq 1 and 2.
    let circle = format!("{}/v1/circles/{}/messages", st.base, hex::encode(CIRCLE));
    assert_eq!(send(st.c.post(circle).body(chat())).await.0, 201);
    let key = member_key(0);
    let (s, v) = st.post_message(text_by(&key, 0)).await;
    assert_eq!((s, v["seq"].as_u64()), (201, Some(1)), "{v}");
    let (s, v) = st.post_message(text_by(&key, 1)).await;
    assert_eq!((s, v["seq"].as_u64()), (201, Some(2)), "{v}");
    // The channel head is now seq 2, the grant's not_after.
    let r = st.post_message(text_by(&key, 2)).await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("expired")));
}

#[tokio::test]
async fn post_over_budget_is_403_budget() {
    let st = setup(Preset::GroupChat).await;
    assert_eq!(
        st.post_grant(&[&root_grant(0, &[Action::Read, Action::Post], 2, 100, 1)])
            .await
            .0,
        201
    );
    let key = member_key(0);
    assert_eq!(st.post_message(text_by(&key, 0)).await.0, 201);
    assert_eq!(st.post_message(text_by(&key, 1)).await.0, 201);
    let r = st.post_message(text_by(&key, 2)).await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("budget")));

    // The count is rebuilt from the log when the station opens again.
    let base = start_at(&st.dir.path().join("station.db")).await;
    let url = format!("{base}/v1/channels/{}/messages", hex::encode(channel_id()));
    let r = send(st.c.post(url).body(text_by(&key, 3))).await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("budget")));
}

#[tokio::test]
async fn revoked_chain_is_403_revoked_and_a_child_of_a_revoked_parent_too() {
    let st = setup(Preset::GroupChat).await;
    let root = root_grant(0, &[Action::Read, Action::Post, Action::Admit], 0, 100, 1);
    let parent = verify_chain(&[&root], &manifest(), &channel_id()).unwrap();
    let child = sign_delegated(
        &issue_delegated(
            &root,
            &parent,
            &member_key(1).public(),
            &[Action::Read, Action::Post],
            0,
            50,
            [2; 16],
        ),
        &member_key(0),
    );
    assert_eq!(st.post_grant(&[&root]).await.0, 201);
    assert_eq!(st.post_grant(&[&root, &child]).await.0, 201);
    assert_eq!(st.post_message(text_by(&member_key(0), 0)).await.0, 201);
    assert_eq!(st.post_message(text_by(&member_key(1), 0)).await.0, 201);

    // Revoke the child: only the child is refused.
    let statement = seat_statement(0, b"cahoots/revoke/v1", &[2; 16], Some("left"));
    let (s, _) = send(
        st.c.post(format!("{}/revocations", st.ch()))
            .body(cbor_array(&[&[2; 16], &statement])),
    )
    .await;
    assert_eq!(s, 201);
    let r = st.post_message(text_by(&member_key(1), 1)).await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("revoked")));
    assert_eq!(st.post_message(text_by(&member_key(0), 1)).await.0, 201);

    // Revoke the parent: its own key is refused too.
    let statement = seat_statement(0, b"cahoots/revoke/v1", &[1; 16], Some("left"));
    let (s, _) = send(
        st.c.post(format!("{}/revocations", st.ch()))
            .body(cbor_array(&[&[1; 16], &statement])),
    )
    .await;
    assert_eq!(s, 201);
    let r = st.post_message(text_by(&member_key(0), 2)).await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("revoked")));
}

#[tokio::test]
async fn a_child_of_a_revoked_parent_is_403_revoked() {
    let st = setup(Preset::GroupChat).await;
    let root = root_grant(0, &[Action::Read, Action::Post, Action::Admit], 0, 100, 1);
    let parent = verify_chain(&[&root], &manifest(), &channel_id()).unwrap();
    let child = sign_delegated(
        &issue_delegated(
            &root,
            &parent,
            &member_key(1).public(),
            &[Action::Read, Action::Post],
            0,
            50,
            [2; 16],
        ),
        &member_key(0),
    );
    assert_eq!(st.post_grant(&[&root, &child]).await.0, 201);
    let statement = seat_statement(0, b"cahoots/revoke/v1", &[1; 16], Some("left"));
    let (s, _) = send(
        st.c.post(format!("{}/revocations", st.ch()))
            .body(cbor_array(&[&[1; 16], &statement])),
    )
    .await;
    assert_eq!(s, 201);
    // Only the parent is on the list; the child's chain holds it.
    let r = st.post_message(text_by(&member_key(1), 0)).await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("revoked")));
}

#[tokio::test]
async fn announcement_short_of_quorum_is_403_quorum() {
    let st = setup(Preset::AnnouncementsQuorum(2)).await;
    let poster = member_key(0);
    assert_eq!(
        st.post_grant(&[&root_grant(0, &[Action::Read, Action::Post], 0, 100, 1)])
            .await
            .0,
        201
    );
    let body = {
        let wire = message_by(&poster, 0, MessageType::Announcement, &[]);
        message::split(&wire).unwrap().0.to_vec()
    };
    let approve = |seat: u8, digest: [u8; 32]| {
        approval_for(
            &seat_key(seat),
            seat,
            &CIRCLE,
            PROFILE_POST,
            Vec::new(),
            digest,
            &[0x11; 32],
            0,
            assurance::PRESENT,
            [seat; 16],
        )
    };
    let digest = body_digest(&body);
    let one = vec![approve(0, digest)];
    let r = st
        .post_message(message_by(&poster, 0, MessageType::Announcement, &one))
        .await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("quorum")));
    // The same seat twice is one seat.
    let twice = vec![approve(0, digest), approve(0, digest)];
    let r = st
        .post_message(message_by(&poster, 0, MessageType::Announcement, &twice))
        .await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("quorum")));
    // Approvals of another body do not count.
    let other = vec![approve(0, digest), approve(1, [0; 32])];
    let r = st
        .post_message(message_by(&poster, 0, MessageType::Announcement, &other))
        .await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("quorum")));
    let two = vec![approve(0, digest), approve(1, digest)];
    let (s, v) = st
        .post_message(message_by(&poster, 0, MessageType::Announcement, &two))
        .await;
    assert_eq!(s, 201, "{v}");
}

#[tokio::test]
async fn control_needs_govern() {
    let st = setup(Preset::GroupChat).await;
    let root = root_grant(0, &[Action::Read, Action::Post, Action::Admit], 0, 100, 1);
    let parent = verify_chain(&[&root], &manifest(), &channel_id()).unwrap();
    let child = sign_delegated(
        &issue_delegated(
            &root,
            &parent,
            &member_key(1).public(),
            &[Action::Read, Action::Post],
            0,
            50,
            [2; 16],
        ),
        &member_key(0),
    );
    assert_eq!(st.post_grant(&[&root]).await.0, 201);
    assert_eq!(st.post_grant(&[&root, &child]).await.0, 201);
    let r = st
        .post_message(message_by(&member_key(1), 0, MessageType::Control, &[]))
        .await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("action")));
    // A root grant without govern may post text but not control.
    let r = st
        .post_message(message_by(&member_key(0), 0, MessageType::Control, &[]))
        .await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("action")));
    assert_eq!(st.post_message(text_by(&member_key(0), 0)).await.0, 201);
    // An admin seat's own grant carries all four actions.
    let all = [Action::Read, Action::Post, Action::Admit, Action::Govern];
    assert_eq!(
        st.post_grant(&[&root_grant(2, &all, 0, 100, 3)]).await.0,
        201
    );
    let (s, v) = st
        .post_message(message_by(&member_key(2), 0, MessageType::Control, &[]))
        .await;
    assert_eq!(s, 201, "{v}");
}

#[tokio::test]
async fn a_replayed_message_is_409_and_spends_no_budget() {
    let st = setup(Preset::GroupChat).await;
    assert_eq!(
        st.post_grant(&[&root_grant(0, &[Action::Read, Action::Post], 2, 100, 1)])
            .await
            .0,
        201
    );
    let key = member_key(0);
    let wire = text_by(&key, 0);
    assert_eq!(st.post_message(wire.clone()).await.0, 201);
    let (s, v) = st.post_message(wire.clone()).await;
    assert_eq!((s, v["error"].as_str()), (409, Some("replay")));
    // The same kid, epoch and counter with another body is a replay too.
    let (s, _) = st
        .post_message(message_by(&key, 0, MessageType::Announcement, &[]))
        .await;
    assert_eq!(s, 409);
    // The budget of 2 still has one post left.
    assert_eq!(st.post_message(text_by(&key, 1)).await.0, 201);
    let r = st.post_message(text_by(&key, 2)).await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("budget")));
    // The set is rebuilt when the station opens again.
    let base = start_at(&st.dir.path().join("station.db")).await;
    let url = format!("{base}/v1/channels/{}/messages", hex::encode(channel_id()));
    let (s, v) = send(st.c.post(url).body(wire)).await;
    assert_eq!((s, v["error"].as_str()), (409, Some("replay")));
}

#[tokio::test]
async fn nonces_are_capped() {
    let st = setup(Preset::GroupChat).await;
    for i in 0..10_000 {
        let (s, _) = send(st.c.get(format!("{}/v1/nonce", st.base))).await;
        assert_eq!(s, 200, "nonce {i}");
    }
    let (s, v) = send(st.c.get(format!("{}/v1/nonce", st.base))).await;
    assert_eq!((s, v["error"].as_str()), (429, Some("rate")));
}

#[tokio::test]
async fn policy_with_govern_quorum_2_needs_two_approvals() {
    // Three seats: the preset's govern quorum is 2.
    let st = setup(Preset::GroupChat).await;
    let next = preset(Preset::Closed, 7, 5).sign(&seat_key(0), 0);
    let digest: [u8; 32] = Sha256::digest(&next).into();
    let approve = |seat: u8| {
        approval_for(
            &seat_key(seat),
            seat,
            &CIRCLE,
            PROFILE_GOVERN,
            Vec::new(),
            digest,
            &[0x11; 32],
            0,
            assurance::PRESENT,
            [seat; 16],
        )
    };
    let r = st.post_policy(&next, &[]).await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("quorum")));
    let r = st.post_policy(&next, &[approve(0)]).await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("quorum")));
    let (s, v) = st.post_policy(&next, &[approve(0), approve(1)]).await;
    assert_eq!(s, 201, "{v}");
    assert_eq!(v["policy_id"], hex::encode(policy::id(&next)));
}

#[tokio::test]
async fn grant_chains_are_verified_when_stored() {
    let st = setup(Preset::GroupChat).await;
    // A root signed by a key that is not seat 1's.
    let claims = issue_root(
        &CIRCLE,
        &channel_id(),
        1,
        &member_key(0).public(),
        &[Action::Read],
        0,
        100,
        0,
        [1; 16],
    );
    let forged = sign_root(&claims, &member_key(3), 1);
    let r = st.post_grant(&[&forged]).await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("chain")));
    // A child that widens its parent.
    let root = root_grant(0, &[Action::Read, Action::Admit], 0, 100, 2);
    let parent = verify_chain(&[&root], &manifest(), &channel_id()).unwrap();
    let wider = sign_delegated(
        &issue_delegated(
            &root,
            &parent,
            &member_key(1).public(),
            &[Action::Read, Action::Post],
            0,
            50,
            [3; 16],
        ),
        &member_key(0),
    );
    let r = st.post_grant(&[&root, &wider]).await;
    assert_eq!(reason(&r), (403, Some("grant"), Some("chain")));
    assert_eq!(st.post_grant(&[&root]).await.0, 201);
}

#[tokio::test]
async fn a_retired_gate_token_is_refused() {
    let st = setup(Preset::GroupChat).await;
    let token = [0x77u8; 32];
    st.add_gate(&token).await;
    let read = || {
        st.c.get(format!("{}/messages", st.ch()))
            .header("Cahoots-Gate", hex::encode(token))
    };
    assert_eq!(send(read()).await.0, 200);
    let hash: [u8; 32] = Sha256::digest(token).into();
    let retire = seat_statement(1, b"cahoots/gate-retire/v1", &token, None);
    let (s, _) = send(
        st.c.delete(format!("{}/gates/{}", st.ch(), hex::encode(hash)))
            .body(retire),
    )
    .await;
    assert_eq!(s, 204);
    let (s, v) = send(read()).await;
    assert_eq!((s, v["error"].as_str()), (401, Some("gate")));
}
