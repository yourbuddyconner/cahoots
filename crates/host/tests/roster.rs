//! `cahoots-host roster --ssh` prints an `authorized_keys` line for every seat in the circle's
//! sealed manifest, read from a badge's state directory.

mod common;

use cahoots_host::device::Device;
use common::*;
use std::process::Command;

#[test]
fn roster_prints_authorized_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let (bottom, top) = chip_ceremony(tmp.path());

    let out = Command::new(HOST)
        .args(["roster", "--ssh", "--circle", "bench", "--state"])
        .arg(&bottom)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");

    // The seat keys are in the chips: the state directories keep none the laptop could use.
    for dir in [&bottom, &top] {
        let d = Device::open(dir).unwrap();
        assert!(d.seat_in_chip());
        assert!(d.seat_key().is_none());
    }
    let manifest = Device::open(&top)
        .unwrap()
        .manifest()
        .unwrap()
        .verify()
        .unwrap();
    for (seat, line) in lines.iter().enumerate() {
        assert!(line.starts_with("ecdsa-sha2-nistp256 "), "{line}");
        assert!(line.ends_with(&format!(" cahoots:bench:{seat}")), "{line}");
        let key = ssh_key::PublicKey::from_openssh(line).unwrap();
        let point = key.key_data().ecdsa().unwrap().as_sec1_bytes().to_vec();
        let want = manifest.seat_key(seat as u8).unwrap();
        assert_eq!(
            p256::PublicKey::from_sec1_bytes(&point).unwrap(),
            p256::PublicKey::from_sec1_bytes(&want.0).unwrap()
        );
    }

    // Another circle's name: nothing printed, and a failure.
    let out = Command::new(HOST)
        .args(["roster", "--ssh", "--circle", "elsewhere", "--state"])
        .arg(&bottom)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
}

/// A badge keeps its first circle in `circle.txt` and the others in `circle-<k>.txt`; roster
/// finds a circle by name in any of them.
#[test]
fn roster_finds_a_circle_in_any_place() {
    let tmp = tempfile::tempdir().unwrap();
    let (bench, _) = chip_ceremony_named(&tmp.path().join("one"), "bench");
    let (lab, _) = chip_ceremony_named(&tmp.path().join("two"), "lab");
    let both = tmp.path().join("both");
    std::fs::create_dir(&both).unwrap();
    std::fs::copy(bench.join("circle.txt"), both.join("circle.txt")).unwrap();
    std::fs::copy(lab.join("circle.txt"), both.join("circle-2.txt")).unwrap();

    let roster = |circle: &str| {
        Command::new(HOST)
            .args(["roster", "--ssh", "--circle", circle, "--state"])
            .arg(&both)
            .output()
            .unwrap()
    };
    for (circle, from) in [("bench", &bench), ("lab", &lab)] {
        let out = roster(circle);
        assert!(
            out.status.success(),
            "{circle}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let text = String::from_utf8(out.stdout).unwrap();
        let alone = Command::new(HOST)
            .args(["roster", "--ssh", "--circle", circle, "--state"])
            .arg(from)
            .output()
            .unwrap();
        assert_eq!(text, String::from_utf8(alone.stdout).unwrap());
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(
            text.lines()
                .all(|l| l.contains(&format!(" cahoots:{circle}:"))),
            "{text}"
        );
    }
    // The second place's circle is read from circle-2.txt.
    let lab_manifest = cahoots_host::roster::manifest(&both, "lab").unwrap();
    assert_eq!(lab_manifest.params.name, "lab");

    let out = roster("elsewhere");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("bench, lab"), "{err}");
}
