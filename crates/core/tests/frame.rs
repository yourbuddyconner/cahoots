use cahoots_core::frame::{decode, encode, Frame, FrameError, HEADER_LEN};

#[test]
fn encodes_type_seq_len_payload_crc() {
    let f = Frame {
        kind: 2,
        seq: 0x01020304,
        payload: b"hi",
    };
    let bytes = encode(&f);
    assert_eq!(HEADER_LEN, 9);
    assert_eq!(&bytes[..9], &[2, 4, 3, 2, 1, 2, 0, 0, 0]);
    assert_eq!(&bytes[9..11], b"hi");
    assert_eq!(bytes.len(), 9 + 2 + 4);
    let (parsed, used) = decode(&bytes).unwrap();
    assert_eq!(parsed, f);
    assert_eq!(used, bytes.len());
}

#[test]
fn rejects_bad_crc_and_short_input() {
    let f = Frame {
        kind: 1,
        seq: 0,
        payload: b"abc",
    };
    let mut bytes = encode(&f);
    *bytes.last_mut().unwrap() ^= 1;
    assert_eq!(decode(&bytes).unwrap_err(), FrameError::Crc);
    assert_eq!(decode(&bytes[..5]).unwrap_err(), FrameError::Incomplete);
    assert_eq!(decode(&bytes[..12]).unwrap_err(), FrameError::Incomplete);
}
