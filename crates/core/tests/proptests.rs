//! Property tests. The decoder is also the fuzz surface: `parse` and `open` must never
//! panic on arbitrary bytes, and any edit to a sealed message must fail the tag.

use cahoots_core::frame;
use cahoots_core::message::{open, parse, seal, MessageError, MessageType, HEADER_LEN, TAG_LEN};
use cahoots_core::pad::{bucket_pad, bucket_unpad};
use proptest::prelude::*;

const CID: [u8; 16] = [0x11; 16];

proptest! {
    #[test]
    fn parse_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..600)) {
        let _ = parse(&bytes);
    }

    #[test]
    fn open_never_panics_on_arbitrary_wire_and_pad(
        bytes in proptest::collection::vec(any::<u8>(), 0..300),
        pad in proptest::collection::vec(any::<u8>(), 0..300),
    ) {
        if let Ok((h, ct, tag)) = parse(&bytes) {
            let _ = open(&h, ct, tag, &pad);
        }
    }

    #[test]
    fn seal_open_roundtrip(
        pt in proptest::collection::vec(any::<u8>(), 0..512),
        pad in proptest::collection::vec(any::<u8>(), 32 + 512),
        slice in 0u8..9,
        offset in any::<u32>(),
    ) {
        let range = &pad[..32 + pt.len()];
        let wire = seal(&CID, slice, offset, MessageType::Text, &pt, range).unwrap();
        prop_assert_eq!(wire.len(), HEADER_LEN + pt.len() + TAG_LEN);
        let (h, ct, tag) = parse(&wire).unwrap();
        prop_assert_eq!(open(&h, ct, tag, range).unwrap(), pt);
    }

    #[test]
    fn any_single_byte_edit_is_rejected(
        pt in proptest::collection::vec(any::<u8>(), 1..128),
        pad in proptest::collection::vec(any::<u8>(), 32 + 128),
        pos in any::<prop::sample::Index>(),
        delta in 1u8..=255,
    ) {
        let range = &pad[..32 + pt.len()];
        let wire = seal(&CID, 0, 0, MessageType::Text, &pt, range).unwrap();
        let mut edited = wire.clone();
        let i = pos.index(edited.len());
        edited[i] ^= delta;
        match parse(&edited) {
            Ok((h, ct, tag)) => prop_assert_eq!(open(&h, ct, tag, range), Err(MessageError::Tag)),
            Err(e) => prop_assert!(matches!(e, MessageError::Version | MessageError::Type | MessageError::Length)),
        }
    }

    #[test]
    fn frame_roundtrip_and_decode_never_panics(
        kind in any::<u8>(), seq in any::<u32>(),
        payload in proptest::collection::vec(any::<u8>(), 0..256),
        junk in proptest::collection::vec(any::<u8>(), 0..64),
    ) {
        let f = frame::Frame { kind, seq, payload: &payload };
        let bytes = frame::encode(&f);
        let (parsed, used) = frame::decode(&bytes).unwrap();
        prop_assert_eq!(parsed, f);
        prop_assert_eq!(used, bytes.len());
        let _ = frame::decode(&junk);
    }

    #[test]
    fn bucket_pad_roundtrip(pt in proptest::collection::vec(any::<u8>(), 0..256)) {
        let p = bucket_pad(&pt).unwrap();
        prop_assert!([32usize, 64, 128, 256].contains(&p.len()));
        prop_assert_eq!(bucket_unpad(&p).unwrap(), pt.as_slice());
    }
}
