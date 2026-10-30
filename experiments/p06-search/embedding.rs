//! Bounded local multilingual retrieval experiment, using existing Candle and `LanceDB` APIs.
#![forbid(unsafe_code)]
use arrow_array::{
    Array, FixedSizeListArray, RecordBatch, StringArray, UInt64Array, types::Float32Type,
};
use arrow_schema::{DataType, Field, Schema};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use futures_util::TryStreamExt as _;
use lancedb::{
    Table,
    expr::{col, lit},
    index::{
        Index,
        scalar::{FtsIndexBuilder, FullTextSearchQuery},
    },
    query::{ExecutableQuery as _, QueryBase as _, QueryExecutionOptions},
    rerankers::rrf::RRFReranker,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    env,
    error::Error,
    fs,
    io::{self, Read as _},
    path::Path,
    sync::Arc,
    time::Instant,
};
use tokenizers::{Tokenizer, TruncationParams};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const DIMENSIONS: usize = 384;

#[derive(Deserialize)]
struct Manifest {
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
struct Fixtures {
    documents: Vec<Document>,
    queries: Vec<Query>,
}
#[derive(Deserialize)]
struct Document {
    id: u64,
    account: String,
    folder: String,
    text: String,
}
#[derive(Deserialize)]
struct Query {
    id: String,
    text: String,
    expected: u64,
}
#[derive(Serialize)]
struct Output {
    documents: Vec<Vec<f32>>,
    queries: Vec<Vec<f32>>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::other(message)
}

fn verify_file(root: &Path, artifact: &Artifact) -> Result<Vec<u8>> {
    if ![
        "model.safetensors",
        "tokenizer.json",
        "config.json",
        "tokenizer_config.json",
        "special_tokens_map.json",
    ]
    .contains(&artifact.name.as_str())
        || artifact.size_bytes > 512 * 1024 * 1024
    {
        return Err(invalid("unsupported artifact").into());
    }
    let path = root.join(&artifact.name);
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err(invalid("artifact must be a regular file").into());
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
        let manifest: Manifest = serde_json::from_str(include_str!("model-manifest.json"))?;
        let mut weights = None;
        let mut config = None;
        let mut tokenizer = None;
        for artifact in &manifest.files {
            let bytes = verify_file(root, artifact)?;
            match artifact.name.as_str() {
                "model.safetensors" => weights = Some(bytes),
                "config.json" => config = Some(serde_json::from_slice::<Config>(&bytes)?),
                "tokenizer.json" => tokenizer = Some(Tokenizer::from_bytes(bytes)?),
                _ => (),
            }
        }
        let config = config.ok_or_else(|| invalid("missing config"))?;
        if config.hidden_size != DIMENSIONS || config.max_position_embeddings != 512 {
            return Err(invalid("unexpected model dimensions").into());
        }
        let builder = VarBuilder::from_buffered_safetensors(
            weights.ok_or_else(|| invalid("missing weights"))?,
            DType::F32,
            &Device::Cpu,
        )?;
        let model = BertModel::load(builder, &config)?;
        let mut tokenizer = tokenizer.ok_or_else(|| invalid("missing tokenizer"))?;
        tokenizer.with_truncation(Some(TruncationParams {
            max_length: 512,
            ..Default::default()
        }))?;
        tokenizer.with_padding(None);
        Ok(Self { model, tokenizer })
    }

    fn encode(&self, prefix: &str, text: &str) -> Result<Vec<f32>> {
        if text.trim().is_empty() || text.len() > 64 * 1024 {
            return Err(invalid("bounded nonempty text required").into());
        }
        let encoding = self.tokenizer.encode(format!("{prefix}{text}"), true)?;
        let input = Tensor::new(encoding.get_ids(), &Device::Cpu)?.unsqueeze(0)?;
        let types = Tensor::new(encoding.get_type_ids(), &Device::Cpu)?.unsqueeze(0)?;
        let mask = Tensor::new(encoding.get_attention_mask(), &Device::Cpu)?.unsqueeze(0)?;
        let hidden = self.model.forward(&input, &types, Some(&mask))?;
        let mask = mask.to_dtype(DType::F32)?.unsqueeze(2)?;
        let pooled = hidden
            .broadcast_mul(&mask)?
            .sum(1)?
            .broadcast_div(&mask.sum(1)?)?;
        let normalized = pooled.broadcast_div(&pooled.sqr()?.sum_keepdim(1)?.sqrt()?)?;
        let vector = normalized.squeeze(0)?.to_vec1::<f32>()?;
        if vector.len() != DIMENSIONS || !vector.iter().all(|value| value.is_finite()) {
            return Err(invalid("invalid embedding").into());
        }
        let norm = vector.iter().map(|value| value * value).sum::<f32>();
        assert!((norm - 1.0).abs() < 1e-4, "embedding must have unit norm");
        Ok(vector)
    }
}

fn batch(documents: &[Document], vectors: &[Vec<f32>]) -> Result<RecordBatch> {
    let vector = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        vectors
            .iter()
            .map(|row| Some(row.iter().copied().map(Some))),
        i32::try_from(DIMENSIONS)?,
    );
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::UInt64, false),
        Field::new("account", DataType::Utf8, false),
        Field::new("folder", DataType::Utf8, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("vector", vector.data_type().clone(), true),
    ]));
    Ok(RecordBatch::try_new(
        schema,
        vec![
            Arc::new(UInt64Array::from_iter_values(
                documents.iter().map(|item| item.id),
            )),
            Arc::new(StringArray::from_iter_values(
                documents.iter().map(|item| &item.account),
            )),
            Arc::new(StringArray::from_iter_values(
                documents.iter().map(|item| &item.folder),
            )),
            Arc::new(StringArray::from_iter_values(
                documents.iter().map(|item| &item.text),
            )),
            Arc::new(vector),
        ],
    )?)
}

