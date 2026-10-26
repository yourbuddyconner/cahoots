use cahoots_core::frame::{decode, decode_max, encode, Frame, FrameError, HEADER_LEN, MAX_PAYLOAD};

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

#[test]
fn rejects_a_header_claiming_a_huge_length() {
    // kind, seq, len = 0xFFFF_FFF0, then four bytes standing in for a CRC.
    let mut bytes = vec![40, 0, 0, 0, 0];
    bytes.extend_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
    bytes.extend_from_slice(&[0xAA; 4]);
    assert_eq!(decode(&bytes).unwrap_err(), FrameError::Crc);
}

#[test]
fn rejects_a_header_one_over_the_payload_limit() {
    let mut bytes = vec![1, 0, 0, 0, 0];
    bytes.extend_from_slice(&(MAX_PAYLOAD as u32 + 1).to_le_bytes());
    assert_eq!(decode(&bytes).unwrap_err(), FrameError::Crc);
    let mut at_limit = vec![1, 0, 0, 0, 0];
    at_limit.extend_from_slice(&(MAX_PAYLOAD as u32).to_le_bytes());
    assert_eq!(decode(&at_limit).unwrap_err(), FrameError::Incomplete);
}

#[test]
fn decode_max_rejects_a_frame_over_its_own_limit() {
    let f = Frame {
        kind: 1,
        seq: 0,
        payload: &[7; 100],
    };
    let bytes = encode(&f);
    assert!(decode_max(&bytes, 100).is_ok());
    assert_eq!(decode_max(&bytes, 99).unwrap_err(), FrameError::Crc);
    assert_eq!(decode_max(&bytes[..9], 99).unwrap_err(), FrameError::Crc);
    assert_eq!(
        decode_max(&bytes[..9], 100).unwrap_err(),
        FrameError::Incomplete
    );
}
