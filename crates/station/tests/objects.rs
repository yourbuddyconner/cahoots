//! The `objects` feature: manifests, channel objects, policies, grants, revocations, deposits,
//! join requests and gate tokens.
#![cfg(feature = "objects")]

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use cahoots_channel::deposit::seal_for_link;
use cahoots_channel::grant::{
    issue_delegated, issue_root, sign_delegated, sign_root, verify_chain,
};
use cahoots_channel::keys::EpochId;
use cahoots_channel::link::LinkSecret;
use cahoots_channel::object::channel_id_for;
use cahoots_channel::{policy, Action, ChannelId, ChannelObject, Policy, Preset};
use cahoots_core::cose;
use cahoots_core::manifest::{Manifest, SealedManifest, Seat, MANIFEST_AAD};
use cahoots_core::params::CircleParams;
use cahoots_core::seat::SeatKey;
use cahoots_objects::grant::key_id;
use cahoots_station::limits::Limits;
use cahoots_station::server;
use reqwest::{Client, Response};
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

fn member(i: u64) -> SeatKey {
    SeatKey::from_seed(&fill(900 + i, 32).try_into().unwrap()).unwrap()
}

fn manifest_with(circle_id: [u8; 16], name: &str) -> Manifest {
    let params = CircleParams::new(3, 192, name, false).unwrap();
    Manifest {
        circle_id,
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

fn manifest() -> Manifest {
    manifest_with(CIRCLE, "channel")
}

fn seal(m: &Manifest) -> Vec<u8> {
    let body = m.encode();
    let attestations = (0..3u8)
        .map(|i| cose::sign1(&seat_key(i), &[i], &body, MANIFEST_AAD, true))
        .collect();
    SealedManifest { body, attestations }.encode()
}

fn channel_id() -> ChannelId {
    channel_id_for(&CIRCLE, &NONCE)
}

fn object(created_log: u64) -> ChannelObject {
    ChannelObject {
        circle_id: CIRCLE,
        channel_id: channel_id(),
        name: "general".into(),
        created_log,
        policy_id: [0x11; 32],
        nonce: NONCE,
    }
}

fn cbor_array(items: &[&[u8]]) -> Vec<u8> {
    let mut e = minicbor::Encoder::new(Vec::new());
    e.array(items.len() as u64).unwrap();
    for i in items {
        e.bytes(i).unwrap();
    }
    e.into_writer()
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

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

async fn start_with(limits: Limits, cap: Option<u64>) -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("station.db");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = server::app(server::State::open_with(&path, limits, cap).unwrap())
        .into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), dir)
}

async fn start() -> (String, tempfile::TempDir) {
    start_with(
        Limits {
            posts_per_minute: None,
            reads_per_minute: None,
            trust_forwarded: false,
            channel_posts_per_minute: None,
        },
        None,
    )
    .await
}

async fn send(r: reqwest::RequestBuilder) -> (u16, Value) {
    let r: Response = r.send().await.unwrap();
    let status = r.status().as_u16();
    let v = r.json().await.unwrap_or(Value::Null);
    (status, v)
}

/// A station holding the manifest and the channel object at `created_log` 5.
async fn with_channel() -> (String, tempfile::TempDir, Client) {
    let (base, dir) = start().await;
    let c = Client::new();
    let url = format!("{base}/v1/circles/{}/manifest", hex::encode(CIRCLE));
    assert_eq!(send(c.put(url).body(seal(&manifest()))).await.0, 201);
    let (s, _) = send(
        c.post(format!("{base}/v1/channels"))
            .body(object(5).sign(&seat_key(0), 0)),
    )
    .await;
    assert_eq!(s, 201);
    (base, dir, c)
}

fn ch(base: &str) -> String {
    format!("{base}/v1/channels/{}", hex::encode(channel_id()))
}

