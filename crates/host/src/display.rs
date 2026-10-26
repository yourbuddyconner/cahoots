//! A 200x200 monochrome display rendered to PNG, matching the 1.54" e-paper panel.

use crate::font;
use cahoots_hal::{Display, Frame};
use std::path::{Path, PathBuf};

pub const SIZE: usize = 200;

/// Writes every frame it is asked to draw to a PNG, one image pixel per frame pixel.
pub struct PngDisplay {
    path: PathBuf,
    name: String,
}

impl PngDisplay {
    pub fn new(path: &Path, name: &str) -> Self {
        PngDisplay {
            path: path.to_path_buf(),
            name: name.to_string(),
        }
    }

    /// Log `lines` and draw them under the device name with [`status_frame`].
    pub fn show_lines(&mut self, lines: &[&str]) {
        eprintln!("[{}] {}", self.name, lines.join(" "));
        let mut all = vec![self.name.as_str()];
        all.extend_from_slice(lines);
        self.draw(&status_frame(&all));
    }
}

impl Display for PngDisplay {
    fn draw(&mut self, frame: &Frame) {
        let mut px = vec![255u8; SIZE * SIZE];
        for y in 0..SIZE {
            for x in 0..SIZE {
                if frame.get(x, y) {
                    px[y * SIZE + x] = 0;
                }
            }
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

/// A plain text frame: one line of the 5x7 font every 10 pixels, from the top.
pub fn status_frame(lines: &[&str]) -> Frame {
    let mut f = Frame::blank();
    for (i, line) in lines.iter().enumerate() {
        draw_text(&mut f, 4, 4 + i * 10, line);
    }
    f
}

fn draw_text(f: &mut Frame, x0: usize, y0: usize, text: &str) {
    let mut x = x0;
    for c in text.chars() {
        let g = font::glyph(c);
        for (cx, col) in g.iter().enumerate() {
            for cy in 0..font::HEIGHT {
                if col >> cy & 1 == 1 {
                    f.set(x + cx, y0 + cy, true);
                }
            }
        }
        x += font::WIDTH + 1;
    }
}
