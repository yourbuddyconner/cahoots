//! The camera path: a QR rendered by `qr_svg`'s encoder must decode from raw pixels.

use cahoots_wasm::{qr_decode_gray, qr_decode_rgba};

/// Render `data` as a grayscale bitmap with a quiet zone, `scale` pixels per module.
fn render_gray(data: &[u8], scale: usize) -> (u32, u32, Vec<u8>) {
    use qrcode::{EcLevel, QrCode};
    let code = QrCode::with_error_correction_level(data, EcLevel::L).unwrap();
    let modules = code.width();
    let quiet = 4;
    let side = (modules + 2 * quiet) * scale;
    let mut gray = vec![255u8; side * side];
    let colors = code.to_colors();
    for y in 0..modules {
        for x in 0..modules {
            if colors[y * modules + x] == qrcode::Color::Dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let px = (x + quiet) * scale + dx;
                        let py = (y + quiet) * scale + dy;
                        gray[py * side + px] = 0;
                    }
                }
            }
        }
    }
    (side as u32, side as u32, gray)
}

#[test]
fn decodes_a_rendered_frame_from_gray_pixels() {
    let payload =
        "CHQ1-this-is-a-fountain-frame-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let (w, h, gray) = render_gray(payload.as_bytes(), 4);
    assert_eq!(qr_decode_gray(w, h, &gray).as_deref(), Some(payload));
}

#[test]
fn decodes_from_rgba_pixels() {
    let payload = "cahoots-commit:2:abcdefghijklmnopqrstuvwxyz0123456789";
    let (w, h, gray) = render_gray(payload.as_bytes(), 3);
    let rgba: Vec<u8> = gray.iter().flat_map(|&g| [g, g, g, 255]).collect();
    assert_eq!(qr_decode_rgba(w, h, &rgba).as_deref(), Some(payload));
}

#[test]
fn returns_none_for_noise_and_bad_sizes() {
    let noise: Vec<u8> = (0..160 * 120).map(|i| (i * 7919 % 251) as u8).collect();
    assert_eq!(qr_decode_gray(160, 120, &noise), None);
    assert_eq!(qr_decode_gray(160, 120, &noise[..100]), None);
    assert_eq!(qr_decode_rgba(10, 10, &[0u8; 10]), None);
}

#[test]
fn decodes_a_large_fountain_sized_payload() {
    // The app sends ~600-byte base64url frames.
    let payload: String = (0..600)
        .map(|i| {
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"[i % 64] as char
        })
        .collect();
    let (w, h, gray) = render_gray(payload.as_bytes(), 3);
    assert_eq!(
        qr_decode_gray(w, h, &gray).as_deref(),
        Some(payload.as_str())
    );
}