#[tokio::test]
async fn manifest_put_then_get() {
    let (base, _dir) = start().await;
    let c = Client::new();
    let url = format!("{base}/v1/circles/{}/manifest", hex::encode(CIRCLE));
    let r = c.get(&url).send().await.unwrap();
    assert_eq!(r.status(), 404);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"], "manifest");

    let sealed = seal(&manifest());
    assert_eq!(send(c.put(&url).body(sealed.clone())).await.0, 201);
    let r = c.get(&url).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers()["content-type"].to_str().unwrap(),
        "application/octet-stream"
    );
    assert_eq!(r.bytes().await.unwrap().to_vec(), sealed);

    // The same bytes again are accepted; another manifest for the circle is refused.
    assert_eq!(send(c.put(&url).body(sealed.clone())).await.0, 201);
    let (s, v) = send(c.put(&url).body(seal(&manifest_with(CIRCLE, "other")))).await;
    assert_eq!((s, v["error"].as_str()), (409, Some("exists")));

    // A manifest for another circle than the path's.
    let other = format!("{base}/v1/circles/{}/manifest", hex::encode([8u8; 16]));
    let (s, v) = send(c.put(&other).body(sealed.clone())).await;
    assert_eq!((s, v["error"].as_str()), (400, Some("circle_id")));

    // An attestation that does not verify.
    let m = manifest_with([8; 16], "channel");
    let body = m.encode();
    let mut attestations: Vec<Vec<u8>> = (0..3u8)
        .map(|i| cose::sign1(&seat_key(i), &[i], &body, MANIFEST_AAD, true))
        .collect();
    attestations[2] = cose::sign1(&seat_key(1), &[2], &body, MANIFEST_AAD, true);
    let bad = SealedManifest { body, attestations }.encode();
    let (s, v) = send(c.put(&other).body(bad)).await;
    assert_eq!((s, v["error"].as_str()), (400, Some("malformed")));

    let (s, v) = send(c.put(format!("{base}/v1/circles/zz/manifest")).body(sealed)).await;
    assert_eq!((s, v["error"].as_str()), (400, Some("circle_id")));
}

#[tokio::test]
async fn channel_post_then_get() {
    let (base, _dir) = start().await;
    let c = Client::new();
    let signed = object(5).sign(&seat_key(0), 0);
    let (s, v) = send(c.post(format!("{base}/v1/channels")).body(signed.clone())).await;
    assert_eq!((s, v["error"].as_str()), (404, Some("manifest")));

    let sealed = seal(&manifest());
    let url = format!("{base}/v1/circles/{}/manifest", hex::encode(CIRCLE));
    assert_eq!(send(c.put(url).body(sealed.clone())).await.0, 201);
    let (s, v) = send(c.post(format!("{base}/v1/channels")).body(signed.clone())).await;
    assert_eq!(s, 201);
    assert_eq!(v["channel_id"], hex::encode(channel_id()));

    let (s, v) = send(c.get(ch(&base))).await;
    assert_eq!(s, 200);
    assert_eq!(v["channel"], b64(&signed));
    assert_eq!(v["policy"], Value::Null);
    assert_eq!(v["manifest"], b64(&sealed));

    // A later object replaces; an earlier or equal one is stale.
    let later = object(9).sign(&seat_key(1), 1);
    assert_eq!(
        send(c.post(format!("{base}/v1/channels")).body(later.clone()))
            .await
            .0,
        201
    );
    let (s, v) = send(
        c.post(format!("{base}/v1/channels"))
            .body(object(7).sign(&seat_key(0), 0)),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (409, Some("stale")));
    let (_, v) = send(c.get(ch(&base))).await;
    assert_eq!(v["channel"], b64(&later));

    // A channel id that is not derived from the circle and the nonce.
    let mut bad = object(5);
    bad.channel_id = [0x42; 16];
    let (s, v) = send(
        c.post(format!("{base}/v1/channels"))
            .body(bad.sign(&seat_key(0), 0)),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (400, Some("channel_id")));

    // Signed by a key that is not the named seat's.
    let (s, v) = send(
        c.post(format!("{base}/v1/channels"))
            .body(object(20).sign(&member(0), 0)),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (403, Some("signature")));

    let (s, v) = send(c.post(format!("{base}/v1/channels")).body(vec![1, 2, 3])).await;
    assert_eq!((s, v["error"].as_str()), (400, Some("malformed")));

    let (s, v) = send(c.get(format!("{base}/v1/channels/{}", hex::encode([3u8; 16])))).await;
    assert_eq!((s, v["error"].as_str()), (404, Some("channel")));
    let (s, v) = send(c.get(format!("{base}/v1/channels/abc"))).await;
    assert_eq!((s, v["error"].as_str()), (400, Some("channel_id")));
}

