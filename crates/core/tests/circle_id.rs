use cahoots_core::circle_id::circle_id;
use sha2::{Digest, Sha256};

#[test]
fn circle_id_is_first_16_bytes_of_domain_separated_hash() {
    let pad = [9u8; 100];
    let mut h = Sha256::new();
    h.update(b"cahoots/circle-id/v1");
    h.update(pad);
    let d = h.finalize();
    assert_eq!(circle_id(&pad), <[u8; 16]>::try_from(&d[..16]).unwrap());
}
