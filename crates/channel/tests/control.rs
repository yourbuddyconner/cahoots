use cahoots_channel::control::Control;
use cahoots_channel::keys::EpochId;
use cahoots_channel::ChannelError;

fn every_kind() -> Vec<Control> {
    vec![
        Control::Rotation {
            epoch_id: EpochId {
                channel_id: [3; 16],
                epoch: 9,
            },
            deposits: vec![[1; 32], [2; 32]],
        },
        Control::Rotation {
            epoch_id: EpochId {
                channel_id: [3; 16],
                epoch: 10,
            },
            deposits: vec![],
        },
        Control::PolicyChange { policy_id: [4; 32] },
        Control::Revocation {
            grant_id: [5; 16],
            reason: "left the team".into(),
        },
        Control::Join {
            grant_id: [6; 16],
            xwing_pk: vec![7; 1216],
            name: "Ada".into(),
        },
        Control::Leave,
    ]
}

#[test]
fn every_control_kind_round_trips() {
    for c in every_kind() {
        let bytes = c.encode();
        assert_eq!(Control::decode(&bytes).unwrap(), c);
    }
    // Leave is the map {0: 4}.
    assert_eq!(Control::Leave.encode(), [0xa1, 0x00, 0x04]);
    // Policy change is {0: 1, 1: h'04..'}.
    let pc = Control::PolicyChange { policy_id: [4; 32] }.encode();
    assert_eq!(&pc[..5], &[0xa2, 0x00, 0x01, 0x01, 0x58]);
    assert_eq!(pc[5], 32);
}

#[test]
fn control_refuses_unknown_kind_and_trailing_bytes() {
    assert_eq!(
        Control::decode(&[0xa1, 0x00, 0x05]),
        Err(ChannelError::Malformed)
    );
    for c in every_kind() {
        let mut bytes = c.encode();
        bytes.push(0);
        assert_eq!(Control::decode(&bytes), Err(ChannelError::Malformed));
    }
    // An indefinite-length map, or a non-shortest kind, is not canonical.
    assert_eq!(
        Control::decode(&[0xbf, 0x00, 0x04, 0xff]),
        Err(ChannelError::NotCanonical)
    );
    assert_eq!(
        Control::decode(&[0xa1, 0x00, 0x18, 0x04]),
        Err(ChannelError::NotCanonical)
    );
    // Leave with an extra key, and a join name of 0 or 65 bytes.
    assert_eq!(
        Control::decode(&[0xa2, 0x00, 0x04, 0x01, 0x00]),
        Err(ChannelError::Malformed)
    );
    for name in [String::new(), "x".repeat(65)] {
        let bytes = Control::Join {
            grant_id: [6; 16],
            xwing_pk: vec![7; 4],
            name,
        }
        .encode();
        assert_eq!(Control::decode(&bytes), Err(ChannelError::Malformed));
    }
    let ok = Control::Join {
        grant_id: [6; 16],
        xwing_pk: vec![7; 4],
        name: "x".repeat(64),
    };
    assert_eq!(Control::decode(&ok.encode()).unwrap(), ok);
    // A deposit id of the wrong length.
    let mut rot = Control::Rotation {
        epoch_id: EpochId {
            channel_id: [3; 16],
            epoch: 1,
        },
        deposits: vec![[1; 32]],
    }
    .encode();
    // {0: 0, 1: h'..20..', 2: [h'..32..']}: shorten the last bstr's length byte.
    let n = rot.len();
    rot[n - 33] = 31;
    rot.pop();
    assert_eq!(Control::decode(&rot), Err(ChannelError::Malformed));
    // Not a map.
    assert_eq!(Control::decode(&[0x80]), Err(ChannelError::Malformed));
}
