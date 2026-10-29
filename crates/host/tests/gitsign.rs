//! `cahoots-host-git-sign`, called the way git calls `gpg.ssh.program`, asks the badge through
//! the agent, shows the commit's author and subject, and writes an SSH signature that
//! `ssh-keygen -Y verify` accepts.

mod common;

use common::*;
use std::path::Path;
use std::process::{Command, Stdio};

const COMMIT: &str = "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
author Ada Lovelace <ada@example.com> 1700000000 +0000\n\
committer Ada Lovelace <ada@example.com> 1700000000 +0000\n\
\n\
Fix the parser for long names\n\
\n\
A body that is not shown.\n";

fn ssh_keygen() -> Option<&'static str> {
    [
        "/usr/bin/ssh-keygen",
        "/usr/local/bin/ssh-keygen",
        "/opt/homebrew/bin/ssh-keygen",
    ]
    .into_iter()
    .find(|p| Path::new(p).exists())
}

#[test]
fn gitsign_writes_sshsig() {
    let tmp = tempfile::tempdir().unwrap();
    let (_bottom, top) = chip_ceremony(tmp.path());
    let badge_sock = tmp.path().join("badge.sock");
    let agent_sock = tmp.path().join("agent.sock");
    let known_hosts = tmp.path().join("known_hosts");
    std::fs::write(&known_hosts, "").unwrap();
    let serving = badge_on(&top, &badge_sock);
    let _agent = spawn_agent(&badge_sock, &agent_sock, &known_hosts);
    serving.start();

    // The signing key as `ssh-add -L` prints it, in the file git passes with -f.
    let (blob, comment) = identities(&agent_sock).remove(0);
    let public = ssh_key::PublicKey::from_bytes(&blob).unwrap();
    let line = format!("{} {comment}", public.to_openssh().unwrap());
    let keyfile = tmp.path().join("key.pub");
    std::fs::write(&keyfile, format!("{line}\n")).unwrap();
    let commit = tmp.path().join("commit");
    std::fs::write(&commit, COMMIT).unwrap();

    // git: `<program> -Y sign -n git -f <key> <file>`, then it reads `<file>.sig`.
    let run = Command::new(GIT_SIGN)
        .args(["-Y", "sign", "-n", "git", "-f"])
        .arg(&keyfile)
        .arg(&commit)
        .env("SSH_AUTH_SOCK", &agent_sock)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let shown = wait_line(&top, "sign a git commit");
    assert!(shown.lines().any(|l| l == "Ada Lovelace"), "{shown}");
    assert!(
        shown.lines().any(|l| l == "Fix the parser for long names"),
        "{shown}"
    );
    press(&top, "B");
    let out = run.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let armored = std::fs::read_to_string(tmp.path().join("commit.sig")).unwrap();
    assert!(armored.starts_with("-----BEGIN SSH SIGNATURE-----\n"));
    assert!(armored.ends_with("-----END SSH SIGNATURE-----\n"));
    let sig = ssh_key::SshSig::from_pem(&armored).unwrap();
    assert_eq!(sig.namespace(), "git");
    assert_eq!(sig.hash_alg(), ssh_key::HashAlg::Sha512);
    public
        .verify("git", COMMIT.as_bytes(), &sig)
        .expect("the signature verifies");

    if let Some(keygen) = ssh_keygen() {
        let allowed = tmp.path().join("allowed_signers");
        std::fs::write(
            &allowed,
            format!("ada@example.com namespaces=\"git\" {line}\n"),
        )
        .unwrap();
        // ssh-keygen itself, and the same check through git-sign, which hands it on.
        for program in [keygen, GIT_SIGN] {
            let out = Command::new(program)
                .args(["-Y", "verify", "-n", "git", "-I", "ada@example.com", "-f"])
                .arg(&allowed)
                .arg("-s")
                .arg(tmp.path().join("commit.sig"))
                .stdin(std::fs::File::open(&commit).unwrap())
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{program}: {}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    // Refused on the badge: git-sign fails and writes no signature.
    wait_line(&top, "serving");
    let again = tmp.path().join("again");
    std::fs::write(&again, COMMIT).unwrap();
    let run = Command::new(GIT_SIGN)
        .args(["-Y", "sign", "-n", "git", "-f"])
        .arg(&keyfile)
        .arg(&again)
        .env("SSH_AUTH_SOCK", &agent_sock)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_line(&top, "sign a git commit");
    press(&top, "R");
    let out = run.wait_with_output().unwrap();
    assert!(!out.status.success());
    assert!(!tmp.path().join("again.sig").exists());
}
