//! An agent's grant on the laptop: `cahoots-host grant request` has the badge sign a grant for
//! the agent's key after its holder allows it; then `cahoots-host-git-sign`, through the
//! agent, signs commits under the grant with no press while the badge counts down, and once
//! the grant is spent the badge asks as it always did.

mod common;

use common::*;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const COMMIT: &str = "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
author Ada Lovelace <ada@example.com> 1700000000 +0000\n\
committer Ada Lovelace <ada@example.com> 1700000000 +0000\n\
\n\
Fix the parser for long names\n\
\n\
A body that is not shown.\n";

/// `cahoots-host args` with the agent's files in `home`.
fn host(home: &Path, args: &[&str]) -> Output {
    Command::new(HOST)
        .args(args)
        .env("CAHOOTS_HOME", home)
        .output()
        .unwrap()
}

/// `cahoots-host-git-sign -Y sign -n git -f <key> <file>` in `dir`, through the agent.
fn git_sign(dir: &Path, agent_sock: &Path, keyfile: &Path, file: &Path) -> std::process::Child {
    Command::new(GIT_SIGN)
        .args(["-Y", "sign", "-n", "git", "-f"])
        .arg(keyfile)
        .arg(file)
        .current_dir(dir)
        .env("SSH_AUTH_SOCK", agent_sock)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// The agent, as `spawn_agent` starts it, with its files in `home`.
fn spawn_agent_in(home: &Path, badge_sock: &Path, agent_sock: &Path, known_hosts: &Path) -> Kill {
    let child = Command::new(HOST)
        .args(["agent", "--sim"])
        .arg(badge_sock)
        .args(["--circle", "bench", "--socket"])
        .arg(agent_sock)
        .arg("--known-hosts")
        .arg(known_hosts)
        .env("CAHOOTS_HOME", home)
        .spawn()
        .unwrap();
    let kill = Kill(child);
    let deadline = Instant::now() + Duration::from_secs(30);
    while std::os::unix::net::UnixStream::connect(agent_sock).is_err() {
        assert!(Instant::now() < deadline, "the agent never listened");
        std::thread::sleep(Duration::from_millis(20));
    }
    kill
}

#[test]
fn an_agent_signs_under_a_grant_until_it_is_spent() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let (_bottom, top) = chip_ceremony(tmp.path());
    let badge_sock = tmp.path().join("badge.sock");

    // The grant: the badge asks its holder, who allows it.
    let grant_file = {
        let serving = badge_on(&top, &badge_sock);
        let child = Command::new(HOST)
            .args(["grant", "request", "--sim"])
            .arg(&badge_sock)
            .args([
                "--circle",
                "bench",
                "--agent",
                "claude",
                "--author",
                "Ada Lovelace <ada@example.com>",
                "--budget",
                "2",
                "--until",
                "2030-01-01",
            ])
            .env("CAHOOTS_HOME", &home)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        serving.start();
        let shown = wait_line(&top, "let an agent sign");
        assert!(shown.lines().any(|l| l.starts_with("claude")), "{shown}");
        assert!(shown.lines().any(|l| l == "2 of them"), "{shown}");
        press(&top, "B");
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let path = String::from_utf8_lossy(&out.stdout).lines().last().unwrap().trim().to_string();
        assert!(path.ends_with(".cwt"), "{path}");
        assert!(Path::new(&path).exists(), "{path}");
        drop(serving);
        path
    };
    let key = std::fs::read_to_string(home.join("agent.key")).unwrap();
    assert_eq!(key.trim().len(), 64, "the agent's key, in hex");
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(home.join("agent.key")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let shown = host(&home, &["grant", "show", &grant_file]);
    assert!(shown.status.success(), "{}", String::from_utf8_lossy(&shown.stderr));
    let json = String::from_utf8_lossy(&shown.stdout);
    assert!(json.contains("\"budget\": 2"), "{json}");
    assert!(json.contains("Ada Lovelace <ada@example.com>"), "{json}");
    assert!(json.contains("\"exp\": 1893456000"), "midnight UTC on 1 Jan 2030: {json}");
    let verified = host(
        &home,
        &["grant", "verify", &grant_file, "--state", top.to_str().unwrap(), "--circle", "bench"],
    );
    assert!(verified.status.success(), "{}", String::from_utf8_lossy(&verified.stderr));

    // The agent, on the badge started again (a simulated badge takes one laptop in its life).
    let agent_sock = tmp.path().join("agent.sock");
    let known_hosts = tmp.path().join("known_hosts");
    std::fs::write(&known_hosts, "").unwrap();
    let serving = badge_on(&top, &badge_sock);
    let _agent = spawn_agent_in(&home, &badge_sock, &agent_sock, &known_hosts);
    serving.start();
    let (blob, comment) = identities(&agent_sock).remove(0);
    let public = ssh_key::PublicKey::from_bytes(&blob).unwrap();
    let keyfile = tmp.path().join("key.pub");
    std::fs::write(&keyfile, format!("{} {comment}\n", public.to_openssh().unwrap())).unwrap();
    wait_line(&top, "1/5");

    // Two commits under the grant: no press, and the badge counts down.
    for (name, left) in [("one", "1 left of 2"), ("two", "0 left of 2")] {
        let commit = tmp.path().join(name);
        std::fs::write(&commit, COMMIT).unwrap();
        let out = git_sign(tmp.path(), &agent_sock, &keyfile, &commit)
            .wait_with_output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        // The badge answers before it redraws, so wait for this commit's count itself.
        let screen = wait_screen(&top, |s| {
            s.lines().any(|l| l == "signed for claude") && s.lines().any(|l| l == left)
        });
        assert!(screen.lines().any(|l| l == "Fix the parser for long names"), "{screen}");
        let armored = std::fs::read_to_string(tmp.path().join(format!("{name}.sig"))).unwrap();
        let sig = ssh_key::SshSig::from_pem(&armored).unwrap();
        public
            .verify("git", COMMIT.as_bytes(), &sig)
            .expect("the signature verifies");
    }

    // The badge refuses a use the laptop thought it had left (here, a count out of step with
    // the badge's): the signature fails once, nobody is asked, and the grant is left alone.
    let meta = Path::new(&grant_file).with_extension("json");
    let spent = std::fs::read_to_string(&meta).unwrap();
    assert!(spent.contains("\"remaining\": 0"), "{spent}");
    std::fs::write(&meta, spent.replace("\"remaining\": 0", "\"remaining\": 1")).unwrap();
    let refused = tmp.path().join("refused");
    std::fs::write(&refused, COMMIT).unwrap();
    let out = git_sign(tmp.path(), &agent_sock, &keyfile, &refused)
        .wait_with_output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!tmp.path().join("refused.sig").exists());
    let screen = std::fs::read_to_string(top.join("screen.txt")).unwrap();
    assert!(!screen.lines().any(|l| l == "sign a git commit"), "{screen}");
    let after = std::fs::read_to_string(&meta).unwrap();
    assert!(after.contains("\"remaining\": 1"), "the grant is left as it was: {after}");
    std::fs::write(&meta, spent).unwrap();

    // Spent: the third commit is asked about as any other.
    let commit = tmp.path().join("three");
    std::fs::write(&commit, COMMIT).unwrap();
    let run = git_sign(tmp.path(), &agent_sock, &keyfile, &commit);
    let shown = wait_line(&top, "sign a git commit");
    assert!(shown.lines().any(|l| l == "Ada Lovelace"), "{shown}");
    press(&top, "B");
    let out = run.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let armored = std::fs::read_to_string(tmp.path().join("three.sig")).unwrap();
    let sig = ssh_key::SshSig::from_pem(&armored).unwrap();
    public
        .verify("git", COMMIT.as_bytes(), &sig)
        .expect("the signature verifies");

    let listed = host(&home, &["grant", "list"]);
    assert!(listed.status.success(), "{}", String::from_utf8_lossy(&listed.stderr));
    let text = String::from_utf8_lossy(&listed.stdout);
    assert!(text.contains("claude") && text.contains("0 left of 2"), "{text}");
}
