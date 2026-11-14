//! Bounded, isolated NLI parity probe using Candle's existing classifier.
#![forbid(unsafe_code)]

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::xlm_roberta::{Config, XLMRobertaForSequenceClassification};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{env, error::Error, fs, io, io::Read as _, path::Path, time::Instant};
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams, TruncationStrategy};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Artifact {
    name: String,
    url: String,
    size_bytes: u64,
    sha256: String,
    hash: String,
}
#[derive(Deserialize)]
struct Manifest {
    files: Vec<Artifact>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pair {
    id: String,
    premise: String,
    hypothesis: String,
    pad_to: Option<usize>,
}
#[derive(Serialize)]
struct Observation {
    id: String,
    input_ids: Vec<u32>,
    attention_mask: Vec<u32>,
    logits_bits: Vec<u32>,
    truncated: bool,
    micros: u64,
}

fn invalid(message: &str) -> io::Error {
    io::Error::other(message)
}

fn verified(root: &Path, artifact: &Artifact) -> Result<(Vec<u8>, usize)> {
    const PREFIX: &str = "https://huggingface.co/MoritzLaurer/multilingual-MiniLMv2-L6-mnli-xnli/resolve/0a71e92a985b6e1ad1828cf67ce9c459639c1dca/";
    if ![
        "model.safetensors",
        "config.json",
        "tokenizer.json",
        "tokenizer_config.json",
        "special_tokens_map.json",
        "README.md",
    ]
    .contains(&artifact.name.as_str())
        || artifact.size_bytes > 512 * 1024 * 1024
        || artifact.url != format!("{PREFIX}{}", artifact.name)
        || !artifact.hash.starts_with("sha256-")
    {
        return Err(invalid("unsupported artifact").into());
    }
    let path = root.join(&artifact.name);
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err(invalid("expected regular artifact").into());
    }
    let mut file = fs::File::open(path)?;
    if file.metadata()?.len() != artifact.size_bytes {
        return Err(invalid("artifact size mismatch").into());
    }
    let (bytes, offset) = if artifact.name == "model.safetensors" {
        let mut header = [0_u8; 8];
        file.read_exact(&mut header)?;
        let header_length = usize::try_from(u64::from_le_bytes(header))?;
        let length = usize::try_from(artifact.size_bytes)?;
        if header_length > 1024 * 1024 || header_length + 8 > length {
            return Err(invalid("unsupported tensor header").into());
        }
        // Keep the exact checkpoint bytes, but align its tensor-data start. The
        // safe library converter otherwise copies the giant embedding twice.
        let mut bytes = vec![0_u8; length + 7];
        let offset = (8 - (bytes.as_ptr().addr() + 8 + header_length) % 8) % 8;
        bytes[offset..offset + 8].copy_from_slice(&header);
        file.read_exact(&mut bytes[offset + 8..offset + length])?;
        if file.read(&mut [0_u8; 1])? != 0 {
            return Err(invalid("artifact size changed").into());
        }
        bytes.truncate(offset + length);
        (bytes, offset)
    } else {
        let mut bytes = Vec::new();
        file.take(artifact.size_bytes + 1).read_to_end(&mut bytes)?;
        (bytes, 0)
    };
    if u64::try_from(bytes.len() - offset)? != artifact.size_bytes
        || format!("{:x}", Sha256::digest(&bytes[offset..])) != artifact.sha256
    {
        return Err(invalid("artifact checksum mismatch").into());
    }
    Ok((bytes, offset))
}

fn memory(stage: &str) -> Result<()> {
    let status = fs::read_to_string("/proc/self/status")?;
    let values = status.lines().filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmHWM:")).collect::<Vec<_>>().join(" ");
    println!("memory_stage={stage} {values}");
    Ok(())
}

