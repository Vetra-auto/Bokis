//! Bilder till agenten.
//!
//! Ett kvittofoto från en telefon är ofta 3–10 MB och 4 000 bildpunkter
//! brett. Modellen skalar ändå ner till ungefär 1 568 bildpunkter, och ett
//! verktygssvar i Claude får vara ungefär 150 000 tecken. Agenten får därför
//! en nedskalad JPEG med rätt orientering. Originalet ligger kvar orört i
//! arkivet och är det som är räkenskapsinformation.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageDecoder, ImageReader, Limits};

/// Största bild som skickas till agenten, i byte före base64.
pub const MAX_PREVIEW_BYTES: usize = 100 * 1024;

/// Längsta sida och JPEG-kvalitet, i den ordning de prövas tills bilden ryms.
const STEPS: [(u32, u8); 6] = [
    (1568, 80),
    (1568, 65),
    (1400, 60),
    (1200, 55),
    (1000, 50),
    (800, 45),
];

/// En bild att visa för agenten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub data: Vec<u8>,
    pub media_type: String,
    pub width: u32,
    pub height: u32,
    /// Bilden är nedskalad eller omkodad; originalet finns i arkivet.
    pub scaled: bool,
}

/// Bildformat som modellen kan läsa direkt.
pub fn readable(media_type: &str) -> bool {
    matches!(
        media_type,
        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
    )
}

fn decode(data: &[u8]) -> Option<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?;
    // Skydd mot bilder som packar upp sig till gigabyte.
    let mut limits = Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).ok()?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Some(img)
}

/// En bild som redan ryms skickas som den är. Annars avkodas den, vänds
/// enligt EXIF, skalas ner och kodas som JPEG med sjunkande storlek och
/// kvalitet tills den ryms. `None` om bilden inte går att avkoda, till
/// exempel HEIC från en iPhone.
pub fn preview(data: &[u8], media_type: &str) -> Option<Preview> {
    if readable(media_type) && data.len() <= MAX_PREVIEW_BYTES {
        let (width, height) = ImageReader::new(Cursor::new(data))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()?;
        return Some(Preview {
            data: data.to_vec(),
            media_type: media_type.to_string(),
            width,
            height,
            scaled: false,
        });
    }
    let img = decode(data)?;
    let mut last = None;
    for (edge, quality) in STEPS {
        let (w, h) = img.dimensions();
        let scaled = if w.max(h) > edge {
            img.resize(edge, edge, FilterType::Triangle)
        } else {
            img.clone()
        };
        let rgb = scaled.to_rgb8();
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, quality)
            .encode_image(&rgb)
            .ok()?;
        let fits = out.len() <= MAX_PREVIEW_BYTES;
        last = Some(Preview {
            data: out,
            media_type: "image/jpeg".into(),
            width: rgb.width(),
            height: rgb.height(),
            scaled: true,
        });
        if fits {
            break;
        }
    }
    last
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgb, RgbImage};

    /// Ett "kvitto": vitt papper med svarta rader och brus, så att JPEG inte
    /// kan packa det till nästan ingenting.
    fn photo(w: u32, h: u32) -> Vec<u8> {
        let mut img = RgbImage::from_pixel(w, h, Rgb([245, 243, 236]));
        let mut seed = 7u32;
        for (x, y, p) in img.enumerate_pixels_mut() {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let noise = (seed >> 24) as u8 % 24;
            if (y / 40) % 3 == 0 && (x / 9) % 4 != 0 {
                *p = Rgb([20 + noise, 20 + noise, 25 + noise]);
            } else {
                p.0[0] = p.0[0].saturating_sub(noise);
            }
        }
        let mut out = Vec::new();
        DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg)
            .unwrap();
        out
    }

    #[test]
    fn small_images_are_sent_as_they_are() {
        let data = photo(300, 200);
        let p = preview(&data, "image/jpeg").unwrap();
        assert!(!p.scaled);
        assert_eq!(p.data, data);
        assert_eq!((p.width, p.height), (300, 200));
    }

    #[test]
    fn large_photos_are_scaled_until_they_fit() {
        let data = photo(2400, 3200);
        assert!(data.len() > MAX_PREVIEW_BYTES);
        let p = preview(&data, "image/jpeg").unwrap();
        assert!(p.scaled);
        assert_eq!(p.media_type, "image/jpeg");
        assert!(p.data.len() <= MAX_PREVIEW_BYTES, "{} byte", p.data.len());
        assert!(p.width.max(p.height) <= 1568);
        // Proportionerna behålls: 3:4.
        let ratio = f64::from(p.width) / f64::from(p.height);
        assert!((ratio - 0.75).abs() < 0.01, "{}x{}", p.width, p.height);
    }

    #[test]
    fn undecodable_images_give_none() {
        assert!(preview(b"inte en bild", "image/heic").is_none());
        assert!(!readable("image/heic"));
    }
}
