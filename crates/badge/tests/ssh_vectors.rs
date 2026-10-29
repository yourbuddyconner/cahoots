//! Rebuilds `crates/vectors/ssh.json`: for each request to sign for ssh or git, the badge shows
//! the screen in the vector and returns the signature in it.

use cahoots_badge::apps::BadgeApps;
use cahoots_badge::{ssh, Circle};
use cahoots_core::params::CircleParams;
use cahoots_core::seat::SeatKey;
use cahoots_custodian::contract::ChipApps;
use cahoots_custodian::wire::{ChipRequest, SshHost};
use serde_json::Value;

fn bytes(x: &Value) -> Vec<u8> {
    hex::decode(x.as_str().unwrap()).unwrap()
}

#[test]
fn the_ssh_vectors_are_reproduced() {
    let v: Value = serde_json::from_str(include_str!("../../vectors/ssh.json")).unwrap();
    let seed: [u8; 32] = bytes(&v["seat_seed"]).try_into().unwrap();
    let seat = SeatKey::from_seed(&seed).unwrap();
    assert_eq!(seat.public().0.to_vec(), bytes(&v["seat_public"]));
    assert_eq!(ssh::public_key_blob(&seat.public()), bytes(&v["seat_blob"]));
    let circle_id: [u8; 16] = bytes(&v["circle_id"]).try_into().unwrap();
    let circles = [Circle {
        params: CircleParams::new(2, 192, v["circle"].as_str().unwrap(), false).unwrap(),
        seat: v["seat"].as_u64().unwrap() as u8,
        circle_id,
        words: [0; 6],
        seat_key: seed,
        chip_slot: None,
        root: None,
        tier: 0,
        manifest: Vec::new(),
    }];
    let cases = v["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    for c in cases {
        let data = bytes(&c["data"]);
        let host = (!c["host"].is_null()).then(|| SshHost {
            host_key: bytes(&c["host"]["host_key"]),
            session_id: bytes(&c["host"]["session_id"]),
            host_sig: bytes(&c["host"]["host_sig"]),
            name: c["host"]["name"].as_str().unwrap().into(),
            user: c["host"]["user"].as_str().unwrap().into(),
        });
        let content = (!c["content"].is_null()).then(|| bytes(&c["content"]));
        let op = ChipRequest::SshSign {
            data: data.clone(),
            host: host.clone(),
            content: content.clone(),
        };
        let mut apps = BadgeApps::new(&circles, None);
        let screen = apps.prompt(&circle_id, &op).unwrap().unwrap();
        let want: Vec<&str> = c["screen"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap())
            .collect();
        assert_eq!(screen.lines, want, "{}", c["case"]);
        let sig = apps
            .ssh_sign(&circle_id, &data, host.as_ref(), content.as_deref())
            .unwrap();
        assert_eq!(sig, bytes(&c["signature"]), "{}", c["case"]);
    }
}
