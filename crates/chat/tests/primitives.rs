use cahoots_chat::{
    bucket_pad, bucket_unpad, chain_init, epoch_key, open, parse_header, seal, step, ChatError,
    EpochId, Header, MessageType, HEADER_LEN, TAG_LEN,
};
use hkdf::Hkdf;
use sha2::Sha256;

const CID: [u8; 16] = [0xC1; 16];

#[test]
fn epoch_key_is_hmac_of_the_root_with_the_chat_label() {
    use hkdf::hmac::{Hmac, Mac};
    let root = [7u8; 32];
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&root).unwrap();
    m.update(b"chat");
    m.update(&CID);
    m.update(&0x01020304u64.to_be_bytes());
    let want: [u8; 32] = m.finalize().into_bytes().into();
    assert_eq!(*epoch_key(&root, &CID, 0x01020304), want);
}

#[test]
fn chain_init_and_step_follow_spec_labels() {
    let ek = [9u8; 32];
    let c0 = chain_init(&ek, 3);
    let mut want = [0u8; 32];
    Hkdf::<Sha256>::from_prk(&ek)
        .unwrap()
        .expand(b"cahoots/chat/chain/v1\x03", &mut want)
        .unwrap();
    assert_eq!(*c0, want);

    let (mk, c1) = step(&c0);
    let mut want_mk = [0u8; 32];
    Hkdf::<Sha256>::from_prk(&*c0)
        .unwrap()
        .expand(b"cahoots/chat/msg/v1", &mut want_mk)
        .unwrap();
    let mut want_c1 = [0u8; 32];
    Hkdf::<Sha256>::from_prk(&*c0)
        .unwrap()
        .expand(b"cahoots/chat/next/v1", &mut want_c1)
        .unwrap();
    assert_eq!(*mk, want_mk);
    assert_eq!(*c1, want_c1);
    assert_ne!(*mk, *c1);
}

fn header() -> Header {
    Header {
        version: 2,
        circle_id: CID,
        epoch: EpochId::root(64),
        sender: 3,
        counter: 7,
        len: 5,
        msg_type: MessageType::Text,
    }
}

#[test]
fn header_layout_is_little_endian_and_32_bytes() {
    assert_eq!(HEADER_LEN, 1 + 16 + 5 + 1 + 4 + 4 + 1);
    let h = header().encode();
    assert_eq!(h[0], 2);
    assert_eq!(&h[1..17], &CID);
    assert_eq!(h[17], 0xFF);
    assert_eq!(&h[18..22], &64u32.to_le_bytes());
    assert_eq!(h[22], 3);
    assert_eq!(&h[23..27], &7u32.to_le_bytes());
    assert_eq!(&h[27..31], &5u32.to_le_bytes());
    assert_eq!(h[31], 0);
    assert_eq!(parse_header(&h).unwrap(), header());
}

#[test]
fn parse_rejects_version_type_and_short() {
    let mut h = header().encode();
    assert_eq!(parse_header(&h[..10]).unwrap_err(), ChatError::Length);
    h[31] = 9;
    assert_eq!(parse_header(&h).unwrap_err(), ChatError::Type);
    h[31] = 0;
    h[0] = 1;
    assert_eq!(parse_header(&h).unwrap_err(), ChatError::Version);
}

#[test]
fn seal_open_roundtrip_and_tamper() {
    let mk = [5u8; 32];
    let pt = b"hello";
    let wire = seal(&header(), &mk, pt);
    assert_eq!(wire.len(), HEADER_LEN + pt.len() + TAG_LEN);
    assert_eq!(&wire[..HEADER_LEN], &header().encode());
    assert_ne!(&wire[HEADER_LEN..HEADER_LEN + 5], pt);
    let (h, body) = parse_header_and_body(&wire);
    assert_eq!(open(&h, &mk, body).unwrap(), pt);
    for i in 0..wire.len() {
        let mut w = wire.clone();
        w[i] ^= 1;
        match cahoots_chat::parse(&w) {
            Ok((h, body)) => {
                assert_eq!(open(&h, &mk, body).unwrap_err(), ChatError::Tag, "byte {i}")
            }
            Err(e) => assert!(matches!(
                e,
                ChatError::Version | ChatError::Type | ChatError::Length
            )),
        }
    }
}

fn parse_header_and_body(wire: &[u8]) -> (Header, &[u8]) {
    cahoots_chat::parse(wire).unwrap()
}

#[test]
fn nonce_is_sender_counter_zero_padded() {
    assert_eq!(
        cahoots_chat::nonce(3, 7),
        [3, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
}

#[test]
fn bucket_padding_uses_chat_buckets() {
    assert_eq!(bucket_pad(b"abc").unwrap().len(), 64);
    assert_eq!(bucket_pad(&[1u8; 100]).unwrap().len(), 256);
    assert_eq!(bucket_pad(&[1u8; 1000]).unwrap().len(), 1024);
    assert_eq!(bucket_pad(&[1u8; 4000]).unwrap().len(), 4096);
    assert!(bucket_pad(&[1u8; 4095]).is_none());
    let p = bucket_pad(&[1u8; 100]).unwrap();
    assert_eq!(bucket_unpad(&p).unwrap(), &[1u8; 100][..]);
    assert_eq!(bucket_unpad(&bucket_pad(b"").unwrap()).unwrap(), b"");
    assert!(bucket_unpad(&[0u8; 65]).is_none());
}