fn ids(batches: &[RecordBatch]) -> Result<Vec<u64>> {
    let mut ids = Vec::new();
    for batch in batches {
        let values = batch
            .column_by_name("id")
            .and_then(|array| array.as_any().downcast_ref::<UInt64Array>())
            .ok_or_else(|| invalid("missing typed IDs"))?;
        ids.extend(values.values().iter().copied());
    }
    Ok(ids)
}

async fn retrieve(table: &Table, queries: &[Query], vectors: &[Vec<f32>]) -> Result<()> {
    let scope = col("account")
        .eq(lit("a"))
        .and(col("folder").eq(lit("INBOX")));
    let mut top1 = 0_u32;
    let mut top3 = 0_u32;
    for (query, vector) in queries.iter().zip(vectors) {
        let semantic = table
            .query()
            .only_if_expr(scope.clone())
            .nearest_to(vector.clone())?
            .limit(3)
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        let semantic = ids(&semantic)?;
        let hybrid = table
            .query()
            .only_if_expr(scope.clone())
            .full_text_search(FullTextSearchQuery::new_fuzzy(query.text.clone(), Some(1)))
            .nearest_to(vector.clone())?
            .rerank(Arc::new(RRFReranker::default()))
            .limit(3)
            .execute_hybrid(QueryExecutionOptions::default())
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        let hybrid = ids(&hybrid)?;
        assert!(
            semantic.iter().chain(&hybrid).all(|id| *id <= 10),
            "account/folder prefilter failed"
        );
        top1 += u32::from(semantic.first() == Some(&query.expected));
        top3 += u32::from(semantic.contains(&query.expected));
        println!(
            "query={} expected={} semantic={semantic:?} hybrid={hybrid:?}",
            query.id, query.expected
        );
        if query.id == "user-example" {
            assert_eq!(
                semantic.first(),
                Some(&query.expected),
                "user's cross-language example must rank first without lexical ranking"
            );
        }
    }
    println!(
        "semantic_top1={top1}/{} semantic_top3={top3}/{}",
        queries.len(),
        queries.len()
    );
    assert!(
        top1 >= 8 && usize::try_from(top3)? == queries.len(),
        "fixed multilingual relevance screen failed"
    );
    Ok(())
}

fn local_path(path: &Path) -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .ok_or_else(|| invalid("missing probe root"))?
        .canonicalize()?;
    let parent = path
        .parent()
        .ok_or_else(|| invalid("missing parent"))?
        .canonicalize()?;
    if !parent.starts_with(root) {
        return Err(invalid("project-local probe paths required").into());
    }
    Ok(())
}

fn measure_queries(encoder: &Encoder, query: &str) -> Result<()> {
    let first = Instant::now();
    encoder.encode("query: ", query)?;
    let first_ms = first.elapsed().as_secs_f64() * 1000.0;
    let mut latencies = Vec::with_capacity(30);
    for _ in 0..30 {
        let started = Instant::now();
        encoder.encode("query: ", query)?;
        latencies.push(started.elapsed());
    }
    latencies.sort_unstable();
    let p95 = latencies
        .get(28)
        .ok_or_else(|| invalid("missing latency sample"))?;
    println!(
        "query_first_measured_ms={first_ms:.3} warm_query_p95_ms={:.3}",
        p95.as_secs_f64() * 1000.0
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    let [model, data, output] = arguments.as_slice() else {
        return Err(invalid("usage: embedding-probe MODEL_DIR NEW_DATA_PATH OUTPUT_JSON").into());
    };
    for path in [model, data, output] {
        local_path(Path::new(path))?;
    }
    if Path::new(data).exists() || Path::new(output).exists() {
        return Err(invalid("fresh output paths required").into());
    }
    let fixtures: Fixtures = serde_json::from_str(include_str!("relevance.json"))?;
    let load = Instant::now();
    let encoder = Encoder::load(Path::new(model))?;
    println!(
        "verified_model_load_ms={:.3}",
        load.elapsed().as_secs_f64() * 1000.0
    );
    let first = fixtures
        .queries
        .first()
        .ok_or_else(|| invalid("missing user query"))?;
    measure_queries(&encoder, &first.text)?;
    let documents = fixtures
        .documents
        .iter()
        .map(|item| encoder.encode("passage: ", &item.text))
        .collect::<Result<Vec<_>>>()?;
    let queries = fixtures
        .queries
        .iter()
        .map(|item| encoder.encode("query: ", &item.text))
        .collect::<Result<Vec<_>>>()?;
    let connection = lancedb::connect(data).execute().await?;
    let table = connection
        .create_table("mail", batch(&fixtures.documents, &documents)?)
        .execute()
        .await?;
    table
        .create_index(
            &["text"],
            Index::FTS(
                FtsIndexBuilder::default()
                    .stem(false)
                    .remove_stop_words(false)
                    .ascii_folding(true),
            ),
        )
        .execute()
        .await?;
    retrieve(&table, &fixtures.queries, &queries).await?;
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    serde_json::to_writer(file, &Output { documents, queries })?;
    Ok(())
}