#[tokio::test]
async fn policy_post_then_get() {
    let (base, _dir, c) = with_channel().await;
    let p = Policy::preset(
        Preset::GroupChat,
        CIRCLE,
        channel_id(),
        [0x33; 32],
        3,
        6,
        [4; 16],
    );
    let signed = p.sign(&seat_key(2), 2);
    let body = {
        let mut e = minicbor::Encoder::new(Vec::new());
        e.array(2)
            .unwrap()
            .bytes(&signed)
            .unwrap()
            .array(0)
            .unwrap();
        e.into_writer()
    };
    let (s, v) = send(c.post(format!("{}/policy", ch(&base))).body(body)).await;
    assert_eq!(s, 201);
    assert_eq!(v["policy_id"], hex::encode(policy::id(&signed)));
    let (_, v) = send(c.get(ch(&base))).await;
    assert_eq!(v["policy"], b64(&signed));

    // A policy for another channel than the path's.
    let other = Policy::preset(
        Preset::GroupChat,
        CIRCLE,
        [0x42; 16],
        [0x33; 32],
        3,
        6,
        [4; 16],
    );
    let body = {
        let mut e = minicbor::Encoder::new(Vec::new());
        e.array(2)
            .unwrap()
            .bytes(&other.sign(&seat_key(2), 2))
            .unwrap()
            .array(0)
            .unwrap();
        e.into_writer()
    };
    let (s, v) = send(c.post(format!("{}/policy", ch(&base))).body(body)).await;
    assert_eq!((s, v["error"].as_str()), (400, Some("channel_id")));

    // Signed by a key that is not seat 2's.
    let body = {
        let mut e = minicbor::Encoder::new(Vec::new());
        e.array(2)
            .unwrap()
            .bytes(&p.sign(&member(0), 2))
            .unwrap()
            .array(0)
            .unwrap();
        e.into_writer()
    };
    let (s, v) = send(c.post(format!("{}/policy", ch(&base))).body(body)).await;
    assert_eq!((s, v["error"].as_str()), (403, Some("signature")));

    // A policy for this channel signed by a seat of another stored circle: it names its own
    // circle, whose manifest it verifies against, but the channel belongs to CIRCLE.
    let other_circle = [8u8; 16];
    let url = format!("{base}/v1/circles/{}/manifest", hex::encode(other_circle));
    let other_manifest = seal(&manifest_with(other_circle, "other"));
    assert_eq!(send(c.put(url).body(other_manifest)).await.0, 201);
    let hijack = Policy::preset(
        Preset::Closed,
        other_circle,
        channel_id(),
        [0x33; 32],
        3,
        1_000_000,
        [5; 16],
    );
    let body = policy_body(&hijack.sign(&seat_key(0), 0));
    let (s, v) = send(c.post(format!("{}/policy", ch(&base))).body(body)).await;
    assert_eq!((s, v["error"].as_str()), (400, Some("circle_id")));
    let (_, v) = send(c.get(ch(&base))).await;
    assert_eq!(v["policy"], b64(&signed));

    // A policy for a channel the relay does not hold.
    let unknown = Policy::preset(
        Preset::GroupChat,
        CIRCLE,
        [0x42; 16],
        [0x33; 32],
        3,
        6,
        [4; 16],
    );
    let body = policy_body(&unknown.sign(&seat_key(2), 2));
    let url = format!("{base}/v1/channels/{}/policy", hex::encode([0x42u8; 16]));
    let (s, v) = send(c.post(url).body(body)).await;
    assert_eq!((s, v["error"].as_str()), (404, Some("channel")));
}

fn policy_body(signed: &[u8]) -> Vec<u8> {
    let mut e = minicbor::Encoder::new(Vec::new());
    e.array(2).unwrap().bytes(signed).unwrap().array(0).unwrap();
    e.into_writer()
}

