use age_core::format::{FileKey, Stanza};
use age_core::secrecy::ExposeSecret;
use age_plugin_cahoots::{
    decode_identity, decode_recipient, encode_identity, encode_recipient, unwrap_stanza,
    wrap_file_key, PluginError, STANZA_TAG,
};
use cahoots_core::params::CircleParams;
use cahoots_provider::{MemProvider, PadError, PadProvider};

fn pad_bytes(len: usize) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(41) ^ (i >> 1)) as u8)
        .collect()
}

fn pair() -> (MemProvider, MemProvider, [u8; 16]) {
    let params = CircleParams::new(2, 4096, "age", false).unwrap();
    let mut a = MemProvider::new();
    let mut b = MemProvider::new();
    let cid = a.add_circle(params.clone(), 0, pad_bytes(4096)).unwrap();
    b.add_circle(params, 1, pad_bytes(4096)).unwrap();
    (a, b, cid)
}

#[test]
fn recipient_and_identity_strings_roundtrip() {
    let cid = [0x5a; 16];
    let r = encode_recipient(&cid);
    assert!(r.starts_with("age1cahoots1"), "{r}");
    assert_eq!(decode_recipient(&r).unwrap(), cid);
    let i = encode_identity(&cid);
    assert!(i.starts_with("AGE-PLUGIN-CAHOOTS-1"), "{i}");
    assert_eq!(decode_identity(&i).unwrap(), cid);
    assert_eq!(
        decode_recipient("age1cahoots1qqqq").unwrap_err(),
        PluginError::Encoding
    );
    assert_eq!(
        decode_recipient("age1yubikey1abc").unwrap_err(),
        PluginError::Encoding
    );
}

#[test]
fn wrap_then_unwrap_file_key_across_members() {
    let (mut a, mut b, cid) = pair();
    let fk = FileKey::new(Box::new([7u8; 16]));
    let stanza = wrap_file_key(&mut a, &cid, &fk).unwrap();
    assert_eq!(stanza.tag, STANZA_TAG);
    assert_eq!(stanza.args.len(), 1);
    assert_eq!(stanza.body.len(), 32, "16-byte file key + 16-byte tag");
    let out = unwrap_stanza(&mut b, &[cid], &stanza).unwrap().unwrap();
    assert_eq!(out.expose_secret(), &[7u8; 16]);
    assert_eq!(a.remaining(&cid), 2048 - 32);
}

#[test]
fn unwrap_ignores_other_circles_and_other_tags() {
    let (mut a, mut b, cid) = pair();
    let fk = FileKey::new(Box::new([1u8; 16]));
    let stanza = wrap_file_key(&mut a, &cid, &fk).unwrap();
    assert!(unwrap_stanza(&mut b, &[[9u8; 16]], &stanza)
        .unwrap()
        .is_none());
    let other = Stanza {
        tag: "X25519".into(),
        args: stanza.args.clone(),
        body: stanza.body.clone(),
    };
    assert!(unwrap_stanza(&mut b, &[cid], &other).unwrap().is_none());
    assert_eq!(b.remaining(&cid), 2048, "nothing spent while ignoring");
}

#[test]
fn replay_is_refused_and_tamper_fails() {
    let (mut a, mut b, cid) = pair();
    let fk = FileKey::new(Box::new([2u8; 16]));
    let stanza = wrap_file_key(&mut a, &cid, &fk).unwrap();
    unwrap_stanza(&mut b, &[cid], &stanza).ok().unwrap();
    let err = unwrap_stanza(&mut b, &[cid], &stanza).err().unwrap();
    assert_eq!(err, PluginError::Pad(PadError::AlreadySpent));
    let mut bad = wrap_file_key(&mut a, &cid, &fk).unwrap();
    bad.body[0] ^= 1;
    let err = unwrap_stanza(&mut b, &[cid], &bad).err().unwrap();
    assert_eq!(err, PluginError::Unwrap);
}
