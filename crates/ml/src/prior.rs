//! Location prior from eBird: which species are plausible where (and roughly when) a photo
//! was taken. Species absent from the local list are down-weighted, not excluded, so a
//! genuine rarity can still surface.
//!
//! - With GPS: species reported within 50 km over the last 30 days (eBird "recent nearby").
//! - Without GPS: every species ever reported in `home_region` (e.g. "US-PA").
//!
//! Lists are cached on disk so each area is fetched once per month.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Result, bail};
use chrono::{Datelike, Utc};

/// Probability multiplier for species not on the local list.
pub const OUT_OF_RANGE_WEIGHT: f32 = 0.02;

pub struct EbirdPrior {
    api_key: String,
    home_region: Option<String>,
    cache_dir: PathBuf,
    mem: Mutex<HashMap<String, Option<HashSet<String>>>>,
    http: reqwest::blocking::Client,
}

impl EbirdPrior {
    pub fn new(api_key: String, home_region: Option<String>, cache_dir: PathBuf) -> Self {
        Self {
            api_key,
            home_region,
            cache_dir,
            mem: Mutex::new(HashMap::new()),
            http: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .expect("http client"),
        }
    }

    /// Species codes plausible for this photo, or None when no prior applies.
    pub fn local_species(&self, lat: Option<f64>, lon: Option<f64>) -> Option<HashSet<String>> {
        let month = Utc::now().format("%Y-%m").to_string();
        let (key, url) = match (lat, lon) {
            (Some(la), Some(lo)) => {
                // ~0.25° grid cell keeps the cache small while staying local.
                let (gla, glo) = ((la * 4.0).round() / 4.0, (lo * 4.0).round() / 4.0);
                (
                    format!("geo_{gla:.2}_{glo:.2}_{month}"),
                    format!(
                        "https://api.ebird.org/v2/data/obs/geo/recent?lat={gla}&lng={glo}&dist=50&back=30&fmt=json"
                    ),
                )
            }
            _ => {
                let region = self.home_region.as_ref()?;
                (
                    format!("region_{region}_{}", Utc::now().year()),
                    format!("https://api.ebird.org/v2/product/spplist/{region}"),
                )
            }
        };
        if let Some(hit) = self.mem.lock().unwrap().get(&key) {
            return hit.clone();
        }
        let list = self.load_or_fetch(&key, &url).map_err(|e| {
            tracing::warn!(error = %e, "eBird lookup failed; ranking without location prior");
        });
        let list = list.ok().filter(|s| !s.is_empty());
        self.mem.lock().unwrap().insert(key, list.clone());
        list
    }

    fn load_or_fetch(&self, key: &str, url: &str) -> Result<HashSet<String>> {
        let path = self.cache_dir.join(format!("{key}.json"));
        if let Ok(text) = std::fs::read_to_string(&path) {
            return Ok(serde_json::from_str(&text)?);
        }
        let resp = self
            .http
            .get(url)
            .header("X-eBirdApiToken", &self.api_key)
            .send()?;
        if !resp.status().is_success() {
            bail!("eBird returned {}", resp.status());
        }
        let v: serde_json::Value = resp.json()?;
        let set: HashSet<String> = match v {
            // spplist: ["amerob", ...]; recent obs: [{"speciesCode": "amerob", ...}, ...]
            serde_json::Value::Array(items) => items
                .into_iter()
                .filter_map(|i| match i {
                    serde_json::Value::String(s) => Some(s),
                    serde_json::Value::Object(o) => o
                        .get("speciesCode")
                        .and_then(|c| c.as_str())
                        .map(str::to_string),
                    _ => None,
                })
                .collect(),
            _ => bail!("unexpected eBird response"),
        };
        std::fs::create_dir_all(&self.cache_dir)?;
        std::fs::write(&path, serde_json::to_string(&set)?)?;
        Ok(set)
    }
}

/// Re-weight probabilities by the local list and renormalize.
pub fn apply(probs: &[f32], codes: &[&str], local: &HashSet<String>) -> Vec<f32> {
    let weighted: Vec<f32> = probs
        .iter()
        .zip(codes)
        .map(|(p, c)| {
            if local.contains(*c) {
                *p
            } else {
                p * OUT_OF_RANGE_WEIGHT
            }
        })
        .collect();
    let sum: f32 = weighted.iter().sum();
    if sum <= 0.0 {
        return probs.to_vec();
    }
    weighted.into_iter().map(|w| w / sum).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prior_promotes_local_species() {
        let probs = [0.5, 0.4, 0.1];
        let codes = ["vagrant", "local1", "local2"];
        let local: HashSet<String> = ["local1", "local2"].iter().map(|s| s.to_string()).collect();
        let out = apply(&probs, &codes, &local);
        assert!(out[1] > out[0]);
        assert!((out.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }
}
