//! The host's code read from a picture: a photo of the host's screen taken with the laptop's
//! camera, or a screenshot sent from the phone. `cahoots-host bridge --offer-image` uses it.

use image::GrayImage;
use std::path::Path;

/// The text of the first QR code in the PNG or JPEG at `path`.
pub fn read_offer_image(path: &Path) -> Result<String, String> {
    let picture = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    decode(picture.to_luma8()).ok_or_else(|| {
        format!(
            "{}: no code found in the picture. Take it again with the whole code in view",
            path.display()
        )
    })
}

/// The text of the first QR code in `gray`.
pub fn decode(gray: GrayImage) -> Option<String> {
    rqrr::PreparedImage::prepare(gray)
        .detect_grids()
        .into_iter()
        .find_map(|grid| grid.decode().ok().map(|(_, text)| text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Luma};
    use qrcode::QrCode;

    const OFFER: &str = "cahoots:ceremony:v2:dqAzSyQj2pEyXTBjlVCA4izDD40aVQSzXpoCp9uR6LsWiEeh4PhifISqOm0TjfhboMtyCDC_iVo5DwkWQvsFtsZPiu1g0TpD5lcK6lrtszwAHWh0dHBzOi8vcmVsYXkuY2Fob290cy5uZXR3b3JrAQMAAAEAAARwYXJr";

    fn drawn() -> GrayImage {
        QrCode::new(OFFER.as_bytes())
            .unwrap()
            .render::<Luma<u8>>()
            .module_dimensions(6, 6)
            .build()
    }

    #[test]
    fn an_offer_reads_back_from_a_png() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("offer.png");
        drawn().save_with_format(&path, ImageFormat::Png).unwrap();
        assert_eq!(read_offer_image(&path).unwrap(), OFFER);
    }

    #[test]
    fn an_offer_reads_back_from_a_jpeg() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("offer.jpg");
        drawn().save_with_format(&path, ImageFormat::Jpeg).unwrap();
        assert_eq!(read_offer_image(&path).unwrap(), OFFER);
    }

    #[test]
    fn a_code_in_a_camera_sized_picture_reads() {
        // A 12 megapixel picture with the code in one corner.
        let code = QrCode::new(OFFER.as_bytes())
            .unwrap()
            .render::<Luma<u8>>()
            .module_dimensions(4, 4)
            .build();
        let mut big = GrayImage::from_pixel(4000, 3000, Luma([255]));
        image::imageops::overlay(&mut big, &code, 100, 100);
        assert_eq!(decode(big).unwrap(), OFFER);
    }

    #[test]
    fn a_picture_without_a_code_says_so() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("blank.png");
        GrayImage::from_pixel(200, 200, Luma([255])).save_with_format(&path, ImageFormat::Png).unwrap();
        let e = read_offer_image(&path).unwrap_err();
        assert!(e.contains("no code found"), "{e}");
    }
}
