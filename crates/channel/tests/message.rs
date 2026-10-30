mod common;

use cahoots_channel::keys::{kid_of, EpochId};
use cahoots_channel::message::{
    body_digest, nonce, open, seal_sign, split, verify_signature, Header, MessageType, DOMAIN,
    HEADER_LEN, VERSION,
};
use cahoots_channel::ChannelError;
use cahoots_core::cose;
use common::seat_key;
use sha2::{Digest, Sha256};

const CHANNEL: [u8; 16] = [3; 16];
const MSG_KEY: [u8; 32] = [0x5a; 32];

fn header(kid: [u8; 32], len: usize, msg_type: MessageType) -> Header {
    Header {
        channel_id: CHANNEL,
        epoch_id: EpochId {
            channel_id: CHANNEL,
            epoch: 4,
        },
        kid,
        counter: 0x0102_0304,
        len: len as u32,
        msg_type,
    }
}

#[test]
fn header_round_trips_and_is_78_bytes() {
    assert_eq!(HEADER_LEN, 78);
    for t in [
        MessageType::Text,
        MessageType::Control,
        MessageType::Announcement,
    ] {
        let h = header([0x77; 32], 300, t);
        let b = h.encode();
        assert_eq!(b.len(), 78);
        assert_eq!(b[0], VERSION);
        assert_eq!(&b[1..17], &CHANNEL);
        assert_eq!(&b[17..37], &h.epoch_id.encode());
        assert_eq!(&b[37..69], &[0x77; 32]);
        assert_eq!(&b[69..73], &0x0102_0304u32.to_le_bytes());
        assert_eq!(&b[73..77], &300u32.to_le_bytes());
        assert_eq!(b[77], t as u8);
        assert_eq!(Header::parse(&b).unwrap(), h);
    }
    let b = header([0x77; 32], 1, MessageType::Text).encode();
    assert_eq!(Header::parse(&b[..77]), Err(ChannelError::Malformed));
    let mut v2 = b;
    v2[0] = 2;
    assert_eq!(Header::parse(&v2), Err(ChannelError::Malformed));
    let mut bad_type = b;
    bad_type[77] = 3;
    assert_eq!(Header::parse(&bad_type), Err(ChannelError::Malformed));
    let mut mixed = b;
    mixed[17] ^= 1;
    assert_eq!(Header::parse(&mixed), Err(ChannelError::WrongChannel));

    let mut want = [0u8; 12];
    want[..4].copy_from_slice(&7u32.to_le_bytes());
    assert_eq!(nonce(7), want);
}

#[test]
fn seal_sign_then_verify_and_open_round_trip() {
    let member = seat_key(0);
    let kid = kid_of(&member.public());
    let plaintext = b"hello, channel";
    let h = header(kid, plaintext.len(), MessageType::Text);
    let wire = seal_sign(&h, &MSG_KEY, plaintext, &[], &member);

    let (body, cose_bytes, parsed) = split(&wire).unwrap();
    assert_eq!(parsed, h);
    assert_eq!(body.len(), HEADER_LEN + plaintext.len() + 16);
    assert_eq!(&body[..HEADER_LEN], &h.encode());
    assert_eq!(cose_bytes, &wire[body.len()..]);
    assert_eq!(cose::kid1(cose_bytes).unwrap(), kid.to_vec());
    assert_eq!(body_digest(body), <[u8; 32]>::from(Sha256::digest(body)));

    let (got, ct, approvals) = verify_signature(&wire, &member.public()).unwrap();
    assert_eq!(got, h);
    assert_eq!(ct, &body[HEADER_LEN..]);
    assert!(approvals.is_empty());
    assert_eq!(open(&got, &MSG_KEY, ct).unwrap(), plaintext);
    assert_eq!(open(&got, &[0; 32], ct), Err(ChannelError::Tag));

    // Approvals ride in the signature's payload and come back in order.
    let approvals = vec![b"first approval".to_vec(), vec![1, 2, 3]];
    let a = header(kid, 0, MessageType::Announcement);
    let wire = seal_sign(&a, &MSG_KEY, b"", &approvals, &member);
    let (_, ct, got) = verify_signature(&wire, &member.public()).unwrap();
    assert_eq!(got, approvals);
    assert_eq!(open(&a, &MSG_KEY, ct).unwrap(), b"");

    // A body shorter than the header's length, or no signature after it, does not split.
    assert_eq!(
        split(&wire[..HEADER_LEN + 15]).map(|_| ()),
        Err(ChannelError::Malformed)
    );
    assert_eq!(
        split(&wire[..HEADER_LEN + 16]).map(|_| ()),
        Err(ChannelError::Malformed)
    );
}

