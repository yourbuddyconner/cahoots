use cahoots_hal::{Display, Frame, Link, PadStore};
use cahoots_host::display::{status_frame, PngDisplay};
use cahoots_host::link::SocketLink;
use cahoots_host::store::FilePadStore;
use std::os::unix::net::UnixStream;

#[test]
fn socket_link_delivers_in_order_and_is_non_blocking() {
    let (a_end, b_end) = UnixStream::pair().unwrap();
    let mut a = SocketLink::new(a_end).unwrap();
    let mut b = SocketLink::new(b_end).unwrap();
    let mut buf = [0u8; 16];
    assert_eq!(b.recv(&mut buf), 0, "nothing yet, must not block");
    a.send(b"hello");
    a.send(b" ring");
    let mut got = Vec::new();
    while got.len() < 10 {
        let n = b.recv(&mut buf);
        got.extend_from_slice(&buf[..n]);
    }
    assert_eq!(got, b"hello ring");
    assert!(a.peer_present());
}

#[test]
fn file_pad_store_erases_sector_aligned_or_not_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pad.bin");
    let bytes: Vec<u8> = (0..8192u32).map(|i| (i % 251) as u8 + 1).collect();
    let mut s = FilePadStore::create(&path, &bytes).unwrap();
    assert_eq!(s.len(), 8192);
    assert_eq!(s.erase_unit(), 4096);
    s.erase(4090, 20).unwrap();
    drop(s);
    let s = FilePadStore::open(&path).unwrap();
    let mut buf = [0u8; 22];
    s.read(4089, &mut buf);
    assert_ne!(buf[0], 0);
    assert_eq!(&buf[1..21], &[0u8; 20]);
    assert_ne!(buf[21], 0);
    let raw = std::fs::read(&path).unwrap();
    assert_eq!(
        &raw[4090..4110],
        &[0u8; 20],
        "erased on disk, not just in memory"
    );
}

#[test]
fn png_display_writes_a_200x200_image() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("display.png");
    let mut d = PngDisplay::new(&path, "dev0");
    let mut frame = status_frame(&["dev0", "commits 2/4"]);
    frame.set(199, 0, true);
    d.draw(&frame);
    let decoder = png::Decoder::new(std::fs::File::open(&path).unwrap());
    let mut reader = decoder.read_info().unwrap();
    let info = reader.info();
    assert_eq!((info.width, info.height), (200, 200));
    let mut buf = vec![0; reader.output_buffer_size()];
    let out = reader.next_frame(&mut buf).unwrap();
    let px = &buf[..out.buffer_size()];
    assert!(px.contains(&0), "some black pixels drawn");
    assert!(px.contains(&255), "white background");
    // One image pixel per frame pixel.
    for y in 0..Frame::HEIGHT {
        for x in 0..Frame::WIDTH {
            assert_eq!(px[y * 200 + x] == 0, frame.get(x, y), "({x}, {y})");
        }
    }
}
