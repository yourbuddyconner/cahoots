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
    start_with(cahoots_station::limits::Limits::default(), None).await
}

async fn start_with(
    limits: cahoots_station::limits::Limits,
    max_log_bytes: Option<u64>,
) -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("station.db");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = server::app(server::State::open_with(&path, limits, max_log_bytes).unwrap())
        .into_make_service_with_connect_info::<std::net::SocketAddr>();
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
    #[cfg(feature = "feed-page")]
    {
        let r = client.get(format!("{base}/")).send().await.unwrap();
        assert!(r.text().await.unwrap().contains("Numbers Station"));
    }
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

#[tokio::test]
async fn cors_allows_any_origin_for_reads_and_posts() {
    let (base, _dir) = start().await;
    let client = reqwest::Client::new();
    let cid_hex = hex::encode([0xCD; 16]);
    let r = client
        .request(
            reqwest::Method::OPTIONS,
            format!("{base}/v1/circles/{cid_hex}/messages"),
        )
        .header("Origin", "http://localhost:8766")
        .header("Access-Control-Request-Method", "POST")
        .header("Access-Control-Request-Headers", "content-type")
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "preflight status {}", r.status());
    assert_eq!(r.headers()["access-control-allow-origin"], "*");
    let methods = r.headers()["access-control-allow-methods"]
        .to_str()
        .unwrap();
    assert!(
        methods.contains("POST") && methods.contains("GET"),
        "{methods}"
    );
    let r = client
        .get(format!("{base}/v1/head"))
        .header("Origin", "http://localhost:8766")
        .send()
        .await
        .unwrap();
    assert_eq!(r.headers()["access-control-allow-origin"], "*");
}

