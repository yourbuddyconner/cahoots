//! PSK derivation vectors (handoff 10.1), published to `vectors/psk.json`.
//! The file is written if missing and compared if present. Change the derivation and
//! this test fails until the file is regenerated (delete it and rerun).

use cahoots_provider::{derive_session_key, PskHandle};
use serde_json::{json, Value};

fn vectors() -> Value {
    let cases: Vec<Value> = [
        (
            [0x11u8; 32],
            [0xAAu8; 16],
            0u8,
            0u32,
            &b"hpke"[..],
            &b""[..],
            &b""[..],
        ),
        (
            [0x22; 32],
            [0xBB; 16],
            3,
            0x0102_0304,
            b"tls13",
            b"ecdhe-secret",
            b"mlkem-secret",
        ),
        ([0x33; 32], [0xCC; 16], 1, 32, b"wireguard", b"", b""),
    ]
    .iter()
    .map(|&(psk, cid, slice, offset, ctx, classical, pq)| {
        let h = PskHandle {
            circle_id: cid,
            slice,
            offset,
        };
        json!({
            "psk": hex::encode(psk),
            "circle_id": hex::encode(cid),
            "slice": slice,
            "offset": offset,
            "handle_bytes": hex::encode(h.to_bytes()),
            "context": String::from_utf8_lossy(ctx),
            "classical_secret": hex::encode(classical),
            "pq_secret": hex::encode(pq),
            "session_key": hex::encode(derive_session_key(&psk, &h, ctx, classical, pq)),
        })
    })
    .collect();
    json!({
        "description": "session_key = HKDF-SHA256(salt = \"cahoots/psk/v1\" || circle_id || slice || offset (u32 LE) || context, ikm = classical_secret || pq_secret || psk, info = \"cahoots/session-key/v1\", L = 32).",
        "cases": cases,
    })
}

#[test]
fn psk_vectors_are_stable() {
    let v = vectors();
    assert_eq!(
        v["cases"][0]["session_key"],
        "1116fa567832c19e988ae1a81ac5a9ab0ea5fd633e67e19e8d847d51720c3b94"
    );
}

#[test]
fn psk_vectors_file_is_current() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../vectors/psk.json");
    let text = serde_json::to_string_pretty(&vectors()).unwrap() + "\n";
    match std::fs::read_to_string(&path) {
        Ok(existing) => assert_eq!(
            existing, text,
            "vectors/psk.json is stale; delete and rerun"
        ),
        Err(_) => std::fs::write(&path, text).unwrap(),
    }
}
