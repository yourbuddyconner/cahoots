use cahoots_core::message::{seal, MessageType};
use cahoots_station::server;
use serde_json::Value;
use std::time::{Duration, Instant};

fn chat(cid: [u8; 16], counter: u32) -> Vec<u8> {
    let mut m = vec![2u8];
    m.extend_from_slice(&cid);
    m.extend_from_slice(&[1, 2, 3, 4, 5]);
    m.push(0);
    m.extend_from_slice(&counter.to_le_bytes());
    m.extend_from_slice(&4u32.to_le_bytes());
    m.push(0);
    m.extend(std::iter::repeat_n(7u8, 20));
    m
}

async fn start() -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("station.jsonl");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = server::app(server::State::open(&path).unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), dir)
}

#[tokio::test]
async fn post_get_head_roundtrip() {
    let (base, _dir) = start().await;
    let client = reqwest::Client::new();
    let cid = [0xAB; 16];
    let cid_hex = hex::encode(cid);
    let r = client.get(format!("{base}/healthz")).send().await.unwrap();
    assert_eq!(r.status(), 200);

    let r = client
        .post(format!("{base}/v1/circles/{cid_hex}/messages"))
        .body(chat(cid, 0))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["seq"], 0);
    assert_eq!(v["hash"].as_str().unwrap().len(), 64);

    let r = client
        .get(format!("{base}/v1/circles/{cid_hex}/messages"))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    let entries = v["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["circle_id"], cid_hex);
    assert_eq!(entries[0]["seq"], 0);
    assert!(entries[0]["message"].as_str().unwrap().starts_with("Aq"));

    let r = client
        .get(format!("{base}/v1/circles/{cid_hex}/head"))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["seq"], 0);
    let r = client.get(format!("{base}/v1/head")).send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["seq"], 0);
    let r = client
        .get(format!("{base}/v1/circles"))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["circles"][0]["count"], 1);
    let r = client.get(format!("{base}/")).send().await.unwrap();
    assert!(r.text().await.unwrap().contains("Numbers Station"));
}

#[tokio::test]
async fn reuse_is_409_and_mismatch_is_400() {
    let (base, _dir) = start().await;
    let client = reqwest::Client::new();
    let cid = [0x11; 16];
    let cid_hex = hex::encode(cid);
    let pad = [3u8; 34];
    let m1 = seal(&cid, 0, 0, MessageType::Text, b"hi", &pad).unwrap();
    let m2 = seal(&cid, 0, 10, MessageType::Text, b"yo", &pad).unwrap();
    let url = format!("{base}/v1/circles/{cid_hex}/messages");
    assert_eq!(
        client
            .post(&url)
            .body(m1.clone())
            .send()
            .await
            .unwrap()
            .status(),
        201
    );
    let r = client.post(&url).body(m2).send().await.unwrap();
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"], "reuse");
    let other = format!("{base}/v1/circles/{}/messages", hex::encode([0x22; 16]));
    assert_eq!(
        client.post(&other).body(m1).send().await.unwrap().status(),
        400
    );
    let bad = format!("{base}/v1/circles/nothex/messages");
    assert_eq!(
        client
            .post(&bad)
            .body(vec![1])
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
}

#[tokio::test]
async fn long_poll_returns_when_entry_arrives() {
    let (base, _dir) = start().await;
    let client = reqwest::Client::new();
    let cid = [0x33; 16];
    let cid_hex = hex::encode(cid);
    let base2 = base.clone();
    let cid_hex2 = cid_hex.clone();
    let poster = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        reqwest::Client::new()
            .post(format!("{base2}/v1/circles/{cid_hex2}/messages"))
            .body(chat(cid, 0))
            .send()
            .await
            .unwrap()
    });
    let t0 = Instant::now();
    let r = client
        .get(format!("{base}/v1/circles/{cid_hex}/messages?wait=5000"))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["entries"].as_array().unwrap().len(), 1);
    assert!(
        t0.elapsed() < Duration::from_secs(4),
        "returned on arrival, not timeout"
    );
    poster.await.unwrap();
    // Empty poll with short wait times out with no entries.
    let r = client
        .get(format!(
            "{base}/v1/circles/{cid_hex}/messages?after=0&wait=100"
        ))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["entries"].as_array().unwrap().len(), 0);
}
