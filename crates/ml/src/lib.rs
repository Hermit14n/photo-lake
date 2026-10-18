//! Bird detection and species identification.
//!
//! The pipeline mirrors how Merlin Photo ID works conceptually: find birds, classify each
//! crop, then re-rank species by what is plausible at the photo's location and date.

use chrono::{DateTime, Utc};
use image::DynamicImage;
use photolake_core::model::BBox;

/// Context that helps species ranking.
#[derive(Debug, Clone, Default)]
pub struct AnalyzeContext {
    pub captured_at: Option<DateTime<Utc>>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpeciesGuess {
    pub species_code: String,
    pub common_name: String,
    pub scientific_name: String,
    /// Classifier probability.
    pub cls_score: f32,
    /// Score after location/season re-ranking.
    pub final_score: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BirdFinding {
    pub bbox: BBox,
    pub det_score: f32,
    /// Best first; may be empty when no classifier is configured.
    pub candidates: Vec<SpeciesGuess>,
}

pub trait Analyzer: Send + Sync {
    /// Identifies the model set; stored with every detection so runs can be compared.
    fn model_version(&self) -> String;

    /// Where inference runs: "cuda" or "cpu".
    fn device(&self) -> &str {
        "cpu"
    }

    /// Find birds in an (already orientation-corrected) image.
    fn analyze(
        &self,
        image: &DynamicImage,
        ctx: &AnalyzeContext,
    ) -> anyhow::Result<Vec<BirdFinding>>;
}

pub mod classifier;
pub mod detector;
pub mod manifest;
pub mod prior;

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use photolake_core::config::MlConfig;

/// Padding (fraction of box size on each side) added around a bird before classifying.
const CROP_PAD: f32 = 0.15;

/// The on-device pipeline: RT-DETR → BioCLIP 2 → eBird prior.
pub struct BirdAnalyzer {
    device: &'static str,
    detector: detector::Detector,
    classifier: classifier::Classifier,
    prior: Option<prior::EbirdPrior>,
    version: String,
    threshold: f32,
    top_k: usize,
    tiling: bool,
}

fn base_builder(
    env: &ort::environment::Environment,
) -> Result<ort::session::builder::SessionBuilder> {
    use ort::session::builder::GraphOptimizationLevel;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    ort::session::Session::builder(env)?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .with_intra_threads(threads)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(feature = "cuda")]
fn cuda_builder(
    env: &ort::environment::Environment,
) -> Result<ort::session::builder::SessionBuilder> {
    base_builder(env)?
        .with_execution_providers([ort::ep::CUDA::default().build().error_on_failure()])
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// GPU builds load `libonnxruntime.so` at run time: `$ORT_DYLIB_PATH`, else next to the
/// executable (`lib/` beside it in packages and containers), else the dev checkout's `.ort/gpu`.
#[cfg(feature = "cuda")]
fn ort_library() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    const NAME: &str = "libonnxruntime.so";
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("ORT_DYLIB_PATH").filter(|p| !p.is_empty()) {
        candidates.push(p.into());
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from))
    {
        candidates.push(dir.join("lib").join(NAME));
        candidates.push(dir.join(NAME));
        candidates.push(dir.join("../lib/photolake").join(NAME));
    }
    candidates
        .push(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.ort/gpu/lib")).join(NAME));
    candidates.into_iter().find(|p| p.exists())
}

/// Pick the inference device. "auto" uses the GPU when this build supports CUDA and the
/// driver/CUDA/cuDNN libraries load, else the CPU; "cuda" fails instead of falling back.
fn choose_device(env: &ort::environment::Environment, requested: &str) -> Result<&'static str> {
    match requested {
        "cpu" => Ok("cpu"),
        "auto" | "cuda" | "gpu" => {
            #[cfg(feature = "cuda")]
            {
                match cuda_builder(env) {
                    Ok(_) => Ok("cuda"),
                    Err(e) if requested == "auto" => {
                        tracing::warn!(error = %e, "GPU unavailable; running bird ML on the CPU");
                        Ok("cpu")
                    }
                    Err(e) => anyhow::bail!("CUDA unavailable: {e}"),
                }
            }
            #[cfg(not(feature = "cuda"))]
            {
                let _ = env;
                if requested == "auto" {
                    Ok("cpu")
                } else {
                    anyhow::bail!(
                        "this build has no GPU support (rebuild with --features photolake-ml/cuda \
                         or use the -cuda container image)"
                    )
                }
            }
        }
        other => anyhow::bail!("ml.device = {other:?}; use \"auto\", \"cuda\" or \"cpu\""),
    }
}

fn session_builder(
    env: &ort::environment::Environment,
    device: &str,
) -> Result<ort::session::builder::SessionBuilder> {
    match device {
        #[cfg(feature = "cuda")]
        "cuda" => cuda_builder(env),
        _ => base_builder(env),
    }
}

impl BirdAnalyzer {
    pub fn load(models_dir: &Path, cfg: &MlConfig, cache_dir: &Path) -> Result<Self> {
        let m = manifest::Manifest::load(models_dir)?;
        #[cfg(feature = "cuda")]
        let env = {
            let lib = ort_library().ok_or_else(|| {
                anyhow::anyhow!(
                    "ONNX Runtime library not found (run scripts/fetch-onnxruntime-gpu.sh, or set \
                     ORT_DYLIB_PATH to libonnxruntime.so)"
                )
            })?;
            tracing::debug!(path = %lib.display(), "loading ONNX Runtime");
            ort::init_from(&lib)?.with_name("photolake").build()?
        };
        #[cfg(not(feature = "cuda"))]
        let env = ort::init().with_name("photolake").build()?;
        let mut device = choose_device(&env, &cfg.device)?;
        let load_models = |device: &str| -> Result<_> {
            let builder = || session_builder(&env, device);
            let detector = detector::Detector::load(models_dir, &m.detector, builder)?;
            let classifier =
                classifier::Classifier::load(models_dir, &m.classifier, &m.species, builder)?;
            // The CUDA provider registers even when cuDNN is missing and only fails on the first
            // run, so prove the device works now (this also warms up the GPU kernels).
            let probe = DynamicImage::new_rgb8(64, 64);
            detector.detect(&probe, 1.0)?;
            classifier.probabilities(&probe)?;
            Ok((detector, classifier))
        };
        let (detector, classifier) = match load_models(device) {
            Ok(models) => models,
            Err(e) if device == "cuda" && cfg.device == "auto" => {
                tracing::warn!(
                    error = %format!("{e:#}"),
                    "GPU inference failed (is cuDNN 9 installed?); running bird ML on the CPU"
                );
                device = "cpu";
                load_models(device)?
            }
            Err(e) => return Err(e),
        };
        let prior = cfg
            .ebird_api_key
            .clone()
            .filter(|k| !k.is_empty())
            .map(|k| prior::EbirdPrior::new(k, cfg.home_region.clone(), cache_dir.join("ebird")));
        let version = format!(
            "{}{}",
            m.version,
            if prior.is_some() { "+ebird" } else { "" }
        );
        tracing::info!(%version, species = classifier.species.len(), device, "bird models loaded");
        Ok(Self {
            device,
            detector,
            classifier,
            prior,
            version,
            threshold: cfg.detection_threshold,
            top_k: cfg.top_k.max(1),
            tiling: cfg.tiling,
        })
    }

