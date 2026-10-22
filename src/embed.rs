//! Potion-base-8M static embeddings and model asset verification.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use model2vec_rs::model::StaticModel;
use sha2::{Digest, Sha256};

use crate::tuning;

const MODEL_REPO: &str = "minishlab/potion-base-8M";
const MODEL_REVISION: &str = "bf8b056651a2c21b8d2565580b8569da283cab23";
const FILES: [(&str, &str); 6] = [
    (
        "model.safetensors",
        "f65d0f325faadc1e121c319e2faa41170d3fa07d8c89abd48ca5358d9a223de2",
    ),
    (
        "tokenizer.json",
        "e67e803f624fb4d67dea1c730d06e1067e1b14d830e2c2202569e3ef0f70bb50",
    ),
    (
        "config.json",
        "2a6ac0e9aaa356a68a5688070db78fc3a464fefe85d2f06a1905ce3718687553",
    ),
    (
        "tokenizer_config.json",
        "6725995e3ab3039857ff5bd99178a7cdf42863abb04449e7bb31feb1f55fe567",
    ),
    (
        "special_tokens_map.json",
        "a9e8fb6f99fb0b8803f0e6942fdf4d95d6645204620b67dc3310a1024bcbac59",
    ),
    (
        "vocab.txt",
        "1394523a67ddd404a825428018c0582a6998bcfa044ecbcbf1f4d71adb94c61c",
    ),
];

/// A verified local model. Potion outputs 256-dimensional vectors.
pub struct Embedder {
    model: StaticModel,
}

impl Embedder {
    /// Verify installed assets, downloading the pinned upstream revision if needed.
    pub async fn load(model_dir: &Path, model_url: &str) -> Result<Self> {
        let model_dir = model_dir.to_owned();
        let base = model_url.trim_end_matches('/').to_owned();
        ensure_model_files(&model_dir, &base).await?;
        let path = model_dir.to_string_lossy().into_owned();
        let model = tokio::task::spawn_blocking(move || {
            StaticModel::from_pretrained(&path, None, Some(true), None)
                .context("loading verified Potion model")
        })
        .await
        .context("model load worker failed")??;
        Ok(Self { model })
    }

    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut vectors = self.model.encode(&[text.to_owned()]);
        let vector = vectors.pop().context("model returned no embedding")?;
        normalize_embedding(vector)
    }

    pub fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let vectors = self.model.encode(texts);
        if vectors.len() != texts.len() {
            bail!(
                "model returned {} vectors for {} inputs",
                vectors.len(),
                texts.len()
            );
        }
        vectors.into_iter().map(normalize_embedding).collect()
    }
}

async fn ensure_model_files(dir: &Path, base_url: &str) -> Result<()> {
    fs::create_dir_all(dir)
        .with_context(|| format!("creating model directory {}", dir.display()))?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(
            tuning::MODEL_DOWNLOAD_TIMEOUT_SECS,
        ))
        .build()?;
    let default_base = format!("https://huggingface.co/{MODEL_REPO}/resolve/{MODEL_REVISION}");
    let url_base = if base_url == format!("https://huggingface.co/{MODEL_REPO}") {
        default_base.as_str()
    } else {
        base_url
    };
    for (name, expected) in FILES {
        let target = dir.join(name);
        if target.exists() {
            verify_file(&target, expected)?;
            continue;
        }
        let url = format!("{url_base}/{name}");
        let mut response = client.get(&url).send().await?.error_for_status()?;
        anyhow::ensure!(
            response
                .content_length()
                .is_none_or(|length| length <= tuning::MODEL_MAX_ASSET_BYTES as u64),
            "model asset {name} exceeds the download limit"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(
                bytes.len().saturating_add(chunk.len()) <= tuning::MODEL_MAX_ASSET_BYTES,
                "model asset {name} exceeds the download limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        if hex_sha256(&bytes) != expected {
            bail!("SHA-256 mismatch for downloaded model asset {name}");
        }
        let temporary = temporary_path(&target);
        fs::write(&temporary, &bytes)
            .with_context(|| format!("writing {}", temporary.display()))?;
        fs::rename(&temporary, &target)
            .with_context(|| format!("installing {}", target.display()))?;
    }
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".download");
    path.with_file_name(name)
}

