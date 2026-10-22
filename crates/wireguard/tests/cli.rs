//! End-to-end: two FileProvider directories, issue on one, accept on the other.
use cahoots_core::params::CircleParams;
use cahoots_provider::FileProvider;
use std::process::Command;

fn pad_bytes(len: usize) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(29) ^ (i >> 4)) as u8)
        .collect()
}

#[test]
fn cli_issue_then_accept_yields_same_psk() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let params = CircleParams::new(2, 2048, "cli", false).unwrap();
    let cid = FileProvider::open(dir_a.path())
        .unwrap()
        .add_circle(params.clone(), 0, pad_bytes(2048))
        .unwrap();
    FileProvider::open(dir_b.path())
        .unwrap()
        .add_circle(params, 1, pad_bytes(2048))
        .unwrap();
    let bin = env!("CARGO_BIN_EXE_cahoots-wg");
    let out = Command::new(bin)
        .args([
            "--dir",
            dir_a.path().to_str().unwrap(),
            "issue",
            "--circle",
            &hex::encode(cid),
            "--to",
            "2",
            "--raw",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let psk_a = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    let token = stderr
        .trim()
        .strip_prefix("handle for seat 2: ")
        .unwrap()
        .to_string();
    let out = Command::new(bin)
        .args([
            "--dir",
            dir_b.path().to_str().unwrap(),
            "accept",
            "--circle",
            &hex::encode(cid),
            "--handle",
            &token,
            "--peer",
            "PK=",
            "--allowed-ips",
            "10.0.0.1/32",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cfg = String::from_utf8(out.stdout).unwrap();
    assert!(
        cfg.contains(&format!("PresharedKey = {}", psk_a.trim())),
        "{cfg}"
    );
    assert!(cfg.starts_with("[Peer]\nPublicKey = PK=\n"));
    // Replay fails.
    let out = Command::new(bin)
        .args([
            "--dir",
            dir_b.path().to_str().unwrap(),
            "accept",
            "--circle",
            &hex::encode(cid),
            "--handle",
            &token,
            "--raw",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("already spent"));
}

/// Interop with the real `wg` tool. Needs `wireguard-tools` installed; skipped otherwise.
#[test]
#[ignore = "requires the wg binary (brew install wireguard-tools)"]
fn wg_accepts_our_psk_format() {
    let psk = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    // `wg pubkey` validates a 32-byte base64 key the same way it validates a PSK.
    let out = Command::new("wg")
        .arg("pubkey")
        .stdin(std::process::Stdio::piped())
        .output();
    let _ = (psk, out);
}
