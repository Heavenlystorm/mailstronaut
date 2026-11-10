//! Encoder-only parity/resource probe. Reuses Candle's complete BERT implementation.
#![forbid(unsafe_code)]

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{env, error::Error, fs, io, io::Read as _, path::Path, time::Instant};
use tokenizers::{Tokenizer, TruncationParams};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    dimensions: usize,
    max_tokens: usize,
    files: Vec<Artifact>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Artifact {
    name: String,
    size_bytes: u64,
    sha256: String,
}
#[derive(Deserialize)]
struct Text {
    text: String,
}
#[derive(Deserialize)]
struct Fixtures {
    documents: Vec<Text>,
    queries: Vec<Text>,
}
#[derive(Serialize)]
struct Output {
    // Lossless interchange avoids the known Cranelift JSON F32 formatter path.
    documents: Vec<Vec<u32>>,
    queries: Vec<Vec<u32>>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::other(message)
}

fn verified(root: &Path, artifact: &Artifact) -> Result<Vec<u8>> {
    if ![
        "model.safetensors",
        "tokenizer.json",
        "config.json",
        "tokenizer_config.json",
        "special_tokens_map.json",
        "sentence_bert_config.json",
        "1_Pooling/config.json",
    ]
    .contains(&artifact.name.as_str())
        || artifact.size_bytes > 512 * 1024 * 1024
    {
        return Err(invalid("unsupported artifact").into());
    }
    let path = root.join(&artifact.name);
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err(invalid("expected regular artifact").into());
    }
    let file = fs::File::open(path)?;
    if file.metadata()?.len() != artifact.size_bytes {
        return Err(invalid("artifact size mismatch").into());
    }
    let mut bytes = Vec::new();
    file.take(artifact.size_bytes + 1).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len())? != artifact.size_bytes
        || format!("{:x}", Sha256::digest(&bytes)) != artifact.sha256
    {
        return Err(invalid("artifact checksum mismatch").into());
    }
    Ok(bytes)
}

struct Encoder {
    model: BertModel,
    tokenizer: Tokenizer,
}
impl Encoder {
    fn load(root: &Path) -> Result<Self> {
        let manifest: Manifest = serde_json::from_str(include_str!("minilm-model-manifest.json"))?;
        let mut weights = None;
        let mut config = None;
        let mut tokenizer = None;
        for artifact in &manifest.files {
            let bytes = verified(root, artifact)?;
            match artifact.name.as_str() {
                "model.safetensors" => weights = Some(bytes),
                "config.json" => config = Some(serde_json::from_slice::<Config>(&bytes)?),
                "tokenizer.json" => tokenizer = Some(Tokenizer::from_bytes(bytes)?),
                _ => (),
            }
        }
        let config = config.ok_or_else(|| invalid("missing configuration"))?;
        if config.hidden_size != manifest.dimensions
            || manifest.dimensions != 384
            || config.max_position_embeddings != 512
            || manifest.max_tokens != 128
        {
            return Err(invalid("unexpected encoder configuration").into());
        }
        let builder = VarBuilder::from_buffered_safetensors(
            weights.ok_or_else(|| invalid("missing weights"))?,
            DType::F32,
            &Device::Cpu,
        )?;
        let model = BertModel::load(builder, &config)?;
        let mut tokenizer = tokenizer.ok_or_else(|| invalid("missing tokenizer"))?;
        tokenizer.with_truncation(Some(TruncationParams {
            max_length: manifest.max_tokens,
            ..Default::default()
        }))?;
        tokenizer.with_padding(None);
        Ok(Self { model, tokenizer })
    }

    fn encode(&self, text: &str) -> Result<Vec<f32>> {
        let encoded = self.tokenizer.encode(text, true)?;
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
        let vector = normalized.squeeze(0)?.to_vec1::<f32>()?;
        if vector.len() != 384 || !vector.iter().all(|value| value.is_finite()) {
            return Err(invalid("invalid embedding").into());
        }
        Ok(vector)
    }
}

fn encode(encoder: &Encoder, rows: &[Text]) -> Result<Vec<Vec<u32>>> {
    rows.iter()
        .map(|row| {
            Ok(encoder
                .encode(&row.text)?
                .into_iter()
                .map(f32::to_bits)
                .collect())
        })
        .collect()
}

fn main() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if !(2..=3).contains(&args.len()) {
        return Err(
            invalid("expected model directory, vector output and optional fixture file").into(),
        );
    }
    let start = Instant::now();
    let encoder = Encoder::load(Path::new(&args[0]))?;
    println!(
        "hash_verified_model_load_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.
    );
    let input: io::Result<Vec<u8>> = args.get(2).map_or_else(
        || Ok(include_bytes!("relevance.json").to_vec()),
        |path| {
            let mut bytes = Vec::new();
            fs::File::open(path)?
                .take(1_048_577)
                .read_to_end(&mut bytes)?;
            Ok(bytes)
        },
    );
    let input = input?;
    if input.len() > 1_048_576 {
        return Err(invalid("oversized fixture").into());
    }
    let fixtures: Fixtures = serde_json::from_slice(&input)?;
    let output = Output {
        documents: encode(&encoder, &fixtures.documents)?,
        queries: encode(&encoder, &fixtures.queries)?,
    };
    fs::write(&args[1], serde_json::to_vec(&output)?)?;
    let mut samples = Vec::new();
    for _ in 0..30 {
        let start = Instant::now();
        std::hint::black_box(encoder.encode("reminder about doctoral school re-enrollment")?);
        samples.push(start.elapsed().as_secs_f64() * 1000.);
    }
    samples.sort_by(f64::total_cmp);
    println!("warm_query_encode_p95_ms={:.3}", samples[28]);
    for line in fs::read_to_string("/proc/self/status")?.lines() {
        if line.starts_with("VmRSS:") {
            println!("steady_{line}");
        }
    }
    Ok(())
}
