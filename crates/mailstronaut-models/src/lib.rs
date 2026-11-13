//! Explicitly installed, checksum-verified CPU sentence embeddings.
//!
//! No downloads, networking, GPU, dynamic code or model-selected file paths. The
//! caller owns one encoder on a blocking worker; never load it on the UI thread.

use std::{fmt, fs, io::Read as _, path::Path};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use tokenizers::{Encoding, Tokenizer, TruncationParams};

/// The only sentence model accepted by this version.
pub const MODEL_REVISION: &str = "e8f8c211226b894fcb81acc59f3b34ba3efd5f42";
/// Hash-pinned installation metadata, also consumed by Nix packaging.
pub const MODEL_MANIFEST: &str = include_str!("../models/minilm.json");
/// Versioned model plus projection policy; changing windows invalidates old vectors.
pub const POLICY: &str = "minilm-e8f8c211-mean-l2-128-stride32-first8-v1";
/// Number of normalized coordinates in an embedding.
pub const DIMENSIONS: usize = 384;
/// Maximum document windows. Coverage is explicit when text exceeds this bound.
pub const MAX_WINDOWS: usize = 8;
const DOCUMENT_BYTES: usize = 32 * 1024;
const QUERY_BYTES: usize = 1024;

/// A redacted inference or installation failure. Contains no input or file paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// A declared model file is absent, unreadable or has an unexpected size/hash.
    Installation,
    /// The pinned model cannot be initialized or evaluated.
    Inference,
    /// A query exceeds the limit, contains controls, or has no readable text.
    Input,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Installation => "local model files are unavailable or fail verification",
            Self::Inference => "local model inference is unavailable",
            Self::Input => "search text is empty or exceeds supported limits",
        })
    }
}
impl std::error::Error for Error {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Artifact {
    name: String,
    size_bytes: u64,
    sha256: String,
}
#[derive(Deserialize)]
struct Manifest {
    files: Vec<Artifact>,
}

fn verified(root: &Path, artifact: &Artifact) -> Result<Vec<u8>, Error> {
    // Artifact paths come only from the embedded manifest, never configuration/mail.
    let path = root.join(&artifact.name);
    // Nix model packages may link immutable artifacts. Follow the link only to a
    // regular target; the complete target bytes must still match the embedded hash.
    if !fs::metadata(&path)
        .map_err(|_| Error::Installation)?
        .is_file()
    {
        return Err(Error::Installation);
    }
    let file = fs::File::open(path).map_err(|_| Error::Installation)?;
    if file.metadata().map_err(|_| Error::Installation)?.len() != artifact.size_bytes {
        return Err(Error::Installation);
    }
    let mut bytes = Vec::new();
    file.take(artifact.size_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Installation)?;
    if u64::try_from(bytes.len()).map_err(|_| Error::Installation)? != artifact.size_bytes
        || format!("{:x}", Sha256::digest(&bytes)) != artifact.sha256
    {
        return Err(Error::Installation);
    }
    Ok(bytes)
}

/// Owned bounded document embeddings, with honest analysis coverage.
#[derive(Debug)]
pub struct Projection {
    /// At most eight overlapping sentence-model windows, in source order.
    pub vectors: Vec<Vec<f32>>,
    /// Some source text was omitted by the byte/window limits.
    pub truncated: bool,
}

/// Existing Candle BERT and Hugging Face tokenizer, CPU only.
pub struct Encoder {
    model: BertModel,
    tokenizer: Tokenizer,
}
impl Encoder {
    /// Verify every pinned artifact and initialize the existing model architecture.
    /// # Errors
    /// Reports only redacted installation/inference failures. No implicit fetching.
    pub fn load(directory: &Path) -> Result<Self, Error> {
        let manifest: Manifest =
            serde_json::from_str(MODEL_MANIFEST).map_err(|_| Error::Installation)?;
        let mut weights = None;
        let mut config = None;
        let mut tokenizer = None;
        for artifact in &manifest.files {
            let bytes = verified(directory, artifact)?;
            match artifact.name.as_str() {
                "model.safetensors" => weights = Some(bytes),
                "config.json" => {
                    config = Some(
                        serde_json::from_slice::<Config>(&bytes)
                            .map_err(|_| Error::Installation)?,
                    );
                }
                "tokenizer.json" => {
                    tokenizer =
                        Some(Tokenizer::from_bytes(bytes).map_err(|_| Error::Installation)?);
                }
                _ => (),
            }
        }
        let config = config.ok_or(Error::Installation)?;
        let builder = VarBuilder::from_buffered_safetensors(
            weights.ok_or(Error::Installation)?,
            DType::F32,
            &Device::Cpu,
        )
        .map_err(|_| Error::Inference)?;
        let model = BertModel::load(builder, &config).map_err(|_| Error::Inference)?;
        let mut tokenizer = tokenizer.ok_or(Error::Installation)?;
        // The library handles special tokens, Unicode, overlap and overflow windows.
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: 128,
                stride: 32,
                ..Default::default()
            }))
            .map_err(|_| Error::Inference)?;
        tokenizer.with_padding(None);
        Ok(Self { model, tokenizer })
    }

    fn window(&self, encoded: &Encoding) -> Result<Vec<f32>, Error> {
        let result = (|| -> candle_core::Result<Vec<f32>> {
            let input = Tensor::new(encoded.get_ids(), &Device::Cpu)?.unsqueeze(0)?;
            let types = Tensor::new(encoded.get_type_ids(), &Device::Cpu)?.unsqueeze(0)?;
            let mask = Tensor::new(encoded.get_attention_mask(), &Device::Cpu)?.unsqueeze(0)?;
            let hidden = self.model.forward(&input, &types, Some(&mask))?;
            let mask = mask.to_dtype(DType::F32)?.unsqueeze(2)?;
            let pooled = hidden
                .broadcast_mul(&mask)?
                .sum(1)?
                .broadcast_div(&mask.sum(1)?)?;
            let normalized = pooled.broadcast_div(&pooled.sqr()?.sum_keepdim(1)?.sqrt()?)?;
            normalized.squeeze(0)?.to_vec1::<f32>()
        })()
        .map_err(|_| Error::Inference)?;
        if result.len() != DIMENSIONS || !result.iter().all(|value| value.is_finite()) {
            return Err(Error::Inference);
        }
        Ok(result)
    }

    /// Encode one bounded query. A query too long for the model is rejected rather
    /// than silently searching for a different, truncated meaning.
    /// # Errors
    /// Rejects empty/control/oversized inputs and failed inference.
    pub fn query(&self, text: &str) -> Result<Vec<f32>, Error> {
        if text.trim().is_empty() || text.len() > QUERY_BYTES || text.chars().any(char::is_control)
        {
            return Err(Error::Input);
        }
        let encoded = self
            .tokenizer
            .encode(text, true)
            .map_err(|_| Error::Input)?;
        if !encoded.get_overflowing().is_empty() {
            return Err(Error::Input);
        }
        self.window(&encoded)
    }

    /// Encode bounded readable text into overlapping library-tokenized windows.
    /// Empty/encrypted/unavailable bodies must be excluded by the canonical caller.
    /// # Errors
    /// Rejects empty text and reports redacted tokenizer/model failures.
    pub fn document(&self, text: &str) -> Result<Projection, Error> {
        let (bounded, byte_truncated) = bounded_text(text);
        if bounded.trim().is_empty() {
            return Err(Error::Input);
        }
        let encoded = self
            .tokenizer
            .encode(bounded, true)
            .map_err(|_| Error::Input)?;
        let mut windows = std::iter::once(&encoded).chain(encoded.get_overflowing());
        let vectors = windows
            .by_ref()
            .take(MAX_WINDOWS)
            .map(|window| self.window(window))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Projection {
            vectors,
            truncated: byte_truncated || windows.next().is_some(),
        })
    }
}

