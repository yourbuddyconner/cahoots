//! A 200x200 monochrome display rendered to PNG, matching the 1.54" e-paper panel.

use crate::font;
use cahoots_hal::Display;
use std::path::{Path, PathBuf};

pub const SIZE: usize = 200;

pub struct PngDisplay {
    path: PathBuf,
    name: String,
    status: String,
    words: Option<[String; 6]>,
    qr_bytes: usize,
}

impl PngDisplay {
    pub fn new(path: &Path, name: &str) -> Self {
        PngDisplay {
            path: path.to_path_buf(),
            name: name.to_string(),
            status: String::new(),
            words: None,
            qr_bytes: 0,
        }
    }

    fn render(&self) {
        let mut px = vec![255u8; SIZE * SIZE];
        draw_text(&mut px, 4, 4, &self.name, 1);
        draw_text(&mut px, 4, 16, &self.status, 1);
        for x in 0..SIZE {
            px[28 * SIZE + x] = 0;
        }
        if let Some(words) = &self.words {
            for (i, w) in words.iter().enumerate() {
                let col = i / 3;
                let row = i % 3;
                draw_text(&mut px, 8 + col * 96, 44 + row * 40, w, 2);
            }
        } else if self.qr_bytes > 0 {
            draw_text(&mut px, 4, 44, &format!("QR {} B", self.qr_bytes), 2);
        }
        let file = match std::fs::File::create(&self.path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!(
                    "[{}] display: cannot write {}: {e}",
                    self.name,
                    self.path.display()
                );
                return;
            }
        };
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), SIZE as u32, SIZE as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        if let Ok(mut w) = enc.write_header() {
            let _ = w.write_image_data(&px);
        }
    }
}

fn draw_text(px: &mut [u8], x0: usize, y0: usize, text: &str, scale: usize) {
    let mut x = x0;
    for c in text.chars() {
        let g = font::glyph(c);
        for (cx, col) in g.iter().enumerate() {
            for cy in 0..font::HEIGHT {
                if col >> cy & 1 == 1 {
                    for sx in 0..scale {
                        for sy in 0..scale {
                            let (px_x, px_y) = (x + cx * scale + sx, y0 + cy * scale + sy);
                            if px_x < SIZE && px_y < SIZE {
                                px[px_y * SIZE + px_x] = 0;
                            }
                        }
                    }
                }
            }
        }
        x += (font::WIDTH + 1) * scale;
    }
}

impl Display for PngDisplay {
    fn show_words(&mut self, w: &[&str; 6]) {
        eprintln!("[{}] check words: {}", self.name, w.join(" "));
        self.words = Some(w.map(String::from));
        self.render();
    }

    fn show_qr(&mut self, data: &[u8]) {
        eprintln!("[{}] qr: {} bytes", self.name, data.len());
        self.qr_bytes = data.len();
        self.words = None;
        self.render();
    }

    fn show_status(&mut self, s: &str) {
        eprintln!("[{}] {s}", self.name);
        self.status = s.to_string();
        self.render();
    }
}
