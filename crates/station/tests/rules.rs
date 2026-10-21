use cahoots_core::message::{seal, MessageType};
use cahoots_station::rules::{RuleError, Submission, MAX_BODY};

fn cid(b: u8) -> [u8; 16] {
    [b; 16]
}

fn chat(cid: [u8; 16], len: u32, ct_len: usize) -> Vec<u8> {
    let mut m = vec![2u8];
    m.extend_from_slice(&cid);
    m.extend_from_slice(&[1, 2, 3, 4, 5]);
    m.push(0);
    m.extend_from_slice(&9u32.to_le_bytes());
    m.extend_from_slice(&len.to_le_bytes());
    m.push(0);
    m.extend(std::iter::repeat_n(0u8, ct_len));
    m
}

#[test]
fn parses_pure_pad_and_reports_its_range() {
    let pad = [1u8; 32 + 5];
    let m = seal(&cid(3), 2, 40, MessageType::Text, b"hello", &pad).unwrap();
    let s = Submission::parse(&cid(3), &m).unwrap();
    assert_eq!(s.circle_id(), cid(3));
    assert_eq!(s.pad_range(), Some((2, 40, 40 + 37)));
    assert_eq!(s.bytes(), &m[..]);
}

#[test]
fn parses_chat_and_validates_lengths() {
    let s = Submission::parse(&cid(3), &chat(cid(3), 10, 26)).unwrap();
    assert_eq!(s.pad_range(), None);
    assert!(matches!(
        Submission::parse(&cid(3), &chat(cid(3), 10, 25)).unwrap_err(),
        RuleError::Length
    ));
    assert!(matches!(
        Submission::parse(&cid(3), &chat(cid(3), 10, 27)).unwrap_err(),
        RuleError::Length
    ));
    assert!(matches!(
        Submission::parse(&cid(3), &chat(cid(3), 0, 16)[..20]).unwrap_err(),
        RuleError::Length
    ));
}

#[test]
fn rejects_circle_mismatch_unknown_version_and_oversize() {
    assert!(matches!(
        Submission::parse(&cid(4), &chat(cid(3), 1, 17)).unwrap_err(),
        RuleError::CircleMismatch
    ));
    let mut m = chat(cid(3), 1, 17);
    m[0] = 9;
    assert!(matches!(
        Submission::parse(&cid(3), &m).unwrap_err(),
        RuleError::Version
    ));
    assert!(matches!(
        Submission::parse(&cid(3), &[]).unwrap_err(),
        RuleError::Length
    ));
    let big = chat(cid(3), MAX_BODY as u32, MAX_BODY + 16);
    assert!(matches!(
        Submission::parse(&cid(3), &big).unwrap_err(),
        RuleError::TooLarge
    ));
    let pad = [1u8; 32 + 3];
    let mut m = seal(&cid(3), 2, 40, MessageType::Text, b"abc", &pad).unwrap();
    m.pop();
    assert!(matches!(
        Submission::parse(&cid(3), &m).unwrap_err(),
        RuleError::Length
    ));
}
