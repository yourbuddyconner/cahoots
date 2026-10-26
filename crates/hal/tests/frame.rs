use cahoots_hal::Frame;

#[test]
fn blank_is_white() {
    let f = Frame::blank();
    for y in 0..Frame::HEIGHT {
        for x in 0..Frame::WIDTH {
            assert!(!f.get(x, y), "({x}, {y}) is black");
        }
    }
    assert_eq!(f.bits.len(), Frame::ROW_BYTES * Frame::HEIGHT);
}

#[test]
fn set_and_get_round_trip() {
    let mut f = Frame::blank();
    f.set(3, 7, true);
    assert!(f.get(3, 7));
    // Row-major, MSB first: pixel x of a row is bit 7 - (x % 8) of byte x / 8.
    assert_ne!(f.bits[7 * 25] & 0x10, 0);
    assert_eq!(f.bits.iter().filter(|&&b| b != 0).count(), 1);
    assert!(!f.get(4, 7));
    assert!(!f.get(3, 6));
    f.set(199, 199, true);
    assert_eq!(f.bits[199 * 25 + 24], 0x01);
    f.set(3, 7, false);
    assert!(!f.get(3, 7));
    assert_eq!(f.bits[7 * 25], 0);
}

#[test]
fn out_of_range_set_is_ignored() {
    let mut f = Frame::blank();
    f.set(Frame::WIDTH, 0, true);
    f.set(0, Frame::HEIGHT, true);
    f.set(usize::MAX, usize::MAX, true);
    assert!(f.bits.iter().all(|&b| b == 0));
    assert!(!f.get(Frame::WIDTH, 0));
    assert!(!f.get(0, Frame::HEIGHT));
}
