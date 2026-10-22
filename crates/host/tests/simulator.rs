//! Four simulated devices, in threads, each with a socket to both neighbours, run a ceremony.
//! Then device 0 sends a message that device 3 decrypts, and the pad files show erased bytes.

use cahoots_core::checkwords::check_words;
use cahoots_core::message::MessageType;
use cahoots_core::params::CircleParams;
use cahoots_hal::{Entropy, HealthError};
use cahoots_host::device::Device;
use cahoots_host::link::SocketLink;
use std::os::unix::net::UnixStream;

struct Det(u64);
impl Entropy for Det {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), HealthError> {
        for b in buf.iter_mut() {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            *b = (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8;
        }
        Ok(())
    }
}

#[test]
fn four_devices_ring_ceremony_then_message() {
    const L: u32 = 16 * 1024;
    let params = CircleParams::new(4, L, "sim", false).unwrap();
    // Ring: cable i joins device i and device i+1; each device holds one end of two cables.
    let mut next: Vec<Option<UnixStream>> = (0..4).map(|_| None).collect();
    let mut prev: Vec<Option<UnixStream>> = (0..4).map(|_| None).collect();
    for i in 0..4 {
        let (a, b) = UnixStream::pair().unwrap();
        next[i] = Some(a);
        prev[(i + 1) % 4] = Some(b);
    }
    let tmp = tempfile::tempdir().unwrap();
    let mut handles = Vec::new();
    for i in 0..4usize {
        let links = vec![
            SocketLink::new(prev[i].take().unwrap()).unwrap(),
            SocketLink::new(next[i].take().unwrap()).unwrap(),
        ];
        let params = params.clone();
        let dir = tmp.path().join(format!("dev{i}"));
        handles.push(std::thread::spawn(move || {
            let mut entropy = Det(5000 + i as u64);
            Device::ceremony(&dir, params, i as u8, &mut entropy, links).unwrap()
        }));
    }
    let mut devices: Vec<Device> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    let mut expected = vec![0u8; L as usize];
    for i in 0..4u64 {
        let mut c = vec![0u8; L as usize];
        Det(5000 + i).fill(&mut c).unwrap();
        for (e, b) in expected.iter_mut().zip(c) {
            *e ^= b;
        }
    }
    // Guard against a degenerate test: a linear generator with related seeds XORs to zero.
    assert!(expected.iter().filter(|&&b| b != 0).count() > L as usize / 2);
    let words = check_words(&params, &expected);
    for d in &devices {
        assert_eq!(d.check_words(), words);
        assert_eq!(d.circle_id(), cahoots_core::circle_id::circle_id(&expected));
    }

    let wire = devices[0]
        .send(MessageType::Text, b"hello from device 0")
        .unwrap();
    let (hdr, pt) = devices[3].receive(&wire).unwrap();
    assert_eq!(pt, b"hello from device 0");
    assert_eq!(hdr.slice, 0);

    // Both pad files show the spent range erased on disk. Slice 0 starts at offset 0.
    let spent = 32 + "hello from device 0".len();
    for i in [0usize, 3] {
        let raw = std::fs::read(tmp.path().join(format!("dev{i}/pad.bin"))).unwrap();
        assert_eq!(&raw[..spent], &vec![0u8; spent][..], "device {i} erased");
        assert_ne!(
            &raw[spent..spent + 32],
            &[0u8; 32][..],
            "device {i} kept the rest"
        );
    }
    // Replay is flagged, and the sender's second message lands at the next offset.
    assert!(devices[3].receive(&wire).is_err());
    let wire2 = devices[0].send(MessageType::Text, b"two").unwrap();
    assert_eq!(&wire2[18..22], &(spent as u32).to_le_bytes());
    assert_eq!(devices[3].receive(&wire2).unwrap().1, b"two");

    // Reopen device 3 from disk: ledger replay still refuses the first message.
    drop(devices);
    let mut d3 = Device::open(&tmp.path().join("dev3")).unwrap();
    assert!(d3.receive(&wire).is_err());
    assert_eq!(d3.check_words(), words);
}