#[tokio::test]
async fn grants_post_then_get() {
    let (base, _dir, c) = with_channel().await;
    let root_claims = issue_root(
        &CIRCLE,
        &channel_id(),
        1,
        &member(0).public(),
        &[Action::Read, Action::Post, Action::Admit],
        0,
        100,
        1,
        [1; 16],
    );
    let root = sign_root(&root_claims, &seat_key(1), 1);
    let parent = verify_chain(&[&root], &manifest(), &channel_id()).unwrap();
    let child_claims = issue_delegated(
        &root,
        &parent,
        &member(1).public(),
        &[Action::Read],
        0,
        50,
        [2; 16],
    );
    let child = sign_delegated(&child_claims, &member(0));
    let chain = cbor_array(&[&root, &child]);

    let (s, v) = send(c.post(format!("{}/grants", ch(&base))).body(chain.clone())).await;
    assert_eq!(s, 201);
    assert_eq!(v["grant_id"], hex::encode([2u8; 16]));
    let root_only = cbor_array(&[&root]);
    let (s, v) = send(
        c.post(format!("{}/grants", ch(&base)))
            .body(root_only.clone()),
    )
    .await;
    assert_eq!(
        (s, v["grant_id"].as_str()),
        (201, Some("01".repeat(16).as_str()))
    );

    let kid = hex::encode(key_id(&member(1).public()));
    let (s, v) = send(c.get(format!("{}/grants?kid={kid}", ch(&base)))).await;
    assert_eq!(s, 200);
    assert_eq!(v["grants"], serde_json::json!([b64(&chain)]));
    let (_, v) = send(c.get(format!("{}/grants?kid={}", ch(&base), "00".repeat(32)))).await;
    assert_eq!(v["grants"], serde_json::json!([]));

    let (s, v) = send(c.get(format!("{}/grants/{}", ch(&base), "01".repeat(16)))).await;
    assert_eq!(s, 200);
    assert_eq!(v["grant"], b64(&root_only));
    let (s, v) = send(c.get(format!("{}/grants/{}", ch(&base), "05".repeat(16)))).await;
    assert_eq!((s, v["error"].as_str()), (404, Some("grant")));

    // A grant for another channel than the path's.
    let other = sign_root(
        &issue_root(
            &CIRCLE,
            &[0x42; 16],
            1,
            &member(0).public(),
            &[Action::Read],
            0,
            100,
            0,
            [3; 16],
        ),
        &seat_key(1),
        1,
    );
    let (s, v) = send(
        c.post(format!("{}/grants", ch(&base)))
            .body(cbor_array(&[&other])),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (400, Some("channel_id")));
    let (s, v) = send(
        c.post(format!("{}/grants", ch(&base)))
            .body(cbor_array(&[])),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (400, Some("malformed")));
}

#[tokio::test]
async fn revocations_post_then_get() {
    let (base, _dir, c) = with_channel().await;
    let grant_id = [2u8; 16];
    let statement = seat_statement(0, b"cahoots/revoke/v1", &grant_id, Some("left"));
    let body = cbor_array(&[&grant_id, &statement]);
    assert_eq!(
        send(c.post(format!("{}/revocations", ch(&base))).body(body))
            .await
            .0,
        201
    );
    let (s, v) = send(c.get(format!("{}/revocations", ch(&base)))).await;
    assert_eq!(s, 200);
    let list = v["revocations"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["grant_id"], hex::encode(grant_id));
    assert_eq!(list[0]["seq"], Value::Null);

    // The statement names another grant than the body's.
    let body = cbor_array(&[&[9u8; 16], &statement]);
    let (s, v) = send(c.post(format!("{}/revocations", ch(&base))).body(body)).await;
    assert_eq!((s, v["error"].as_str()), (400, Some("malformed")));
    // Signed under the wrong domain.
    let wrong = seat_statement(0, b"cahoots/gate/v1", &grant_id, Some("left"));
    let body = cbor_array(&[&grant_id, &wrong]);
    let (s, v) = send(c.post(format!("{}/revocations", ch(&base))).body(body)).await;
    assert_eq!((s, v["error"].as_str()), (403, Some("signature")));
}

