//! The `channels` feature: version 3 messages chained in each channel's stream, per-channel
//! heads and long-poll, and the per-channel post limit.
#![cfg(feature = "channels")]

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use cahoots_channel::keys::{epoch_key, EpochId};
use cahoots_channel::message::{MessageType, HEADER_LEN};
use cahoots_channel::object::channel_id_for;
use cahoots_channel::session::ChannelSession;
use cahoots_channel::{ChannelId, ChannelObject};
use cahoots_core::cose;
use cahoots_core::manifest::{Manifest, SealedManifest, Seat, MANIFEST_AAD};
use cahoots_core::params::CircleParams;
use cahoots_core::seat::SeatKey;
use cahoots_station::limits::Limits;
use cahoots_station::log::Log;
use cahoots_station::rules::{RuleError, Submission};
use cahoots_station::server;
use reqwest::{Client, Response};
use serde_json::Value;
use std::time::{Duration, Instant};

const CIRCLE: [u8; 16] = [7; 16];
const NONCE: [u8; 16] = [9; 16];
const OTHER_NONCE: [u8; 16] = [10; 16];
/// The body of a message whose plaintext is `b"hello"`: the header and 5 + 16 bytes of
/// ciphertext. Cut there, a message has no signature and does not split.
const BODY_LEN: usize = HEADER_LEN + 5 + 16;

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

fn sealed_manifest() -> Vec<u8> {
    let params = CircleParams::new(3, 192, "channel", false).unwrap();
    let m = Manifest {
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
    };
    let body = m.encode();
    let attestations = (0..3u8)
        .map(|i| cose::sign1(&seat_key(i), &[i], &body, MANIFEST_AAD, true))
        .collect();
    SealedManifest { body, attestations }.encode()
}

fn object(nonce: [u8; 16]) -> ChannelObject {
    ChannelObject {
        circle_id: CIRCLE,
        channel_id: channel_id_for(&CIRCLE, &nonce),
        name: "general".into(),
        created_log: 5,
        policy_id: [0x11; 32],
        nonce,
    }
}

/// A member of `channel` holding epoch 1.
fn member(channel: ChannelId, i: u64) -> ChannelSession {
    let mut s = ChannelSession::new(channel, member_key(i));
    let epoch = EpochId {
        channel_id: channel,
        epoch: 1,
    };
    s.install_epoch(epoch, epoch_key(&epoch, &[0x5a; 32]))
        .unwrap();
    s
}

fn text(s: &mut ChannelSession, body: &[u8]) -> Vec<u8> {
    s.send(MessageType::Text, body, &[]).unwrap()
}

fn no_address_limits() -> Limits {
    Limits {
        posts_per_minute: None,
        reads_per_minute: None,
        ..Limits::default()
    }
}

async fn start_with(limits: Limits) -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("station.db");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = server::app(server::State::open_with(&path, limits, None).unwrap())
        .into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), dir)
}

async fn send(r: reqwest::RequestBuilder) -> (u16, Value) {
    let r: Response = r.send().await.unwrap();
    let status = r.status().as_u16();
    let v = r.json().await.unwrap_or(Value::Null);
    (status, v)
}

/// A station holding the manifest and the channel objects for `nonces`.
async fn with_channels(limits: Limits, nonces: &[[u8; 16]]) -> (String, tempfile::TempDir, Client) {
    let (base, dir) = start_with(limits).await;
    let c = Client::new();
    let url = format!("{base}/v1/circles/{}/manifest", hex::encode(CIRCLE));
    assert_eq!(send(c.put(url).body(sealed_manifest())).await.0, 201);
    for n in nonces {
        let (s, _) = send(
            c.post(format!("{base}/v1/channels"))
                .body(object(*n).sign(&seat_key(0), 0)),
        )
        .await;
        assert_eq!(s, 201);
    }
    (base, dir, c)
}

fn ch(base: &str, channel: &ChannelId) -> String {
    format!("{base}/v1/channels/{}", hex::encode(channel))
}

