//! BioCLIP 2 zero-shot species classifier over the eBird taxonomy.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result, ensure};
use image::DynamicImage;
use ndarray::Array4;
use ort::session::Session;
use ort::value::TensorRef;
use photolake_core::model::BBox;

use crate::detector::{resize_exact, write_planar};
use crate::manifest::{ClassifierSpec, Species, SpeciesSpec, load_species};

pub struct Classifier {
    session: Mutex<Session>,
    spec: ClassifierSpec,
    pub species: Vec<Species>,
    /// Row-major [species x dim], L2-normalized.
    text: Vec<f32>,
}

/// Square crop around a box with `pad` context on each side, clamped to the image.
pub fn crop_box(img: &DynamicImage, b: &BBox, pad: f32) -> DynamicImage {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let side = (b.w * w).max(b.h * h) * (1.0 + 2.0 * pad);
    let side = side.min(w).min(h).max(8.0);
    let cx = (b.x + b.w / 2.0) * w;
    let cy = (b.y + b.h / 2.0) * h;
    let x0 = (cx - side / 2.0).clamp(0.0, w - side);
    let y0 = (cy - side / 2.0).clamp(0.0, h - side);
    img.crop_imm(x0 as u32, y0 as u32, side as u32, side as u32)
}

impl Classifier {
    pub fn load(
        models_dir: &Path,
        spec: &ClassifierSpec,
        species_spec: &SpeciesSpec,
        builder: impl Fn() -> Result<ort::session::builder::SessionBuilder>,
    ) -> Result<Self> {
        let path = models_dir.join(&spec.file);
        let session = builder()?
            .commit_from_file(&path)
            .with_context(|| format!("loading classifier {}", path.display()))?;
        let species = load_species(models_dir, species_spec)?;
        let raw = std::fs::read(models_dir.join(&species_spec.embeddings))?;
        let text: Vec<f32> = raw
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        ensure!(
            text.len() == species.len() * spec.dim,
            "embedding matrix has {} floats, expected {} species x {}",
            text.len(),
            species.len(),
            spec.dim
        );
        Ok(Self {
            session: Mutex::new(session),
            spec: spec.clone(),
            species,
            text,
        })
    }

    /// Image embeddings for several crops in one model run.
    fn embed_batch(&self, crops: &[DynamicImage]) -> Result<Vec<Vec<f32>>> {
        let s = self.spec.image_size as usize;
        let mut input = Array4::<f32>::zeros((crops.len(), 3, s, s));
        for (i, crop) in crops.iter().enumerate() {
            let rgb = resize_exact(crop, self.spec.image_size, self.spec.image_size)?;
            write_planar(
                input.index_axis_mut(ndarray::Axis(0), i),
                &rgb,
                self.spec.mean,
                self.spec.std,
            );
        }
        let mut session = self.session.lock().unwrap();
        let out = session.run(ort::inputs!["pixels" => TensorRef::from_array_view(&input)?])?;
        let (_, emb) = out["embedding"].try_extract_tensor::<f32>()?;
        Ok(emb
            .chunks_exact(self.spec.dim)
            .map(<[f32]>::to_vec)
            .collect())
    }

    /// Probability for every species (softmax over scaled cosine similarity).
    pub fn probabilities(&self, crop: &DynamicImage) -> Result<Vec<f32>> {
        let mut out = self.probabilities_batch(std::slice::from_ref(crop))?;
        Ok(out.pop().unwrap_or_default())
    }

    /// [`Self::probabilities`] for several crops, embedded in one batch.
    pub fn probabilities_batch(&self, crops: &[DynamicImage]) -> Result<Vec<Vec<f32>>> {
        if crops.is_empty() {
            return Ok(vec![]);
        }
        let dim = self.spec.dim;
        let scale = self.spec.logit_scale;
        Ok(self
            .embed_batch(crops)?
            .into_iter()
            .map(|emb| {
                let logits: Vec<f32> = self
                    .text
                    .chunks_exact(dim)
                    .map(|row| scale * row.iter().zip(&emb).map(|(a, b)| a * b).sum::<f32>())
                    .collect();
                let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                let exps: Vec<f32> = logits.iter().map(|l| (l - max).exp()).collect();
                let sum: f32 = exps.iter().sum();
                exps.into_iter().map(|e| e / sum).collect()
            })
            .collect())
    }
}