fn load(root: &Path) -> Result<(XLMRobertaForSequenceClassification, Tokenizer)> {
    let manifest: Manifest = serde_json::from_str(include_str!("nli-model-manifest.json"))?;
    let mut weights = None;
    let mut configuration = None;
    let mut tokenizer = None;
    for artifact in manifest.files {
        let (bytes, offset) = verified(root, &artifact)?;
        match artifact.name.as_str() {
            "model.safetensors" => weights = Some((bytes, offset)),
            "tokenizer.json" => tokenizer = Some(bytes),
            "config.json" => {
                configuration = Some(serde_json::from_slice::<serde_json::Value>(&bytes)?);
            }
            _ => (),
        }
    }
    let configuration = configuration.ok_or_else(|| invalid("missing configuration"))?;
    if configuration["id2label"]
        != serde_json::json!({"0":"entailment","1":"neutral","2":"contradiction"})
    {
        return Err(invalid("unexpected label order").into());
    }
    let config: Config = serde_json::from_value(configuration)?;
    if config.hidden_size != 384
        || config.num_hidden_layers != 6
        || config.pad_token_id != 1
        || config.max_position_embeddings != 514
    {
        return Err(invalid("unexpected classifier configuration").into());
    }
    memory("verified_files")?;
    let (bytes, offset) = weights.ok_or_else(|| invalid("missing weights"))?;
    let builder = VarBuilder::from_slice_safetensors(&bytes[offset..], DType::F32, &Device::Cpu)?;
    memory("builder_ready")?;
    let model = XLMRobertaForSequenceClassification::new(3, &config, builder)?;
    drop(bytes);
    memory("model_loaded")?;
    let tokenizer = Tokenizer::from_bytes(tokenizer.ok_or_else(|| invalid("missing tokenizer"))?)?;
    memory("tokenizer_loaded")?;
    Ok((model, tokenizer))
}

fn infer(
    model: &XLMRobertaForSequenceClassification,
    tokenizer: &mut Tokenizer,
    pair: Pair,
) -> Result<Observation> {
    if pair.premise.len() > 32768
        || pair.hypothesis.len() > 4096
        || pair
            .pad_to
            .is_some_and(|length| !(1..=512).contains(&length))
    {
        return Err(invalid("oversized synthetic pair").into());
    }
    tokenizer.with_truncation(Some(TruncationParams {
        max_length: 512,
        strategy: TruncationStrategy::OnlyFirst,
        ..Default::default()
    }))?;
    tokenizer.with_padding(pair.pad_to.map(|length| PaddingParams {
        strategy: PaddingStrategy::Fixed(length),
        pad_id: 1,
        pad_token: "<pad>".into(),
        ..Default::default()
    }));
    let encoded = tokenizer.encode((pair.premise.as_str(), pair.hypothesis.as_str()), true)?;
    if encoded.len() > 512 || encoded.get_type_ids().iter().any(|id| *id != 0) {
        return Err(invalid("unexpected pair encoding").into());
    }
    let input = Tensor::new(encoded.get_ids(), &Device::Cpu)?.unsqueeze(0)?;
    let mask = Tensor::new(encoded.get_attention_mask(), &Device::Cpu)?.unsqueeze(0)?;
    let types = Tensor::zeros((1, encoded.len()), DType::U32, &Device::Cpu)?;
    let start = Instant::now();
    let logits = model
        .forward(&input, &mask, &types)?
        .squeeze(0)?
        .to_vec1::<f32>()?;
    if logits.len() != 3 || !logits.iter().all(|value| value.is_finite()) {
        return Err(invalid("invalid classifier output").into());
    }
    Ok(Observation {
        id: pair.id,
        input_ids: encoded.get_ids().to_vec(),
        attention_mask: encoded.get_attention_mask().to_vec(),
        logits_bits: logits.into_iter().map(f32::to_bits).collect(),
        truncated: !encoded.get_overflowing().is_empty(),
        micros: start.elapsed().as_micros().try_into()?,
    })
}

fn main() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err(invalid("expected model directory and observation output").into());
    }
    let start = Instant::now();
    let (model, mut tokenizer) = load(Path::new(&args[0]))?;
    println!(
        "verified_load_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.
    );
    let pairs: Vec<Pair> = serde_json::from_str(include_str!("nli-pairs.json"))?;
    let mut results = Vec::new();
    for pair in pairs {
        let result = infer(&model, &mut tokenizer, pair)?;
        println!(
            "pair={} tokens={} truncated={} inference_us={}",
            result.id,
            result.input_ids.len(),
            result.truncated,
            result.micros
        );
        results.push(result);
    }
    memory("after_inference")?;
    fs::write(&args[1], serde_json::to_vec(&results)?)?;
    Ok(())
}