#[tokio::test]
async fn post_poll_and_head_per_channel() {
    let (base, _dir, c) = with_channels(no_address_limits(), &[NONCE, OTHER_NONCE]).await;
    let channel = channel_id_for(&CIRCLE, &NONCE);
    let other = channel_id_for(&CIRCLE, &OTHER_NONCE);

    // Empty stream: genesis head, no entries.
    let (s, v) = send(c.get(format!("{}/head", ch(&base, &channel)))).await;
    assert_eq!(s, 200);
    assert_eq!(v["seq"], Value::Null);
    assert_eq!(v["hash"], hex::encode([0u8; 32]));
    let (s, v) = send(c.get(format!("{}/messages", ch(&base, &channel)))).await;
    assert_eq!(s, 200);
    assert_eq!(v["entries"], serde_json::json!([]));

    // A circle message first, so the channel's first seq is not 0: seq is global.
    let mut pad_free = vec![2u8];
    pad_free.extend_from_slice(&CIRCLE);
    pad_free.extend_from_slice(&[1, 2, 3, 4, 5, 0]);
    pad_free.extend_from_slice(&0u32.to_le_bytes());
    pad_free.extend_from_slice(&1u32.to_le_bytes());
    pad_free.push(0);
    pad_free.extend_from_slice(&[0u8; 17]);
    let circle_url = format!("{base}/v1/circles/{}/messages", hex::encode(CIRCLE));
    assert_eq!(send(c.post(&circle_url).body(pad_free)).await.0, 201);

    let mut alice = member(channel, 0);
    let mut bob = member(channel, 1);
    let w0 = text(&mut alice, b"hello");
    let (s, v) = send(
        c.post(format!("{}/messages", ch(&base, &channel)))
            .body(w0.clone()),
    )
    .await;
    assert_eq!(s, 201, "{v}");
    assert_eq!(v["seq"], 1);
    let hash0 = v["hash"].as_str().unwrap().to_string();

    // Polled back whole, and it opens for another member.
    let (s, v) = send(c.get(format!("{}/messages", ch(&base, &channel)))).await;
    assert_eq!(s, 200);
    let entries = v["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["seq"], 1);
    assert_eq!(entries[0]["channel_id"], hex::encode(channel));
    assert_eq!(entries[0]["hash"], hash0);
    let wire = URL_SAFE_NO_PAD
        .decode(entries[0]["message"].as_str().unwrap())
        .unwrap();
    assert_eq!(wire, w0);
    assert_eq!(bob.receive(&wire, &alice.my_public()).unwrap().1, b"hello");
    assert_eq!(v["head_seq"], 1);

    // The channel head is the channel's last entry.
    let (_, v) = send(c.get(format!("{}/head", ch(&base, &channel)))).await;
    assert_eq!(
        (v["seq"].clone(), v["hash"].as_str()),
        (1.into(), Some(&hash0[..]))
    );

    // The circle's stream does not hold the channel's message, and the circle head is seq 0.
    let (_, v) = send(c.get(&circle_url)).await;
    assert_eq!(v["entries"].as_array().unwrap().len(), 1);
    assert_eq!(v["entries"][0]["seq"], 0);

    // The other channel's stream is untouched until it gets its own message.
    let (_, v) = send(c.get(format!("{}/head", ch(&base, &other)))).await;
    assert_eq!(v["seq"], Value::Null);
    let mut carol = member(other, 2);
    let (s, v) = send(
        c.post(format!("{}/messages", ch(&base, &other)))
            .body(text(&mut carol, b"elsewhere")),
    )
    .await;
    assert_eq!((s, v["seq"].clone()), (201, 2.into()));

    // A long-poll after the last entry wakes on the next post to this channel.
    let poll = {
        let c = c.clone();
        let url = format!("{}/messages?after=1&wait=5000", ch(&base, &channel));
        tokio::spawn(async move { send(c.get(url)).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    let started = Instant::now();
    let w1 = text(&mut alice, b"again");
    let (s, _) = send(
        c.post(format!("{}/messages", ch(&base, &channel)))
            .body(w1.clone()),
    )
    .await;
    assert_eq!(s, 201);
    let (s, v) = poll.await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(4));
    assert_eq!(s, 200);
    let entries = v["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["seq"], 3);

    // after=1 skips the first message and the other channel's.
    let (_, v) = send(c.get(format!("{}/messages?after=1", ch(&base, &channel)))).await;
    assert_eq!(v["entries"].as_array().unwrap().len(), 1);

    // The listing names both channels with their counts; circles list only the circle.
    let (s, v) = send(c.get(format!("{base}/v1/channels"))).await;
    assert_eq!(s, 200);
    let list = v["channels"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    let find = |id: &ChannelId| {
        list.iter()
            .find(|e| e["channel_id"] == hex::encode(id))
            .unwrap()
            .clone()
    };
    assert_eq!(
        (
            find(&channel)["count"].clone(),
            find(&channel)["last_seq"].clone()
        ),
        (2.into(), 3.into())
    );
    assert_eq!(
        (
            find(&other)["count"].clone(),
            find(&other)["last_seq"].clone()
        ),
        (1.into(), 2.into())
    );
    let (_, v) = send(c.get(format!("{base}/v1/circles"))).await;
    let circles = v["circles"].as_array().unwrap();
    assert_eq!(circles.len(), 1);
    assert_eq!(circles[0]["circle_id"], hex::encode(CIRCLE));
}

#[tokio::test]
async fn a_channel_without_messages_is_listed_with_no_last_seq() {
    let (base, _dir, c) = with_channels(no_address_limits(), &[NONCE]).await;
    let (_, v) = send(c.get(format!("{base}/v1/channels"))).await;
    assert_eq!(
        v["channels"],
        serde_json::json!([{
            "channel_id": hex::encode(channel_id_for(&CIRCLE, &NONCE)),
            "count": 0,
            "last_seq": null,
        }])
    );
}

#[tokio::test]
async fn refusals() {
    let (base, _dir, c) = with_channels(no_address_limits(), &[NONCE]).await;
    let channel = channel_id_for(&CIRCLE, &NONCE);
    let mut alice = member(channel, 0);
    let w = text(&mut alice, b"hello");

    // A version 3 body on a circle route is malformed.
    let (s, v) = send(
        c.post(format!(
            "{base}/v1/circles/{}/messages",
            hex::encode(CIRCLE)
        ))
        .body(w.clone()),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (400, Some("malformed")));

    // A channel the relay does not hold.
    let unknown = channel_id_for(&CIRCLE, &OTHER_NONCE);
    let mut stranger = member(unknown, 0);
    let (s, v) = send(
        c.post(format!("{}/messages", ch(&base, &unknown)))
            .body(text(&mut stranger, b"hi")),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (404, Some("channel")));

    // A message for another channel than the path's.
    let (s, v) = send(
        c.post(format!("{}/messages", ch(&base, &channel)))
            .body(text(&mut stranger, b"hi")),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (400, Some("malformed")));

    // A circle-version body on a channel route, junk, and a truncated message.
    let (s, v) = send(
        c.post(format!("{}/messages", ch(&base, &channel)))
            .body(vec![2u8; 40]),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (400, Some("malformed")));
    let (s, v) = send(
        c.post(format!("{}/messages", ch(&base, &channel)))
            .body(w[..BODY_LEN].to_vec()),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (400, Some("malformed")));

    // Bad hex in the path.
    let (s, v) = send(
        c.post(format!("{base}/v1/channels/zz/messages"))
            .body(w.clone()),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (400, Some("channel_id")));
    let (s, v) = send(c.get(format!("{base}/v1/channels/zz/head"))).await;
    assert_eq!((s, v["error"].as_str()), (400, Some("channel_id")));

    // Too large.
    let (s, v) = send(
        c.post(format!("{}/messages", ch(&base, &channel)))
            .body(vec![3u8; 64 * 1024 + 1]),
    )
    .await;
    assert_eq!((s, v["error"].as_str()), (413, Some("too_large")));

    // Nothing was stored.
    let (_, v) = send(c.get(format!("{base}/v1/head"))).await;
    assert_eq!(v["seq"], Value::Null);
}

#[tokio::test]
async fn the_default_channel_limit_refuses_the_301st_post() {
    let limits = no_address_limits();
    assert_eq!(limits.channel_posts_per_minute, Some(300));
    let (base, _dir, c) = with_channels(limits, &[NONCE, OTHER_NONCE]).await;
    let channel = channel_id_for(&CIRCLE, &NONCE);
    let url = format!("{}/messages", ch(&base, &channel));
    let mut alice = member(channel, 0);
    let started = Instant::now();
    for i in 0..300 {
        let (s, v) = send(c.post(&url).body(text(&mut alice, b"x"))).await;
        assert_eq!(s, 201, "post {i}: {v}");
    }
    // The bucket refills at 5 tokens a second while the 300 posts go in, so the refusal comes
    // at the 301st post plus at most the tokens refilled since the first.
    let mut extra = 0u32;
    let r = loop {
        let r = c
            .post(&url)
            .body(text(&mut alice, b"x"))
            .send()
            .await
            .unwrap();
        if r.status() != 201 {
            break r;
        }
        extra += 1;
        let refilled = started.elapsed().as_secs_f64() * 5.0;
        assert!(
            f64::from(extra) <= refilled.ceil(),
            "post {} accepted",
            300 + extra
        );
    };
    assert_eq!(r.status(), 429);
    assert_eq!(r.headers()["retry-after"], "10");
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"], "rate");

    // The bucket is the channel's: another channel still takes posts.
    let other = channel_id_for(&CIRCLE, &OTHER_NONCE);
    let mut bob = member(other, 1);
    let (s, _) = send(
        c.post(format!("{}/messages", ch(&base, &other)))
            .body(text(&mut bob, b"y")),
    )
    .await;
    assert_eq!(s, 201);
}

#[tokio::test]
async fn a_small_channel_limit_and_none() {
    let limits = Limits {
        channel_posts_per_minute: Some(2),
        ..no_address_limits()
    };
    let (base, _dir, c) = with_channels(limits, &[NONCE]).await;
    let channel = channel_id_for(&CIRCLE, &NONCE);
    let url = format!("{}/messages", ch(&base, &channel));
    let mut alice = member(channel, 0);
    assert_eq!(send(c.post(&url).body(text(&mut alice, b"1"))).await.0, 201);
    assert_eq!(send(c.post(&url).body(text(&mut alice, b"2"))).await.0, 201);
    let (s, v) = send(c.post(&url).body(text(&mut alice, b"3"))).await;
    assert_eq!((s, v["error"].as_str()), (429, Some("rate")));
    // Reads are not limited by the channel bucket.
    assert_eq!(send(c.get(&url)).await.0, 200);

    let (_, v) = send(c.get(format!("{base}/v1/info"))).await;
    assert_eq!(v["limits"]["channel_posts_per_minute"], 2);

    let limits = Limits {
        channel_posts_per_minute: None,
        ..no_address_limits()
    };
    let (base, _dir, c) = with_channels(limits, &[NONCE]).await;
    let url = format!("{}/messages", ch(&base, &channel));
    let mut alice = member(channel, 0);
    for _ in 0..5 {
        assert_eq!(send(c.post(&url).body(text(&mut alice, b"x"))).await.0, 201);
    }
    let (_, v) = send(c.get(format!("{base}/v1/info"))).await;
    assert_eq!(v["limits"]["channel_posts_per_minute"], Value::Null);
}

#[test]
fn parse_channel_rules() {
    let channel = channel_id_for(&CIRCLE, &NONCE);
    let mut alice = member(channel, 0);
    let w = text(&mut alice, b"hello");
    let s = Submission::parse_channel(&channel, &w).unwrap();
    assert_eq!(s.stream(), channel);
    assert_eq!(s.pad_range(), None);
    assert_eq!(s.bytes(), &w[..]);

    assert!(matches!(
        Submission::parse_channel(&[1; 16], &w),
        Err(RuleError::ChannelMismatch)
    ));
    assert!(matches!(
        Submission::parse_channel(&channel, &[2, 0, 0]),
        Err(RuleError::Version)
    ));
    assert!(matches!(
        Submission::parse_channel(&channel, &[]),
        Err(RuleError::Length)
    ));
    assert!(matches!(
        Submission::parse_channel(&channel, &w[..BODY_LEN]),
        Err(RuleError::Length)
    ));
    // The circle rules keep refusing version 3.
    assert!(matches!(
        Submission::parse(&channel, &w),
        Err(RuleError::Version)
    ));
}

#[test]
fn channel_entries_survive_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("station.db");
    let channel = channel_id_for(&CIRCLE, &NONCE);
    let mut alice = member(channel, 0);
    let head = {
        let mut log = Log::open(&path).unwrap();
        for body in [&b"one"[..], b"two"] {
            let sub = Submission::parse_channel(&channel, &text(&mut alice, body)).unwrap();
            log.append(sub, 1).unwrap();
        }
        log.stream_head(&channel)
    };
    let log = Log::open(&path).unwrap();
    assert_eq!(log.stream_head(&channel), head);
    assert_eq!(head.seq, Some(1));
    assert_eq!(log.entries_after_stream(&channel, Some(0)).len(), 1);
}

/// A well-formed version 2 (chat) body for `circle`, with one byte of plaintext.
fn chat(circle: [u8; 16]) -> Vec<u8> {
    let mut m = vec![2u8];
    m.extend_from_slice(&circle);
    m.extend_from_slice(&[1, 2, 3, 4, 5, 0]);
    m.extend_from_slice(&0u32.to_le_bytes());
    m.extend_from_slice(&1u32.to_le_bytes());
    m.push(0);
    m.extend_from_slice(&[0u8; 17]);
    m
}

#[tokio::test]
async fn circle_routes_refuse_a_channel_id_and_the_feed_leaves_channels_out() {
    let (base, _dir, c) = with_channels(no_address_limits(), &[NONCE]).await;
    let channel = channel_id_for(&CIRCLE, &NONCE);
    let as_circle = format!("{base}/v1/circles/{}", hex::encode(channel));

    // A chat body naming the channel's id as its circle: refused, and nothing is stored.
    let (s, v) = send(c.post(format!("{as_circle}/messages")).body(chat(channel))).await;
    assert_eq!((s, v["error"].as_str()), (404, Some("channel")), "{v}");
    assert_eq!(v["detail"], "this id is a channel; use /v1/channels/...");
    let (s, v) = send(c.get(format!("{as_circle}/messages"))).await;
    assert_eq!((s, v["error"].as_str()), (404, Some("channel")));
    let (s, v) = send(c.get(format!("{as_circle}/head"))).await;
    assert_eq!((s, v["error"].as_str()), (404, Some("channel")));
    let (_, v) = send(c.get(format!("{base}/v1/head"))).await;
    assert_eq!(v["seq"], Value::Null);

    // The circle still takes posts; the feed shows the circle's entries, not the channel's.
    let circle_url = format!("{base}/v1/circles/{}/messages", hex::encode(CIRCLE));
    assert_eq!(send(c.post(&circle_url).body(chat(CIRCLE))).await.0, 201);
    let mut alice = member(channel, 0);
    let (s, _) = send(
        c.post(format!("{}/messages", ch(&base, &channel)))
            .body(text(&mut alice, b"hi")),
    )
    .await;
    assert_eq!(s, 201);
    assert_eq!(send(c.post(&circle_url).body(chat(CIRCLE))).await.0, 201);
    let (_, v) = send(c.get(format!("{base}/v1/feed"))).await;
    let seen: Vec<_> = v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["seq"].clone(), e["circle_id"].clone()))
        .collect();
    let circle = Value::from(hex::encode(CIRCLE));
    assert_eq!(seen, vec![(0.into(), circle.clone()), (2.into(), circle)]);
    assert_eq!(v["head_seq"], 2);
}