#[test]
fn signature_covers_body_and_approvals() {
    let member = seat_key(0);
    let kid = kid_of(&member.public());
    let h = header(kid, 5, MessageType::Announcement);
    let wire = seal_sign(&h, &MSG_KEY, b"hello", &[vec![1; 8]], &member);
    let body_len = HEADER_LEN + 5 + 16;

    // A flipped ciphertext byte, or a header byte that still parses.
    for i in [body_len - 1, HEADER_LEN, 70] {
        let mut w = wire.clone();
        w[i] ^= 1;
        assert_eq!(
            verify_signature(&w, &member.public()).map(|_| ()),
            Err(ChannelError::Signature),
            "byte {i}"
        );
    }

    // The signature is over the approvals with `DOMAIN || body` as external AAD.
    let (body, original_cose, _) = split(&wire).unwrap();
    let mut aad = DOMAIN.to_vec();
    aad.extend_from_slice(body);
    assert!(cose::verify1(&member.public(), original_cose, None, &aad).is_ok());
    assert!(cose::verify1(&member.public(), original_cose, None, body).is_err());

    // The same body with another approvals array swapped into the original signature.
    let other = seal_sign(&h, &MSG_KEY, b"hello", &[vec![2; 8]], &member);
    let (_, other_cose, _) = split(&other).unwrap();
    let payload_a = cose::payload1(original_cose).unwrap();
    let payload_b = cose::payload1(other_cose).unwrap();
    assert_eq!(payload_a.len(), payload_b.len());
    let at = original_cose
        .windows(payload_a.len())
        .position(|w| w == payload_a.as_slice())
        .unwrap();
    let mut swapped = original_cose.to_vec();
    swapped[at..at + payload_b.len()].copy_from_slice(&payload_b);
    let mut w = body.to_vec();
    w.extend_from_slice(&swapped);
    assert_eq!(
        verify_signature(&w, &member.public()).map(|_| ()),
        Err(ChannelError::Signature)
    );
}

#[test]
fn verify_refuses_another_key_and_a_kid_mismatch() {
    let member = seat_key(0);
    let stranger = seat_key(1);
    let kid = kid_of(&member.public());
    let h = header(kid, 2, MessageType::Text);
    let wire = seal_sign(&h, &MSG_KEY, b"hi", &[], &member);

    // Another key: its kid does not match the message's.
    assert_eq!(
        verify_signature(&wire, &stranger.public()).map(|_| ()),
        Err(ChannelError::Signature)
    );

    // The stranger signs under the member's kid: the kid matches, the signature does not.
    let forged = seal_sign(&h, &MSG_KEY, b"hi", &[], &stranger);
    assert_eq!(
        verify_signature(&forged, &member.public()).map(|_| ()),
        Err(ChannelError::Signature)
    );

    // A header naming one kid with a COSE kid of another.
    let mut other_kid = h;
    other_kid.kid = kid_of(&stranger.public());
    let theirs = seal_sign(&other_kid, &MSG_KEY, b"hi", &[], &stranger);
    let (_, their_cose, _) = split(&theirs).unwrap();
    let (body, _, _) = split(&wire).unwrap();
    let mut spliced = body.to_vec();
    spliced.extend_from_slice(their_cose);
    assert_eq!(
        verify_signature(&spliced, &stranger.public()).map(|_| ()),
        Err(ChannelError::Signature)
    );
}
