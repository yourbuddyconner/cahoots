use cahoots_core::message::{open, parse, seal, MessageError, MessageType, HEADER_LEN, TAG_LEN};

const CID: [u8; 16] = [0xAB; 16];

fn pad(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 + 3) as u8).collect()
}

#[test]
fn header_is_25_bytes_tag_16() {
    assert_eq!(HEADER_LEN, 25);
    assert_eq!(TAG_LEN, 16);
}

#[test]
fn seal_then_open_roundtrips() {
    let pt = b"meet at noon";
    let bytes = pad(32 + pt.len());
    let msg = seal(&CID, 2, 64, MessageType::Text, pt, &bytes).unwrap();
    assert_eq!(msg.len(), HEADER_LEN + pt.len() + TAG_LEN);
    let (hdr, ct, tag) = parse(&msg).unwrap();
    assert_eq!(hdr.version, 1);
    assert_eq!(hdr.circle_id, CID);
    assert_eq!(hdr.slice, 2);
    assert_eq!(hdr.offset, 64);
    assert_eq!(hdr.len, pt.len() as u16);
    assert_eq!(hdr.msg_type, MessageType::Text);
    assert_eq!(ct.len(), pt.len());
    assert_eq!(tag.len(), TAG_LEN);
    assert_ne!(ct, pt);
    assert_eq!(open(&hdr, ct, tag, &bytes).unwrap(), pt);
}

#[test]
fn header_layout_is_little_endian() {
    let bytes = pad(32 + 1);
    let msg = seal(&CID, 3, 0x01020304, MessageType::Vouch, b"x", &bytes).unwrap();
    assert_eq!(msg[0], 1);
    assert_eq!(&msg[1..17], &CID);
    assert_eq!(msg[17], 3);
    assert_eq!(&msg[18..22], &[4, 3, 2, 1]);
    assert_eq!(&msg[22..24], &[1, 0]);
    assert_eq!(msg[24], 2);
}

#[test]
fn flipped_byte_fails_tag() {
    let pt = b"hello";
    let bytes = pad(32 + pt.len());
    let msg = seal(&CID, 0, 0, MessageType::Text, pt, &bytes).unwrap();
    for i in 0..msg.len() {
        let mut m = msg.clone();
        m[i] ^= 0x01;
        match parse(&m) {
            Ok((h, ct, tag)) => assert_eq!(
                open(&h, ct, tag, &bytes).unwrap_err(),
                MessageError::Tag,
                "byte {i}"
            ),
            Err(e) => assert!(
                matches!(
                    e,
                    MessageError::Version | MessageError::Type | MessageError::Length
                ),
                "byte {i}: {e:?}"
            ),
        }
    }
}

#[test]
fn seal_requires_exactly_32_plus_len_pad_bytes() {
    assert_eq!(
        seal(&CID, 0, 0, MessageType::Text, b"ab", &pad(33)).unwrap_err(),
        MessageError::PadLength
    );
    assert_eq!(
        seal(&CID, 0, 0, MessageType::Text, b"ab", &pad(35)).unwrap_err(),
        MessageError::PadLength
    );
}

#[test]
fn parse_rejects_truncated_and_unknown_type() {
    let bytes = pad(32 + 3);
    let msg = seal(&CID, 0, 0, MessageType::Approve, b"abc", &bytes).unwrap();
    assert_eq!(
        parse(&msg[..msg.len() - 1]).unwrap_err(),
        MessageError::Length
    );
    assert_eq!(parse(&msg[..10]).unwrap_err(), MessageError::Length);
    let mut m = msg.clone();
    m[24] = 9;
    assert_eq!(parse(&m).unwrap_err(), MessageError::Type);
    let mut m = msg.clone();
    m[0] = 2;
    assert_eq!(parse(&m).unwrap_err(), MessageError::Version);
}

#[test]
fn tag_is_poly1305_over_header_and_ciphertext() {
    use poly1305::universal_hash::KeyInit;
    let pt = b"zz";
    let bytes = pad(34);
    let msg = seal(&CID, 1, 5, MessageType::Control, pt, &bytes).unwrap();
    let key: [u8; 32] = bytes[..32].try_into().unwrap();
    let expect = poly1305::Poly1305::new(&key.into()).compute_unpadded(&msg[..HEADER_LEN + 2]);
    assert_eq!(&msg[HEADER_LEN + 2..], expect.as_slice());
}
