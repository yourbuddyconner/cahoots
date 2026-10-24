//! A pile of simulated cards on sockets: the top card presses B, everyone agrees.

use cahoots_core::message::MessageType;
use cahoots_host::pile::run_pile;

#[test]
fn three_simulated_cards_in_a_pile_make_one_circle() {
    let tmp = tempfile::tempdir().unwrap();
    let devices = run_pile(3, None, "bench", 16 * 1024, tmp.path(), Some(77)).unwrap();
    assert_eq!(devices.len(), 3);
    let words = devices[0].check_words();
    let id = devices[0].circle_id();
    for d in &devices {
        assert_eq!(d.check_words(), words);
        assert_eq!(d.circle_id(), id);
        assert_eq!(d.params().n, 3);
        assert_eq!(d.params().name, "bench", "the bottom card names the circle");
    }
    let mut devices = devices;
    let wire = devices[2].send(MessageType::Text, b"from the top").unwrap();
    let (_, pt) = devices[0].receive(&wire).unwrap();
    assert_eq!(pt, b"from the top");
}

#[test]
fn a_pile_needs_two_cards() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(run_pile(1, None, "x", 16 * 1024, tmp.path(), Some(1)).is_err());
}
