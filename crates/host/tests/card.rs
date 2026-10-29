//! `cahoots-host card` and `grant` against a simulated badge with a software chip: the badge
//! asks before it signs a card, releases the card seal key, or signs a grant.

mod common;

use cahoots_objects::grant::verify_chain;
use common::*;
use std::path::Path;
use std::process::{Command, Stdio};

/// Run `cahoots-host args` against the badge kept in `dir`, started fresh on `sock` (a
/// simulated badge takes one laptop connection in its life), and answer each prompt whose
/// line `answers` names with both buttons, after `check` has read its screen.
fn session(
    dir: &Path,
    sock: &Path,
    args: &[&str],
    answers: &[&str],
    check: impl Fn(&str, &str),
) -> std::process::Output {
    let serving = badge_on(dir, sock);
    let child = Command::new(HOST)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    serving.start();
    for line in answers {
        let screen = wait_line(dir, line);
        check(line, &screen);
        press(dir, "B");
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

#[test]
fn card_set_fetch_and_grant_v2() {
    let tmp = tempfile::tempdir().unwrap();
    let (bottom, top) = chip_ceremony(tmp.path());
    let badge_sock = tmp.path().join("badge.sock");
    let sock = badge_sock.to_str().unwrap();
    let sealed = tmp.path().join("ada.card");
    let path = sealed.to_str().unwrap();

    // The badge shows the card's name and counts, then the seal key's label.
    session(
        &top,
        &badge_sock,
        &[
            "card",
            "set",
            "--sim",
            sock,
            "--circle",
            "bench",
            "--name",
            "Ada",
            "--handle",
            "email=ada@example.com",
            "--handle",
            "github=ada",
            "--endpoint",
            "ssh=ada.example.com",
            "--out",
            path,
        ],
        &["sign your card", "key for seal/card"],
        |line, screen| match line {
            "sign your card" => {
                assert!(screen.lines().any(|l| l == "Ada"), "{screen}");
                assert!(
                    screen.lines().any(|l| l == "2 handles, 1 endpoint"),
                    "{screen}"
                );
            }
            _ => assert!(screen.lines().any(|l| l == "release a key"), "{screen}"),
        },
    );
    assert!(sealed.exists());

    // Fetch opens it with the seal key and checks it against the manifest the bottom card
    // keeps.
    let out_dir = tmp.path().join("cards");
    std::fs::create_dir(&out_dir).unwrap();
    let out = session(
        &top,
        &badge_sock,
        &[
            "card",
            "fetch",
            "--sim",
            sock,
            "--circle",
            "bench",
            "--state",
            bottom.to_str().unwrap(),
            "--in",
            sealed.to_str().unwrap(),
            "--out",
            out_dir.to_str().unwrap(),
        ],
        &["key for seal/card"],
        |_, _| {},
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["name"], "Ada");
    assert_eq!(json["seat"], 1);
    assert_eq!(json["handles"][1][1], "ada");
    let shown = Command::new(HOST)
        .args([
            "card",
            "show",
            "--circle",
            "bench",
            "--state",
            bottom.to_str().unwrap(),
        ])
        .arg(out_dir.join("1.card"))
        .output()
        .unwrap();
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let shown: serde_json::Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(shown, json);

    // A circle kept in a later place (`circle-<k>.txt`, as a badge with several circles keeps
    // them) is found as well.
    let later = tmp.path().join("later");
    std::fs::create_dir(&later).unwrap();
    std::fs::copy(bottom.join("circle.txt"), later.join("circle-5.txt")).unwrap();
    let shown = Command::new(HOST)
        .args(["card", "show", "--circle", "bench", "--state"])
        .arg(&later)
        .arg(out_dir.join("1.card"))
        .output()
        .unwrap();
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&shown.stdout).unwrap(),
        json
    );

    // A version 2 grant, signed by the chip's seat key, verifies against the manifest.
    let grantee = cahoots_core::seat::SeatKey::from_seed(&[0x44; 32])
        .unwrap()
        .public();
    let to: String = grantee.0.iter().map(|b| format!("{b:02x}")).collect();
    let signed = tmp.path().join("grant.cose");
    session(
        &top,
        &badge_sock,
        &[
            "grant",
            "--sim",
            sock,
            "--circle",
            "bench",
            "--to",
            &to,
            "--purpose",
            "wireguard",
            "--purpose",
            "tls",
            "--budget",
            "4096",
            "--until",
            "1000",
            "--depth",
            "1",
            "--v2",
            "--out",
            signed.to_str().unwrap(),
        ],
        &["until log entry 1000 (laptop)"],
        |_, screen| {
            assert!(
                screen.lines().any(|l| l == "for wireguard, tls"),
                "{screen}"
            );
            assert!(screen.lines().any(|l| l == "may delegate"), "{screen}");
        },
    );
    let signed = std::fs::read(&signed).unwrap();
    let manifest = cahoots_host::roster::manifest(&bottom, "bench").unwrap();
    let g = verify_chain(&[&signed], &manifest).unwrap();
    assert_eq!(g.grantee, grantee);
    assert_eq!(g.purposes, [0, 1]);
    assert_eq!((g.budget, g.not_after, g.depth), (4096, 1000, 1));
}