#[tokio::test]
async fn deposits_post_then_get() {
    let (base, _dir, c) = with_channel().await;
    let secret = LinkSecret([5; 32]);
    let epoch = EpochId {
        channel_id: channel_id(),
        epoch: 1,
    };
    let d = seal_for_link(&[6; 32], &secret, &epoch);
    let bytes = d.encode();
    let (s, v) = send(
        c.post(format!("{}/deposits", ch(&base)))
            .body(bytes.clone()),
    )
    .await;
    assert_eq!(s, 201);
    assert_eq!(v["deposit_id"], hex::encode(d.id()));

    let to = hex::encode(secret.id());
    let url = format!(
        "{}/deposits?to={to}&epoch={}",
        ch(&base),
        hex::encode(epoch.encode())
    );
    let (s, v) = send(c.get(url)).await;
    assert_eq!(s, 200);
    assert_eq!(v["deposits"], serde_json::json!([b64(&bytes)]));

    // An epoch with no deposits is an empty list, not 404.
    let unknown = EpochId {
        channel_id: channel_id(),
        epoch: 2,
    };
    let url = format!(
        "{}/deposits?to={to}&epoch={}",
        ch(&base),
        hex::encode(unknown.encode())
    );
    let (s, v) = send(c.get(url)).await;
    assert_eq!(s, 200);
    assert_eq!(v["deposits"], serde_json::json!([]));

    // A deposit for another channel than the path's.
    let other = seal_for_link(
        &[6; 32],
        &secret,
        &EpochId {
            channel_id: [0x42; 16],
            epoch: 1,
        },
    );
    let (s, v) = send(
        c.post(format!("{}/deposits", ch(&base)))
            .body(other.encode()),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (400, Some("channel_id")));
}

