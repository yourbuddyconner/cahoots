//! `cahoots-host attest` asks a simulated badge's software chip to attest a seat key, checks it
//! against the software chip's test root, ties it to the seat key the badge describes, and says
//! it is a software chip, not hardware.

mod common;

use common::*;
use std::process::{Command, Stdio};

#[test]
fn attest_prints_and_verifies_a_soft_chip_bundle() {
    let tmp = tempfile::tempdir().unwrap();
    let (_bottom, top) = chip_ceremony(tmp.path());
    let badge_sock = tmp.path().join("badge.sock");
    let serving = badge_on(&top, &badge_sock);
    let run = Command::new(HOST)
        .args(["attest", "--sim"])
        .arg(&badge_sock)
        .args(["--circle", "bench", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The badge answers once it is serving; until then it is busy and attest asks again.
    serving.start();
    let out = run.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["circle"], "bench");
    assert_eq!(json["seat"], 1);
    assert_eq!(json["check"]["verified"], true);
    assert_eq!(json["attestation"]["freshness"], json["freshness"]);
    assert_eq!(json["check"]["public"], json["attestation"]["public"]);
    // The attested key is the seat key the badge describes, not only the key in the answer.
    assert_eq!(json["check"]["public"], json["seat_public"]);
    // A pass against the test root is a software chip, never hardware.
    assert_eq!(json["check"]["hardware"], false);
    assert_eq!(json["check"]["root"], "software chip test root");
}
