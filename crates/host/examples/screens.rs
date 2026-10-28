//! Render every badge screen into one PNG sheet, for looking at the design on a laptop.
//! `cargo run -p cahoots-host --example screens -- out.png`

use cahoots_badge::screens;
use cahoots_hal::Frame;

fn main() {
    let out = std::env::args().nth(1).expect("output path");
    let words = ["ridge", "proud", "other", "twin", "trend", "begin"];
    let selftest: Vec<String> = [
        "trng: health tests pass",
        "flash: erase, program, read back ok",
        "buttons: A up  B up",
        "circle: none yet",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let shots = vec![
        screens::self_test(&selftest),
        screens::pile(None),
        screens::pile(Some(2)),
        screens::ceremony(3, 1, 0, 3),
        screens::keep_prompt(&words),
        screens::discard_prompt(),
        screens::words("bench", "TRNG T1", &words),
        screens::words_page("bench", "TRNG T1", 1, 3, &words, false, true),
        screens::menu_page("bench", "TRNG T1", 2, 3, "key as code", &["Both spends 32 bytes and shows them as a code"], true, true),
        screens::menu_page("bench", "TRNG T1", 3, 3, "serve a laptop", &["off", "Both turns it on"], true, false),
        screens::code(&[0xAB, 0x12, 0xC4, 0x5F, 0x00, 0x99, 0xDE, 0x01].repeat(4)),
        screens::consent(32, "bench"),
        screens::status(&["ceremony: waiting for the others"]),
        screens::status(&["saved. read the words aloud"]),
        screens::panic("hard fault: pc=0x10012345 lr=0x1000abcd"),
    ];
    let (cols, scale, gap) = (5usize, 2usize, 12usize);
    let rows = shots.len().div_ceil(cols);
    let cell = Frame::WIDTH * scale + gap;
    let (w, h) = (cols * cell + gap, rows * cell + gap);
    let mut img = vec![0x80u8; w * h];
    for (i, s) in shots.iter().enumerate() {
        let (ox, oy) = (gap + (i % cols) * cell, gap + (i / cols) * cell);
        for y in 0..Frame::HEIGHT {
            for x in 0..Frame::WIDTH {
                let v = if s.frame.get(x, y) { 0x10 } else { 0xE8 };
                for dy in 0..scale {
                    for dx in 0..scale {
                        img[(oy + y * scale + dy) * w + ox + x * scale + dx] = v;
                    }
                }
            }
        }
    }
    let file = std::fs::File::create(&out).expect("create");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().expect("header").write_image_data(&img).expect("png");
}
