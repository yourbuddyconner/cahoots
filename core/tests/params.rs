use cahoots_core::params::{CircleParams, ParamsError};

fn p() -> CircleParams {
    CircleParams::new(4, 256 * 1024, "sf-meetup", false).unwrap()
}

#[test]
fn encodes_as_fixed_header_then_name() {
    let bytes = p().encode();
    // version=1 | n=4 | L=0x40000 LE | agent_seat=0 | name_len=9 | name
    assert_eq!(bytes[0], 1);
    assert_eq!(bytes[1], 4);
    assert_eq!(&bytes[2..6], &[0x00, 0x00, 0x04, 0x00]);
    assert_eq!(bytes[6], 0);
    assert_eq!(bytes[7], 9);
    assert_eq!(&bytes[8..], b"sf-meetup");
}

#[test]
fn slices_count_agent_seat() {
    assert_eq!(p().slices(), 4);
    let a = CircleParams::new(4, 256 * 1024, "x", true).unwrap();
    assert_eq!(a.slices(), 5);
    assert_eq!(a.agent_slice(), Some(4));
    assert_eq!(p().agent_slice(), None);
}

#[test]
fn slice_k_starts_at_k_times_floor_l_over_slices() {
    let a = CircleParams::new(3, 1000, "x", false).unwrap();
    assert_eq!(a.slice_len(), 333);
    assert_eq!(a.slice_start(0), 0);
    assert_eq!(a.slice_start(2), 666);
}

#[test]
fn rejects_out_of_range_n() {
    assert_eq!(
        CircleParams::new(1, 1024, "x", false).unwrap_err(),
        ParamsError::MemberCount
    );
    assert_eq!(
        CircleParams::new(9, 1024, "x", false).unwrap_err(),
        ParamsError::MemberCount
    );
}

#[test]
fn rejects_name_over_255_bytes_or_pad_too_small() {
    let long = "a".repeat(256);
    assert_eq!(
        CircleParams::new(2, 1024, &long, false).unwrap_err(),
        ParamsError::NameTooLong
    );
    // Each slice must hold at least one 32-byte key.
    assert_eq!(
        CircleParams::new(8, 100, "x", false).unwrap_err(),
        ParamsError::PadTooSmall
    );
}

#[test]
fn decode_roundtrips() {
    let bytes = p().encode();
    assert_eq!(CircleParams::decode(&bytes).unwrap(), p());
}
