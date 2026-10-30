//! `cahoots-host agent` against a simulated badge whose seat key is in a software chip: it
//! lists the seat key, forwards each signature request to the badge, answers only what the
//! holder allowed, and names the host from `known_hosts` when OpenSSH binds the session.

mod common;

use cahoots_host::device::Device;
use common::*;
use rand_core::OsRng;
use signature::Verifier;
use ssh_encoding::{Decode, Encode};
use ssh_key::{Algorithm, PrivateKey};
use std::os::unix::net::UnixStream;

const SESSION: &[u8] = &[0x5a; 32];

/// The userauth request OpenSSH asks an agent to sign for `key` (an SSH key blob).
fn userauth(session_id: &[u8], user: &str, key: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    put_string(&mut v, session_id);
    v.push(50);
    put_string(&mut v, user.as_bytes());
    put_string(&mut v, b"ssh-connection");
    put_string(&mut v, b"publickey");
    v.push(1);
    put_string(&mut v, b"ecdsa-sha2-nistp256");
    put_string(&mut v, key);
    v
}

fn sign_request(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = vec![13];
    put_string(&mut m, key);
    put_string(&mut m, data);
    m.extend_from_slice(&0u32.to_be_bytes());
    m
}

/// Ask the agent to sign on a thread, since the answer waits for the buttons.
fn sign_later(
    mut s: UnixStream,
    key: Vec<u8>,
    data: Vec<u8>,
) -> std::thread::JoinHandle<(UnixStream, Vec<u8>)> {
    std::thread::spawn(move || {
        let answer = agent_call(&mut s, &sign_request(&key, &data));
        (s, answer)
    })
}

/// The signature in a sign response verifies over `data` under the key blob `key`.
fn check_signature(answer: &[u8], key: &[u8], data: &[u8]) {
    assert_eq!(answer[0], 14, "a sign response, not {answer:?}");
    let mut b = &answer[1..];
    let sig = take_string(&mut b);
    assert!(b.is_empty());
    let sig = ssh_key::Signature::decode(&mut &sig[..]).unwrap();
    let key = ssh_key::public::KeyData::decode(&mut &key[..]).unwrap();
    key.verify(data, &sig).expect("the signature verifies");
}

#[test]
fn agent_lists_seat_key_and_signs() {
    let tmp = tempfile::tempdir().unwrap();
    let (_bottom, top) = chip_ceremony(tmp.path());
    let badge_sock = tmp.path().join("badge.sock");
    let agent_sock = tmp.path().join("agent.sock");
    let known_hosts = tmp.path().join("known_hosts");
    std::fs::write(&known_hosts, "").unwrap();
    let serving = badge_on(&top, &badge_sock);
    let _agent = spawn_agent(&badge_sock, &agent_sock, &known_hosts);
    serving.start();

    // One identity: the top card's seat key, as the manifest lists it, named for its seat.
    let ids = identities(&agent_sock);
    assert_eq!(ids.len(), 1);
    let (key, comment) = &ids[0];
    assert_eq!(comment, "cahoots:bench:1");
    let manifest = Device::open(&top)
        .unwrap()
        .manifest()
        .unwrap()
        .verify()
        .unwrap();
    let seat = manifest.seat_key(1).unwrap();
    let parsed = ssh_key::public::KeyData::decode(&mut &key[..]).unwrap();
    let point = parsed.ecdsa().unwrap().as_sec1_bytes().to_vec();
    let want = p256::PublicKey::from_sec1_bytes(&seat.0).unwrap();
    assert_eq!(p256::PublicKey::from_sec1_bytes(&point).unwrap(), want);

    // Allowed: the badge shows the login with no host named, and the signature verifies.
    let data = userauth(SESSION, "git", key);
    let s = UnixStream::connect(&agent_sock).unwrap();
    let pending = sign_later(s, key.clone(), data.clone());
    let shown = wait_line(&top, "ssh login");
    assert!(shown.lines().any(|l| l == "as git"), "{shown}");
    assert!(shown.lines().any(|l| l == "host not named"), "{shown}");
    press(&top, "B");
    let (s, answer) = pending.join().unwrap();
    check_signature(&answer, key, &data);

    // Refused: a failure, and no signature.
    wait_line(&top, "1/5");
    let pending = sign_later(s, key.clone(), data.clone());
    wait_line(&top, "ssh login");
    press(&top, "L");
    let (mut s, answer) = pending.join().unwrap();
    assert_eq!(answer, vec![5]);

    // A key the agent does not hold: refused without asking.
    wait_line(&top, "1/5");
    let other = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
    let other = other.public_key().to_bytes().unwrap();
    assert_eq!(agent_call(&mut s, &sign_request(&other, &data)), vec![5]);
    // Anything else the agent is asked: a failure.
    assert_eq!(agent_call(&mut s, &[17]), vec![5]);
    assert_eq!(agent_call(&mut s, &[19]), vec![5]);
}

#[test]
fn agent_forwards_session_bind() {
    let tmp = tempfile::tempdir().unwrap();
    let (_bottom, top) = chip_ceremony(tmp.path());
    let badge_sock = tmp.path().join("badge.sock");
    let agent_sock = tmp.path().join("agent.sock");

    // The server's host key, in known_hosts under github.com.
    let host = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
    let known_hosts = tmp.path().join("known_hosts");
    std::fs::write(
        &known_hosts,
        format!(
            "# a comment\nexample.org ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl\n{}\n",
            host.public_key().to_openssh().unwrap().replace("ssh-ed25519", "github.com ssh-ed25519")
        ),
    )
    .unwrap();

    let serving = badge_on(&top, &badge_sock);
    let _agent = spawn_agent(&badge_sock, &agent_sock, &known_hosts);
    serving.start();
    let ids = identities(&agent_sock);
    let key = &ids[0].0;

    // OpenSSH binds the session: the host key, the session id, the server's signature over
    // it, and whether the connection is forwarded.
    let host_sig: ssh_key::Signature = signature::Signer::sign(&host, SESSION);
    let mut bind = vec![27];
    put_string(&mut bind, b"session-bind@openssh.com");
    put_string(&mut bind, &host.public_key().to_bytes().unwrap());
    put_string(&mut bind, SESSION);
    let mut sig = Vec::new();
    host_sig.encode(&mut sig).unwrap();
    put_string(&mut bind, &sig);
    bind.push(0);
    let mut s = UnixStream::connect(&agent_sock).unwrap();
    assert_eq!(agent_call(&mut s, &bind), vec![6]);
    // An extension the agent does not know: a failure.
    let mut unknown = vec![27];
    put_string(&mut unknown, b"no-such-extension@example.org");
    assert_eq!(agent_call(&mut s, &unknown), vec![5]);

    let data = userauth(SESSION, "git", key);
    let pending = sign_later(s, key.clone(), data.clone());
    let shown = wait_line(&top, "to github.com (laptop)");
    assert!(shown.lines().any(|l| l == "as git"), "{shown}");
    press(&top, "B");
    let (_s, answer) = pending.join().unwrap();
    check_signature(&answer, key, &data);
}