#[tokio::test]
async fn requests_post_then_get() {
    let (base, _dir, c) = with_channel().await;
    let url = format!("{}/requests", ch(&base));
    let (s, v) = send(c.post(&url).body(b"first".to_vec())).await;
    assert_eq!(s, 201);
    assert_eq!(v["request_id"], hex::encode(Sha256::digest(b"first")));
    assert_eq!(send(c.post(&url).body(b"second".to_vec())).await.0, 201);

    let (_, v) = send(c.get(&url)).await;
    let list = v["requests"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["seq"], 0);
    assert_eq!(list[0]["body"], b64(b"first"));
    let (_, v) = send(c.get(format!("{url}?after=0"))).await;
    let list = v["requests"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], hex::encode(Sha256::digest(b"second")));
    assert_eq!(list[0]["body"], b64(b"second"));
    assert_eq!(list[0]["seq"], 1);

    // The same body in another channel is that channel's own request.
    let other = format!("{base}/v1/channels/{}/requests", hex::encode([0x42u8; 16]));
    let (s, v) = send(c.post(&other).body(b"first".to_vec())).await;
    assert_eq!((s, v["seq"].as_u64()), (201, Some(0)));
    let (_, v) = send(c.get(&other)).await;
    let list = v["requests"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["body"], b64(b"first"));
    let (_, v) = send(c.get(&url)).await;
    assert_eq!(v["requests"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn gates_post_then_delete() {
    let (base, dir, c) = with_channel().await;
    let token = [0x77u8; 32];
    let hash: [u8; 32] = Sha256::digest(token).into();
    let signed = seat_statement(1, b"cahoots/gate/v1", &token, None);
    let (s, v) = send(c.post(format!("{}/gates", ch(&base))).body(signed.clone())).await;
    assert_eq!(s, 201);
    assert_eq!(v["token_hash"], hex::encode(hash));

    let count = |dir: &tempfile::TempDir| -> i64 {
        rusqlite::Connection::open(dir.path().join("station.db"))
            .unwrap()
            .query_row(
                "SELECT count(*) FROM gates WHERE token_hash = ?1",
                [&hash[..]],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(count(&dir), 1);

    let retire = seat_statement(1, b"cahoots/gate-retire/v1", &token, None);
    let delete_url = format!("{}/gates/{}", ch(&base), hex::encode(hash));

    // A POST statement does not remove the token, and a retire statement does not add one.
    let (s, v) = send(c.delete(&delete_url).body(signed.clone())).await;
    assert_eq!((s, v["error"].as_str()), (403, Some("signature")));
    assert_eq!(count(&dir), 1);
    let other_token = [0x78u8; 32];
    let other_retire = seat_statement(1, b"cahoots/gate-retire/v1", &other_token, None);
    let (s, v) = send(c.post(format!("{}/gates", ch(&base))).body(other_retire)).await;
    assert_eq!((s, v["error"].as_str()), (403, Some("signature")));

    // Signed by a key that is not seat 1's.
    let mut e = minicbor::Encoder::new(Vec::new());
    e.map(2).unwrap();
    e.u8(0).unwrap().bytes(&channel_id()).unwrap();
    e.u8(1).unwrap().bytes(&token).unwrap();
    let forged = cose::sign1(
        &member(0),
        &[1],
        &e.into_writer(),
        b"cahoots/gate-retire/v1",
        false,
    );
    let (s, v) = send(
        c.delete(format!("{}/gates/{}", ch(&base), hex::encode(hash)))
            .body(forged),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (403, Some("signature")));
    assert_eq!(count(&dir), 1);

    // The path's hash must be the hash of the signed token.
    let (s, _) = send(
        c.delete(format!("{}/gates/{}", ch(&base), "00".repeat(32)))
            .body(retire.clone()),
    )
    .await;
    assert_eq!(s, 400);

    let (s, _) = send(c.delete(&delete_url).body(retire)).await;
    assert_eq!(s, 204);
    assert_eq!(count(&dir), 0);
}

#[tokio::test]
async fn object_writes_answer_507_when_full() {
    let fresh = tempfile::tempdir().unwrap();
    let empty = cahoots_station::log::Log::open(&fresh.path().join("station.db"))
        .unwrap()
        .bytes();
    let (base, _dir) = start_with(Limits::default(), Some(empty + 1)).await;
    let c = Client::new();
    let url = format!("{base}/v1/circles/{}/manifest", hex::encode(CIRCLE));
    // The cap is checked before the write, so the first write lands and fills past it.
    assert_eq!(send(c.put(&url).body(seal(&manifest()))).await.0, 201);
    let (s, v) = send(
        c.post(format!("{base}/v1/channels"))
            .body(object(5).sign(&seat_key(0), 0)),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (507, Some("full")));
    let (s, v) = send(
        c.post(format!("{}/requests", ch(&base)))
            .body(b"x".to_vec()),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (507, Some("full")));
    let gate = format!("{}/gates/{}", ch(&base), "00".repeat(32));
    let (s, v) = send(c.delete(gate).body(vec![0])).await;
    assert_eq!((s, v["error"].as_str()), (507, Some("full")));
    // Reads still answer.
    assert_eq!(c.get(&url).send().await.unwrap().status(), 200);
}

#[tokio::test]
async fn puts_and_deletes_count_as_posts() {
    let (base, _dir) = start_with(
        Limits {
            posts_per_minute: Some(1),
            reads_per_minute: Some(1),
            trust_forwarded: false,
            channel_posts_per_minute: None,
        },
        None,
    )
    .await;
    let c = Client::new();
    let url = format!("{base}/v1/circles/{}/manifest", hex::encode(CIRCLE));
    assert_eq!(send(c.put(&url).body(seal(&manifest()))).await.0, 201);
    let (s, v) = send(c.put(&url).body(seal(&manifest()))).await;
    assert_eq!((s, v["error"].as_str()), (429, Some("rate")));
    let gate = format!("{}/gates/{}", ch(&base), "00".repeat(32));
    assert_eq!(send(c.delete(&gate).body(vec![0])).await.0, 429);
    assert_eq!(c.get(&url).send().await.unwrap().status(), 200);
    assert_eq!(c.get(&url).send().await.unwrap().status(), 429);
}
