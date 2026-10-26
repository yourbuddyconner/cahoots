use cahoots_core::ledger::MemLedger;
use cahoots_core::pad::{MemPadStorage, Pad};
use cahoots_core::params::CircleParams;
use cahoots_custodian::{Custodian, PadError, PSK_LEN};

#[test]
fn pad_is_a_custodian() {
    let params = CircleParams::new(3, 192, "vectors", false).unwrap();
    let bytes: Vec<u8> = (0..192u32).map(|i| (i * 7 + 3) as u8).collect();
    let slices = params.slices();
    let mut pad = Pad::new(params, 0, MemPadStorage::new(bytes), MemLedger::new(slices)).unwrap();

    let infos = pad.circles();
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].seat, 0);
    assert_eq!(infos[0].n, 3);
    assert_eq!(infos[0].tier, 0);
    assert_eq!(infos[0].name, "vectors");
    let before = Custodian::remaining(&pad, &infos[0].circle_id);
    let (h, key) = pad.psk(&infos[0].circle_id, b"test").unwrap();
    assert_eq!(h.slice, 0);
    assert_eq!(key.len(), PSK_LEN as usize);
    assert_eq!(
        Custodian::remaining(&pad, &infos[0].circle_id),
        before - u64::from(PSK_LEN)
    );
    assert_eq!(pad.psk_for(&h, b"test"), Err(PadError::AlreadySpent));
}
