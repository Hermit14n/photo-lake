//! RT-DETR (COCO) bird detector.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::DynamicImage;
use ndarray::Array4;
use ort::session::Session;
use ort::value::TensorRef;
use photolake_core::model::BBox;

use crate::manifest::DetectorSpec;

pub struct Detector {
    session: Mutex<Session>,
    size: u32,
    bird_class: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct Det {
    pub bbox: BBox,
    pub score: f32,
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

pub fn iou(a: &BBox, b: &BBox) -> f32 {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = (a.x + a.w).min(b.x + b.w);
    let y2 = (a.y + a.h).min(b.y + b.h);
    let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let union = a.w * a.h + b.w * b.h - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

/// Greedy non-maximum suppression (DETR rarely duplicates, but tiles do).
pub fn nms(mut dets: Vec<Det>, iou_threshold: f32) -> Vec<Det> {
    dets.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut keep: Vec<Det> = Vec::new();
    for d in dets {
        if keep.iter().all(|k| iou(&k.bbox, &d.bbox) < iou_threshold) {
            keep.push(d);
        }
    }
    keep
}

/// Resize to exactly `w`x`h` RGB (aspect is not preserved, matching the HF processors).
pub(crate) fn resize_exact(img: &DynamicImage, w: u32, h: u32) -> Result<image::RgbImage> {
    let src = DynamicImage::ImageRgb8(img.to_rgb8());
    let mut dst = DynamicImage::new_rgb8(w, h);
    Resizer::new().resize(
        &src,
        &mut dst,
        &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear)),
    )?;
    Ok(dst.to_rgb8())
}

impl Detector {
    pub fn load(
        models_dir: &Path,
        spec: &DetectorSpec,
        builder: impl Fn() -> Result<ort::session::builder::SessionBuilder>,
    ) -> Result<Self> {
        let path = models_dir.join(&spec.file);
        let session = builder()?
            .commit_from_file(&path)
            .with_context(|| format!("loading detector {}", path.display()))?;
        Ok(Self {
            session: Mutex::new(session),
            size: spec.input_size,
            bird_class: spec.bird_class,
        })
    }

    /// Detect birds in `img`; boxes are normalized to `img`.
    pub fn detect(&self, img: &DynamicImage, threshold: f32) -> Result<Vec<Det>> {
        let mut out = self.detect_batch(std::slice::from_ref(img), threshold)?;
        Ok(out.pop().unwrap_or_default())
    }

    /// Detect birds in several images with one model run (one GPU launch instead of many).
    pub fn detect_batch(&self, imgs: &[DynamicImage], threshold: f32) -> Result<Vec<Vec<Det>>> {
        if imgs.is_empty() {
            return Ok(vec![]);
        }
        let s = self.size as usize;
        let mut input = Array4::<f32>::zeros((imgs.len(), 3, s, s));
        for (i, img) in imgs.iter().enumerate() {
            let rgb = resize_exact(img, self.size, self.size)?;
            write_planar(
                input.index_axis_mut(ndarray::Axis(0), i),
                &rgb,
                [0.0; 3],
                [1.0; 3],
            );
        }
        let mut session = self.session.lock().unwrap();
        let outputs =
            session.run(ort::inputs!["pixel_values" => TensorRef::from_array_view(&input)?])?;
        let logits = outputs["logits"].try_extract_array::<f32>()?;
        let boxes = outputs["pred_boxes"].try_extract_array::<f32>()?;
        let n = logits.shape()[1];
        let mut all = Vec::with_capacity(imgs.len());
        for b in 0..imgs.len() {
            let mut dets = Vec::new();
            for q in 0..n {
                let score = sigmoid(logits[[b, q, self.bird_class]]);
                if score < threshold {
                    continue;
                }
                let (cx, cy, w, h) = (
                    boxes[[b, q, 0]],
                    boxes[[b, q, 1]],
                    boxes[[b, q, 2]],
                    boxes[[b, q, 3]],
                );
                let x = (cx - w / 2.0).clamp(0.0, 1.0);
                let y = (cy - h / 2.0).clamp(0.0, 1.0);
                let bbox = BBox {
                    x,
                    y,
                    w: w.min(1.0 - x),
                    h: h.min(1.0 - y),
                };
                if bbox.w > 0.0 && bbox.h > 0.0 {
                    dets.push(Det { bbox, score });
                }
            }
            all.push(nms(dets, 0.6));
        }
        Ok(all)
    }

    /// A 2x2 grid of overlapping tiles (one batched run), for small or distant birds.
    /// Used after the full-frame pass found nothing.
    pub fn detect_tiles(&self, img: &DynamicImage, threshold: f32) -> Result<Vec<Det>> {
        let (w, h) = (img.width(), img.height());
        let (tw, th) = ((w as f32 * 0.55) as u32, (h as f32 * 0.55) as u32);
        let offsets = [(0, 0), (w - tw, 0), (0, h - th), (w - tw, h - th)];
        let tiles: Vec<DynamicImage> = offsets
            .iter()
            .map(|&(ox, oy)| img.crop_imm(ox, oy, tw, th))
            .collect();
        let mut all = Vec::new();
        for ((ox, oy), dets) in offsets
            .into_iter()
            .zip(self.detect_batch(&tiles, threshold)?)
        {
            for d in dets {
                all.push(Det {
                    bbox: BBox {
                        x: (ox as f32 + d.bbox.x * tw as f32) / w as f32,
                        y: (oy as f32 + d.bbox.y * th as f32) / h as f32,
                        w: d.bbox.w * tw as f32 / w as f32,
                        h: d.bbox.h * th as f32 / h as f32,
                    },
                    score: d.score,
                });
            }
        }
        Ok(nms(all, 0.5))
    }
}

/// Write an RGB image into a [3, h, w] tensor slice as `(v / 255 - mean) / std`.
pub(crate) fn write_planar(
    mut dst: ndarray::ArrayViewMut3<f32>,
    rgb: &image::RgbImage,
    mean: [f32; 3],
    std: [f32; 3],
) {
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    let raw = rgb.as_raw();
    for c in 0..3 {
        let mut plane = dst.index_axis_mut(ndarray::Axis(0), c);
        let plane = plane
            .as_slice_mut()
            .expect("freshly allocated tensors are contiguous");
        let (m, s) = (mean[c], std[c]);
        for (i, px) in plane.iter_mut().enumerate().take(w * h) {
            *px = (raw[i * 3 + c] as f32 / 255.0 - m) / s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nms_drops_overlaps() {
        let b = |x: f32, s: f32| Det {
            bbox: BBox {
                x,
                y: 0.0,
                w: 0.2,
                h: 0.2,
            },
            score: s,
        };
        let kept = nms(vec![b(0.0, 0.5), b(0.01, 0.9), b(0.6, 0.4)], 0.5);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].score, 0.9);
    }
}
