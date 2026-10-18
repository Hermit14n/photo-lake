//! `manifest.json` written by `tools/export_models.py`.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub detector: DetectorSpec,
    pub classifier: ClassifierSpec,
    pub species: SpeciesSpec,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DetectorSpec {
    pub file: String,
    pub input_size: u32,
    pub bird_class: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClassifierSpec {
    pub file: String,
    pub image_size: u32,
    pub mean: [f32; 3],
    pub std: [f32; 3],
    pub dim: usize,
    pub logit_scale: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpeciesSpec {
    pub file: String,
    pub embeddings: String,
    pub count: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Species {
    pub code: String,
    pub common: String,
    pub sci: String,
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub order: String,
}

impl Manifest {
    pub fn load(models_dir: &Path) -> Result<Self> {
        let p = models_dir.join("manifest.json");
        let text = std::fs::read_to_string(&p).with_context(|| {
            format!(
                "reading {} (run tools/export_models.py to download the models)",
                p.display()
            )
        })?;
        Ok(serde_json::from_str(&text)?)
    }
}

pub fn load_species(models_dir: &Path, spec: &SpeciesSpec) -> Result<Vec<Species>> {
    let text = std::fs::read_to_string(models_dir.join(&spec.file))?;
    Ok(serde_json::from_str(&text)?)
}