fn bounded_text(text: &str) -> (&str, bool) {
    let end = text.floor_char_boundary(DOCUMENT_BYTES.min(text.len()));
    (&text[..end], end < text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_and_same_size_corrupt_models_fail_closed_without_exposing_paths() {
        let directory = tempfile::tempdir().expect("private model");
        assert!(matches!(
            Encoder::load(directory.path()),
            Err(Error::Installation)
        ));
        let manifest: Manifest = serde_json::from_str(MODEL_MANIFEST).expect("manifest");
        let file = &manifest.files[0];
        fs::create_dir(directory.path().join("1_Pooling")).expect("pooling directory");
        fs::write(
            directory.path().join(&file.name),
            vec![b'x'; usize::try_from(file.size_bytes).expect("size")],
        )
        .expect("corrupt artifact");
        assert!(matches!(
            Encoder::load(directory.path()),
            Err(Error::Installation)
        ));
        assert_eq!(
            Error::Installation.to_string(),
            "local model files are unavailable or fail verification"
        );
    }

    #[test]
    fn nix_style_artifact_links_still_verify_the_complete_target_content() {
        let directory = tempfile::tempdir().expect("private artifact");
        let bytes = b"pinned model metadata";
        fs::write(directory.path().join("target"), bytes).expect("target");
        std::os::unix::fs::symlink("target", directory.path().join("artifact")).expect("link");
        let artifact = Artifact {
            name: "artifact".into(),
            size_bytes: u64::try_from(bytes.len()).expect("length"),
            sha256: format!("{:x}", Sha256::digest(bytes)),
        };
        assert_eq!(
            verified(directory.path(), &artifact).expect("verified link"),
            bytes
        );
        fs::write(directory.path().join("target"), vec![b'x'; bytes.len()])
            .expect("corrupt target");
        assert_eq!(
            verified(directory.path(), &artifact),
            Err(Error::Installation)
        );
    }

    #[test]
    fn byte_cap_keeps_utf8_and_reports_coverage() {
        let text = "é".repeat(DOCUMENT_BYTES);
        let (bounded, truncated) = bounded_text(&text);
        assert_eq!(bounded.len(), DOCUMENT_BYTES);
        assert!(truncated);
        assert_eq!(bounded_text("ordinary body"), ("ordinary body", false));
    }
}