fn verify_file(path: &Path, expected: &str) -> Result<()> {
    let bytes =
        fs::read(path).with_context(|| format!("reading model asset {}", path.display()))?;
    if hex_sha256(&bytes) != expected {
        bail!("SHA-256 mismatch for model asset {}", path.display());
    }
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn normalize_embedding(mut vector: Vec<f32>) -> Result<Vec<f32>> {
    if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
        bail!("model produced an empty or non-finite embedding");
    }
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        bail!("model produced a zero embedding");
    }
    for value in &mut vector {
        *value /= norm;
    }
    Ok(vector)
}

/// Text used for article-level similarity: title plus the first 120 body words.
pub fn article_embedding_input(title: &str, text: &str) -> String {
    let lede = text
        .split_whitespace()
        .take(tuning::ARTICLE_EMBED_WORDS)
        .collect::<Vec<_>>()
        .join(" ");
    if title.trim().is_empty() {
        lede
    } else if lede.is_empty() {
        title.trim().to_owned()
    } else {
        format!("{}\n{}", title.trim(), lede)
    }
}

/// Dot product of normalized vectors, or `None` when dimensions/data disagree.
pub fn similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    (a.len() == b.len() && !a.is_empty() && a.iter().chain(b).all(|x| x.is_finite()))
        .then(|| a.iter().zip(b).map(|(x, y)| x * y).sum())
}

#[cfg(test)]
impl Embedder {
    /// Tiny, deterministic static model for offline pipeline integration only.
    /// It exercises model2vec's actual tokenizer/pooling, not Potion quality.
    pub(crate) fn fixture_model() -> Result<Self> {
        let tokenizer = serde_json::json!({
            "version":"1.0", "truncation":null, "padding":null,
            "added_tokens":[], "normalizer":{"type":"Lowercase"},
            "pre_tokenizer":{"type":"Whitespace"}, "post_processor":null,
            "decoder":null,
            "model":{"type":"WordLevel", "vocab":{
                "[UNK]":0,"housing":1,"harbour":2,"rocket":3,"garden":4
            },"unk_token":"[UNK]"}
        });
        let mut header = serde_json::to_vec(&serde_json::json!({
            "embeddings":{"dtype":"F32","shape":[5,256],"data_offsets":[0,5*256*4]}
        }))?;
        while header.len() % 8 != 0 {
            header.push(b' ');
        }
        let mut tensor = (header.len() as u64).to_le_bytes().to_vec();
        tensor.extend_from_slice(&header);
        for row in 0..5 {
            for dimension in 0..256 {
                let value: f32 = if row > 0 && dimension == row - 1 {
                    if row == 4 { 0.5 } else { 1.0 }
                } else {
                    0.0
                };
                tensor.extend_from_slice(&value.to_le_bytes());
            }
        }
        let model = StaticModel::from_bytes(
            serde_json::to_vec(&tokenizer)?,
            tensor,
            b"{\"normalize\":true}",
            Some(true),
        )?;
        Ok(Self { model })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn static_model_pooling_and_asset_checks_are_offline_and_normalized() {
        let model = Embedder::fixture_model().unwrap();
        let housing = model.embed("Housing housing").unwrap();
        let harbour = model.embed("Harbour").unwrap();
        assert_eq!(housing.len(), 256);
        assert!((housing.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 1e-6);
        assert!(similarity(&housing, &harbour).unwrap().abs() < 1e-6);
        assert!(model.embed("").is_err());
        assert!(model.embed_batch(&[]).unwrap().is_empty());
        let path = std::env::temp_dir().join(format!(
            "wagwan-asset-{}-{}",
            std::process::id(),
            crate::now()
        ));
        fs::write(&path, b"fixture model asset").unwrap();
        let digest = hex_sha256(b"fixture model asset");
        assert!(verify_file(&path, &digest).is_ok());
        fs::write(&path, b"corrupted model asset").unwrap();
        assert!(verify_file(&path, &digest).is_err());
        fs::remove_file(path).unwrap();
    }
}
