//! `cahoots-host sign`: a file signed by the badge's seat key through the agent. A small text
//! file is shown to the badge (its first line and size); a big one is signed by its hash,
//! which the badge shows marked as the laptop's word. Both verify with `ssh-keygen -Y verify`.

mod common;

use common::*;
use std::path::Path;
use std::process::{Command, Stdio};

fn ssh_keygen() -> Option<&'static str> {
    [
        "/usr/bin/ssh-keygen",
        "/usr/local/bin/ssh-keygen",
        "/opt/homebrew/bin/ssh-keygen",
    ]
    .into_iter()
    .find(|p| Path::new(p).exists())
}

fn verify(keygen: &str, allowed: &Path, namespace: &str, sig: &Path, file: &Path) -> bool {
    Command::new(keygen)
        .args(["-Y", "verify", "-n", namespace, "-I", "ada@example.com", "-f"])
        .arg(allowed)
        .arg("-s")
        .arg(sig)
        .stdin(std::fs::File::open(file).unwrap())
        .output()
        .unwrap()
        .status
        .success()
}

#[test]
fn sign_shows_a_small_file_and_hashes_a_big_one() {
    let tmp = tempfile::tempdir().unwrap();
    let (_bottom, top) = chip_ceremony(tmp.path());
    let badge_sock = tmp.path().join("badge.sock");
    let agent_sock = tmp.path().join("agent.sock");
    let known_hosts = tmp.path().join("known_hosts");
    std::fs::write(&known_hosts, "").unwrap();
    let serving = badge_on(&top, &badge_sock);
    let _agent = spawn_agent(&badge_sock, &agent_sock, &known_hosts);
    serving.start();
    let (blob, _) = identities(&agent_sock).remove(0);
    let public = ssh_key::PublicKey::from_bytes(&blob).unwrap();
    let allowed = tmp.path().join("allowed_signers");
    std::fs::write(
        &allowed,
        format!("ada@example.com namespaces=\"file,release\" {}\n", public.to_openssh().unwrap()),
    )
    .unwrap();

    // A text file: the badge shows its first line and its size.
    let small = tmp.path().join("notes.txt");
    std::fs::write(&small, "Release notes for 1.4\n\nFixed the thing.\n").unwrap();
    let run = Command::new(HOST)
        .args(["sign", "--file"])
        .arg(&small)
        .args(["--circle", "bench"])
        .env("SSH_AUTH_SOCK", &agent_sock)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let shown = wait_line(&top, "sign a file");
    assert!(shown.lines().any(|l| l == "in file"), "{shown}");
    assert!(shown.lines().any(|l| l == "40 bytes"), "{shown}");
    assert!(shown.lines().any(|l| l == "Release notes for 1.4"), "{shown}");
    assert!(!shown.contains("(laptop)"), "{shown}");
    press(&top, "B");
    let out = run.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let sig_path = tmp.path().join("notes.txt.sig");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), sig_path.display().to_string());
    let sig = ssh_key::SshSig::from_pem(std::fs::read_to_string(&sig_path).unwrap()).unwrap();
    assert_eq!(sig.namespace(), "file");
    public
        .verify("file", &std::fs::read(&small).unwrap(), &sig)
        .expect("the signature verifies");

    // A big binary file, in a namespace of the caller's choosing: signed by its hash, which
    // the badge shows as the laptop's word.
    wait_line(&top, "1/5");
    let big = tmp.path().join("build.bin");
    let bytes: Vec<u8> = (0..40_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
    std::fs::write(&big, &bytes).unwrap();
    let run = Command::new(HOST)
        .args(["sign", "--file"])
        .arg(&big)
        .args(["--circle", "bench", "--namespace", "release"])
        .env("SSH_AUTH_SOCK", &agent_sock)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let shown = wait_line(&top, "sign a file");
    assert!(shown.lines().any(|l| l == "in release"), "{shown}");
    assert!(shown.lines().any(|l| l == "size not sent"), "{shown}");
    assert!(shown.contains("sha512 ") && shown.contains("(laptop)"), "{shown}");
    press(&top, "B");
    let out = run.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("too big to show"));
    let sig_path = tmp.path().join("build.bin.sig");
    let sig = ssh_key::SshSig::from_pem(std::fs::read_to_string(&sig_path).unwrap()).unwrap();
    public.verify("release", &bytes, &sig).expect("the signature verifies");

    if let Some(keygen) = ssh_keygen() {
        assert!(verify(keygen, &allowed, "file", &tmp.path().join("notes.txt.sig"), &small));
        assert!(verify(keygen, &allowed, "release", &sig_path, &big));
        // The wrong namespace does not verify.
        assert!(!verify(keygen, &allowed, "file", &sig_path, &big));
    }

    // Refused on the badge: no signature.
    wait_line(&top, "1/5");
    let other = tmp.path().join("other.txt");
    std::fs::write(&other, "do not sign\n").unwrap();
    let run = Command::new(HOST)
        .args(["sign", "--file"])
        .arg(&other)
        .args(["--circle", "bench"])
        .env("SSH_AUTH_SOCK", &agent_sock)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_line(&top, "sign a file");
    press(&top, "A");
    let out = run.wait_with_output().unwrap();
    assert!(!out.status.success());
    assert!(!tmp.path().join("other.txt.sig").exists());
}