#[tokio::test]
async fn rate_limit_answers_429_after_the_burst_and_reads_have_their_own_bucket() {
    let limits = cahoots_station::limits::Limits {
        posts_per_minute: Some(3),
        reads_per_minute: Some(2),
        trust_forwarded: false,
        channel_posts_per_minute: None,
    };
    let (base, _dir) = start_with(limits, None).await;
    let cid = [9u8; 16];
    let url = format!("{base}/v1/circles/{}/messages", hex::encode(cid));
    let client = reqwest::Client::new();
    for i in 0..3 {
        let r = client.post(&url).body(chat(cid, i)).send().await.unwrap();
        assert_eq!(r.status(), 201, "post {i} inside the burst");
    }
    let r = client.post(&url).body(chat(cid, 3)).send().await.unwrap();
    assert_eq!(r.status(), 429);
    assert_eq!(r.headers().get("retry-after").unwrap(), "10");
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"], "rate");
    // Reads are a separate bucket: two pass, the third is refused.
    assert_eq!(client.get(&url).send().await.unwrap().status(), 200);
    assert_eq!(client.get(&url).send().await.unwrap().status(), 200);
    assert_eq!(client.get(&url).send().await.unwrap().status(), 429);
    // The page and the health check are never limited.
    assert_eq!(
        client
            .get(format!("{base}/healthz"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    #[cfg(feature = "feed-page")]
    assert_eq!(
        client
            .get(format!("{base}/"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
}

#[tokio::test]
async fn forwarded_address_is_ignored_unless_trusted() {
    let limits = cahoots_station::limits::Limits {
        posts_per_minute: Some(1),
        reads_per_minute: None,
        trust_forwarded: false,
        channel_posts_per_minute: None,
    };
    let (base, _dir) = start_with(limits, None).await;
    let cid = [8u8; 16];
    let url = format!("{base}/v1/circles/{}/messages", hex::encode(cid));
    let client = reqwest::Client::new();
    let post = |i: u32, ip: &'static str| {
        client
            .post(&url)
            .header("x-forwarded-for", ip)
            .body(chat(cid, i))
            .send()
    };
    assert_eq!(post(0, "10.0.0.1").await.unwrap().status(), 201);
    // A different claimed address does not buy a fresh bucket.
    assert_eq!(post(1, "10.0.0.2").await.unwrap().status(), 429);

    let trusted = cahoots_station::limits::Limits {
        trust_forwarded: true,
        ..limits
    };
    let (base, _dir) = start_with(trusted, None).await;
    let url = format!("{base}/v1/circles/{}/messages", hex::encode(cid));
    let post = |i: u32, ip: &'static str| {
        client
            .post(&url)
            .header("x-forwarded-for", ip)
            .body(chat(cid, i))
            .send()
    };
    assert_eq!(post(0, "10.0.0.1").await.unwrap().status(), 201);
    assert_eq!(post(1, "10.0.0.2").await.unwrap().status(), 201);
    assert_eq!(post(2, "10.0.0.2").await.unwrap().status(), 429);
}

#[tokio::test]
async fn full_log_answers_507() {
    // An empty database already holds its schema, so the cap sits one byte above that.
    let fresh = tempfile::tempdir().unwrap();
    let empty = cahoots_station::log::Log::open(&fresh.path().join("station.db"))
        .unwrap()
        .bytes();
    let (base, _dir) =
        start_with(cahoots_station::limits::Limits::default(), Some(empty + 1)).await;
    let cid = [7u8; 16];
    let url = format!("{base}/v1/circles/{}/messages", hex::encode(cid));
    let client = reqwest::Client::new();
    // The first post lands (the cap is checked before the append) and fills the log past the cap.
    assert_eq!(
        client
            .post(&url)
            .body(chat(cid, 0))
            .send()
            .await
            .unwrap()
            .status(),
        201
    );
    let r = client.post(&url).body(chat(cid, 1)).send().await.unwrap();
    assert_eq!(r.status(), 507);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"], "full");
    // Reads still work on a full log.
    assert_eq!(client.get(&url).send().await.unwrap().status(), 200);
}

#[tokio::test]
async fn info_reports_version_and_default_features() {
    let limits = cahoots_station::limits::Limits {
        posts_per_minute: Some(5),
        reads_per_minute: None,
        trust_forwarded: false,
        channel_posts_per_minute: None,
    };
    let (base, _dir) = start_with(limits, None).await;
    let r = reqwest::get(format!("{base}/v1/info")).await.unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    let expect: Vec<Value> = [
        ("feed-page", cfg!(feature = "feed-page")),
        ("objects", cfg!(feature = "objects")),
        ("channels", cfg!(feature = "channels")),
        ("gate", cfg!(feature = "gate")),
    ]
    .into_iter()
    .filter(|&(_, on)| on)
    .map(|(name, _)| Value::from(name))
    .collect();
    assert_eq!(v["features"], Value::Array(expect));
    // `issuer` and `anchor` have no code, so they are never reported, even compiled in.
    let names = v["features"].as_array().unwrap();
    assert!(!names.contains(&Value::from("issuer")) && !names.contains(&Value::from("anchor")));
    // Under default features, exactly the feed page.
    #[cfg(all(
        feature = "feed-page",
        not(any(feature = "objects", feature = "issuer", feature = "anchor"))
    ))]
    assert_eq!(v["features"], serde_json::json!(["feed-page"]));
    assert_eq!(v["limits"]["max_body"], 65536);
    assert_eq!(v["limits"]["posts_per_minute"], 5);
    assert_eq!(v["limits"]["reads_per_minute"], Value::Null);
}

fn ceremony_message(id: [u8; 16], payload: &[u8]) -> Vec<u8> {
    let ch = cahoots_core::channel::Channel::from_key(id, [5; 32]);
    ch.seal_with_nonce(1, cahoots_core::channel::Round::Commit, payload, [9; 12])
}

#[tokio::test]
async fn a_ceremony_message_is_stored_and_served() {
    let (base, _dir) = start().await;
    let client = reqwest::Client::new();
    let id = [0x3C; 16];
    let url = format!("{base}/v1/circles/{}/messages", hex::encode(id));
    let first = ceremony_message(id, b"a commitment");
    let second = ceremony_message(id, b"the same bytes are not a reuse");
    for (m, seq) in [(&first, 0), (&second, 1), (&first, 2)] {
        let r = client.post(&url).body(m.clone()).send().await.unwrap();
        assert_eq!(r.status(), 201);
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["seq"], seq);
    }
    let v: Value = client.get(&url).send().await.unwrap().json().await.unwrap();
    let entries = v["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    use base64::Engine;
    let got = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(entries[1]["message"].as_str().unwrap())
        .unwrap();
    assert_eq!(got, second);
    // The id in the body must be the id in the path, and the length must be right.
    let other = format!("{base}/v1/circles/{}/messages", hex::encode([0x3D; 16]));
    let r = client
        .post(&other)
        .body(first.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    let mut short = first.clone();
    short.pop();
    let r = client.post(&url).body(short).send().await.unwrap();
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn an_oversized_ceremony_message_is_refused() {
    let (base, _dir) = start().await;
    let client = reqwest::Client::new();
    let id = [0x4E; 16];
    let url = format!("{base}/v1/circles/{}/messages", hex::encode(id));
    let cap = cahoots_station::rules::MAX_BODY;
    let fits = ceremony_message(id, &vec![1u8; cap - 21 - 30]);
    assert_eq!(fits.len(), cap);
    let r = client.post(&url).body(fits).send().await.unwrap();
    assert_eq!(r.status(), 201);
    let over = ceremony_message(id, &vec![1u8; cap - 21 - 30 + 1]);
    let r = client.post(&url).body(over).send().await.unwrap();
    assert_eq!(r.status(), 413);
}

#[tokio::test]
async fn ceremony_channels_are_not_listed_publicly() {
    let (base, _dir) = start().await;
    let client = reqwest::Client::new();
    let id = [0x5F; 16];
    let cid = [0x60; 16];
    let post = |path: [u8; 16], body: Vec<u8>| {
        client
            .post(format!("{base}/v1/circles/{}/messages", hex::encode(path)))
            .body(body)
            .send()
    };
    assert_eq!(
        post(id, ceremony_message(id, b"a commitment"))
            .await
            .unwrap()
            .status(),
        201
    );
    assert_eq!(post(cid, chat(cid, 0)).await.unwrap().status(), 201);
    assert_eq!(
        post(id, ceremony_message(id, b"a reveal"))
            .await
            .unwrap()
            .status(),
        201
    );
    // The listing and the feed show the circle, not the channel.
    let v: Value = client
        .get(format!("{base}/v1/circles"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let listed: Vec<&str> = v["circles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["circle_id"].as_str().unwrap())
        .collect();
    assert_eq!(listed, [hex::encode(cid)]);
    let v: Value = client
        .get(format!("{base}/v1/feed"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let fed: Vec<u64> = v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_u64().unwrap())
        .collect();
    assert_eq!(fed, [1]);
    // Whoever holds the channel id still reads it.
    let v: Value = client
        .get(format!("{base}/v1/circles/{}/messages", hex::encode(id)))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["entries"].as_array().unwrap().len(), 2);
}
