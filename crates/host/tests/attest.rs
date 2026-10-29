//! `cahoots-host attest` asks a simulated badge's software chip to attest a seat key and checks
//! it against the roots built in, which today are the software chip's test CA.

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
}