    pub fn species(&self) -> &[manifest::Species] {
        &self.classifier.species
    }

    /// Top-k species for one crop, given its classifier probabilities.
    fn rank(&self, probs: Vec<f32>, local: Option<&HashSet<String>>) -> Vec<SpeciesGuess> {
        let species = &self.classifier.species;
        let finals = match local {
            Some(set) => {
                let codes: Vec<&str> = species.iter().map(|s| s.code.as_str()).collect();
                prior::apply(&probs, &codes, set)
            }
            None => probs.clone(),
        };
        let mut idx: Vec<usize> = (0..species.len()).collect();
        idx.sort_by(|&a, &b| finals[b].total_cmp(&finals[a]));
        idx.into_iter()
            .take(self.top_k)
            .map(|i| SpeciesGuess {
                species_code: species[i].code.clone(),
                common_name: species[i].common.clone(),
                scientific_name: species[i].sci.clone(),
                cls_score: probs[i],
                final_score: finals[i],
            })
            .collect()
    }
}

impl Analyzer for BirdAnalyzer {
    fn model_version(&self) -> String {
        self.version.clone()
    }

    fn device(&self) -> &str {
        self.device
    }

    fn analyze(&self, image: &DynamicImage, ctx: &AnalyzeContext) -> Result<Vec<BirdFinding>> {
        let mut dets = self.detector.detect(image, self.threshold)?;
        if dets.is_empty() && self.tiling {
            dets = self.detector.detect_tiles(image, self.threshold)?;
        }
        if dets.is_empty() {
            return Ok(vec![]);
        }
        // All crops of a photo go through the classifier as one batch.
        let crops: Vec<DynamicImage> = dets
            .iter()
            .map(|d| classifier::crop_box(image, &d.bbox, CROP_PAD))
            .collect();
        let probs = self.classifier.probabilities_batch(&crops)?;
        let local = self
            .prior
            .as_ref()
            .and_then(|p| p.local_species(ctx.lat, ctx.lon));
        Ok(dets
            .into_iter()
            .zip(probs)
            .map(|(d, p)| BirdFinding {
                bbox: d.bbox,
                det_score: d.score,
                candidates: self.rank(p, local.as_ref()),
            })
            .collect())
    }
}
