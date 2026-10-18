//! Decode a shot to pixels and produce preview/thumbnail JPEGs.

use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::codecs::jpeg::JpegEncoder;
use image::metadata::Orientation;
use image::{DynamicImage, ImageEncoder};

pub struct Rendered {
    /// Full-resolution, orientation-corrected image (used for ML crops).
    pub full: DynamicImage,
    pub preview_jpeg: Vec<u8>,
    pub thumb_jpeg: Vec<u8>,
}

/// Decode JPEG bytes and apply EXIF orientation.
pub fn decode_jpeg(bytes: &[u8], orientation: Option<u8>) -> Result<DynamicImage> {
    let mut img = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)
        .context("decoding JPEG")?;
    if let Some(o) = orientation.and_then(Orientation::from_exif) {
        img.apply_orientation(o);
    }
    Ok(img)
}

/// The largest embedded preview in a RAW file (cameras embed a full-size or near
/// full-size JPEG). Orientation is applied by rawler.
pub fn decode_raw_preview(path: &Path) -> Result<DynamicImage> {
    let src = rawler::rawsource::RawSource::new(path)?;
    let decoder = rawler::get_decoder(&src)?;
    let params = rawler::decoders::RawDecodeParams::default();
    decoder
        .full_image(&src, &params)
        .ok()
        .flatten()
        .or_else(|| decoder.preview_image(&src, &params).ok().flatten())
        .or_else(|| decoder.thumbnail_image(&src, &params).ok().flatten())
        .ok_or_else(|| anyhow!("no embedded preview in {}", path.display()))
}

/// Resize so the long edge is at most `max_px` (never upscales).
pub fn fit(img: &DynamicImage, max_px: u32) -> Result<DynamicImage> {
    let (w, h) = (img.width(), img.height());
    let long = w.max(h);
    if long <= max_px {
        return Ok(img.clone());
    }
    let scale = max_px as f64 / long as f64;
    let (nw, nh) = (
        ((w as f64) * scale).round().max(1.0) as u32,
        ((h as f64) * scale).round().max(1.0) as u32,
    );
    let src = DynamicImage::ImageRgb8(img.to_rgb8());
    let mut dst = DynamicImage::new(nw, nh, src.color());
    Resizer::new().resize(
        &src,
        &mut dst,
        &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3)),
    )?;
    Ok(dst)
}

pub fn encode_jpeg(img: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
    let rgb = img.to_rgb8();
    let mut out = Cursor::new(Vec::with_capacity(
        (rgb.width() * rgb.height() / 4) as usize,
    ));
    JpegEncoder::new_with_quality(&mut out, quality).write_image(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(out.into_inner())
}

pub fn render(full: DynamicImage, preview_px: u32, thumb_px: u32) -> Result<Rendered> {
    let preview = fit(&full, preview_px)?;
    let thumb = fit(&preview, thumb_px)?;
    Ok(Rendered {
        preview_jpeg: encode_jpeg(&preview, 85)?,
        thumb_jpeg: encode_jpeg(&thumb, 80)?,
        full,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_aspect_and_never_upscales() {
        let img = DynamicImage::new_rgb8(4000, 3000);
        let small = fit(&img, 400).unwrap();
        assert_eq!((small.width(), small.height()), (400, 300));
        let same = fit(&small, 1000).unwrap();
        assert_eq!(same.width(), 400);
    }

    #[test]
    fn jpeg_roundtrip_with_orientation() {
        let img = DynamicImage::new_rgb8(40, 20);
        let bytes = encode_jpeg(&img, 80).unwrap();
        let rotated = decode_jpeg(&bytes, Some(6)).unwrap(); // 90° clockwise
        assert_eq!((rotated.width(), rotated.height()), (20, 40));
    }
}
