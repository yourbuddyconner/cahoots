use cahoots_chat::{
    open, parse, seal, ChatError, EpochId, Header, MessageType, HEADER_LEN, TAG_LEN,
};
use proptest::prelude::*;

proptest! {
    #[test]
    fn parse_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
        let _ = parse(&bytes);
    }

    #[test]
    fn open_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..300), key in any::<[u8; 32]>()) {
        if let Ok((h, body)) = parse(&bytes) {
            let _ = open(&h, &key, body);
        }
    }

    #[test]
    fn roundtrip_and_single_byte_edit_fails(
        pt in proptest::collection::vec(any::<u8>(), 0..300),
        key in any::<[u8; 32]>(),
        sender in any::<u8>(), counter in any::<u32>(), slice in any::<u8>(), offset in any::<u32>(),
        pos in any::<prop::sample::Index>(), delta in 1u8..=255,
    ) {
        let h = Header { version: 2, circle_id: [1; 16], epoch: EpochId { slice, offset }, sender, counter, len: pt.len() as u32, msg_type: MessageType::Text };
        let wire = seal(&h, &key, &pt);
        prop_assert_eq!(wire.len(), HEADER_LEN + pt.len() + TAG_LEN);
        let (ph, body) = parse(&wire).unwrap();
        prop_assert_eq!(open(&ph, &key, body).unwrap(), pt);
        let mut w = wire.clone();
        let i = pos.index(w.len());
        w[i] ^= delta;
        match parse(&w) {
            Ok((ph, body)) => prop_assert_eq!(open(&ph, &key, body), Err(ChatError::Tag)),
            Err(e) => prop_assert!(matches!(e, ChatError::Version | ChatError::Type | ChatError::Length)),
        }
    }
}
