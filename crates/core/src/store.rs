//! Object storage access (S3-compatible: RustFS, MinIO, AWS S3, B2, R2, ...).

use std::time::Duration;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use opendal::{Operator, services::S3};
use rusty_s3::S3Action;

use crate::config::StorageConfig;

/// A handle on one bucket.
#[derive(Clone, Debug)]
pub struct ObjectStore {
    op: Operator,
    bucket: String,
}

impl ObjectStore {
    pub fn new(cfg: &StorageConfig, bucket: &str) -> Result<Self> {
        let builder = S3::default()
            .bucket(bucket)
            .endpoint(&cfg.endpoint)
            .region(&cfg.region)
            .access_key_id(&cfg.access_key)
            .secret_access_key(&cfg.secret_key)
            .disable_config_load()
            .disable_ec2_metadata();
        let op = Operator::new(builder)?.finish();
        Ok(Self {
            op,
            bucket: bucket.to_string(),
        })
    }

    pub fn photos(cfg: &StorageConfig) -> Result<Self> {
        Self::new(cfg, &cfg.photos_bucket)
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub fn operator(&self) -> &Operator {
        &self.op
    }

    pub async fn put(&self, key: &str, data: impl Into<Bytes>, content_type: &str) -> Result<()> {
        self.op
            .write_with(key, data.into())
            .content_type(content_type)
            .await
            .with_context(|| format!("uploading s3://{}/{key}", self.bucket))?;
        Ok(())
    }

    pub async fn get(&self, key: &str) -> Result<Bytes> {
        let buf = self
            .op
            .read(key)
            .await
            .with_context(|| format!("reading s3://{}/{key}", self.bucket))?;
        Ok(buf.to_bytes())
    }

    /// Size in bytes, or None when the object doesn't exist.
    pub async fn size(&self, key: &str) -> Result<Option<u64>> {
        match self.op.stat(key).await {
            Ok(meta) => Ok(Some(meta.content_length())),
            Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

/// Create the photos and warehouse buckets if they don't exist yet.
pub async fn ensure_buckets(cfg: &StorageConfig) -> Result<()> {
    let creds = rusty_s3::Credentials::new(&cfg.access_key, &cfg.secret_key);
    let endpoint: url::Url = cfg.endpoint.parse().context("invalid storage endpoint")?;
    let http = reqwest::Client::new();
    for name in [&cfg.photos_bucket, &cfg.warehouse_bucket] {
        let bucket = rusty_s3::Bucket::new(
            endpoint.clone(),
            rusty_s3::UrlStyle::Path,
            name.clone(),
            cfg.region.clone(),
        )?;
        let head = bucket
            .head_bucket(Some(&creds))
            .sign(Duration::from_secs(60));
        let resp = http
            .head(head)
            .send()
            .await
            .with_context(|| format!("contacting object storage at {}", cfg.endpoint))?;
        if resp.status().is_success() {
            continue;
        }
        let create = bucket.create_bucket(&creds).sign(Duration::from_secs(60));
        let resp = http.put(create).send().await?;
        let status = resp.status();
        if !status.is_success() && status.as_u16() != 409 {
            bail!(
                "creating bucket {name}: {status} {}",
                resp.text().await.unwrap_or_default()
            );
        }
        tracing::info!(bucket = %name, "created bucket");
    }
    Ok(())
}
