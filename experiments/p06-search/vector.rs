//! Existing vector-index API probe using actual, independently checked model vectors.
#![forbid(unsafe_code)]

use rand::{Rng as _, SeedableRng as _, rngs::StdRng};
use rayon::prelude::*;
use serde::{Deserialize, de::DeserializeOwned};
use std::time::Instant;
use std::{collections::HashSet, env, error::Error, fs, io, io::Read as _, path::Path};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Deserialize)]
struct Vectors {
    documents: Vec<Vec<u32>>,
    queries: Vec<Vec<u32>>,
}
#[derive(Deserialize)]
struct Document {
    id: u64,
    account: String,
    folder: String,
}
#[derive(Deserialize)]
struct Query {
    id: String,
    expected: u64,
}
#[derive(Deserialize)]
struct Fixtures {
    documents: Vec<Document>,
    queries: Vec<Query>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::other(message)
}

fn read<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(invalid("oversized fixture").into());
    }
    Ok(serde_json::from_slice(&bytes)?)
}

fn floats(bits: &[Vec<u32>]) -> Result<Vec<Vec<f32>>> {
    bits.iter()
        .map(|row| {
            let vector: Vec<_> = row.iter().copied().map(f32::from_bits).collect();
            if vector.len() != 384 || !vector.iter().all(|value| value.is_finite()) {
                return Err(invalid("invalid vector").into());
            }
            Ok(vector)
        })
        .collect()
}

const fn options() -> IndexOptions {
    IndexOptions {
        dimensions: 384,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F16,
        connectivity: 32,
        expansion_add: 128,
        expansion_search: 128,
        multi: false,
    }
}

fn ranks(
    index: &Index,
    fixture: &Fixtures,
    queries: &[Vec<f32>],
    allowed: &HashSet<u64>,
) -> Result<()> {
    let mut first = 0;
    let mut three = 0;
    for (query, vector) in fixture.queries.iter().zip(queries) {
        let hits = index.filtered_search(vector, 3, |key| allowed.contains(&key))?;
        if hits.keys.iter().any(|key| !allowed.contains(key))
            || hits.distances.iter().any(|distance| !distance.is_finite())
        {
            return Err(invalid("invalid or foreign result").into());
        }
        let rank = hits
            .keys
            .iter()
            .position(|key| *key == query.expected)
            .map(|rank| rank + 1);
        first += usize::from(rank == Some(1));
        three += usize::from(rank.is_some());
        println!("{} rank={rank:?} top3={:?}", query.id, hits.keys);
    }
    let minimum = match fixture.queries.len() {
        10 => (8, 10),
        24 => (20, 23),
        _ => return Err(invalid("unknown frozen screen").into()),
    };
    println!(
        "F16 index: first={first} top3={three} total={}",
        fixture.queries.len()
    );
    if first < minimum.0 || three < minimum.1 {
        return Err(invalid("index relevance screen failed").into());
    }
    Ok(())
}

fn lifecycle(
    index: &Index,
    fixture: &Fixtures,
    vectors: &[Vec<f32>],
    query: &[f32],
    allowed: &HashSet<u64>,
    directory: &Path,
) -> Result<()> {
    let target = fixture.documents[0].id;
    if index.remove(target)? != 1 || index.contains(target) {
        return Err(invalid("removal failed").into());
    }
    if index
        .filtered_search(query, 3, |key| allowed.contains(&key))?
        .keys
        .contains(&target)
    {
        return Err(invalid("deleted key returned").into());
    }
    index.add(target, &vectors[0])?;
    index.compact()?;
    let expected = index
        .filtered_search(query, 3, |key| allowed.contains(&key))?
        .keys;
    fs::create_dir_all(directory)?;
    let file = directory.join("vectors.usearch");
    let file = file
        .to_str()
        .ok_or_else(|| invalid("non-UTF8 fixture path"))?;
    index.save(file)?;
    let restored = Index::new(&options())?;
    restored.load(file)?;
    if restored.size() != fixture.documents.len()
        || restored
            .filtered_search(query, 3, |key| allowed.contains(&key))?
            .keys
            != expected
    {
        return Err(invalid("persistence changed results").into());
    }
    if !restored
        .filtered_search(query, 3, |_| false)?
        .keys
        .is_empty()
    {
        return Err(invalid("empty scope returned results").into());
    }
    println!("remove/reinsert/compact/save/reopen and empty scope passed");
    Ok(())
}

fn random_vector(seed: u64) -> Vec<f32> {
    let mut random = StdRng::seed_from_u64(seed);
    let mut vector: Vec<f32> = (0..384)
        .map(|_| random.random::<f32>().mul_add(2., -1.))
        .collect();
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    for value in &mut vector {
        *value /= norm;
    }
    vector
}

fn benchmark(count: usize, directory: &Path) -> Result<()> {
    if ![10_000, 100_000].contains(&count) {
        return Err(invalid("expected 10k or 100k synthetic vectors").into());
    }
    let index = Index::new(&options())?;
    index.reserve_capacity_and_threads(count, rayon::current_num_threads() + 1)?;
    let start = Instant::now();
    (0..count)
        .into_par_iter()
        .try_for_each(|id| -> Result<()> {
            let key = u64::try_from(id)?;
            index.add(key, &random_vector(key + 128))?;
            Ok(())
        })?;
    println!(
        "rows={count} dimensions=384 scalar=F16 build_seconds={:.3}",
        start.elapsed().as_secs_f64()
    );
    println!("library_memory_bytes={}", index.memory_usage());
    let queries: Vec<_> = (0..30).map(|id| random_vector(100_000_000 + id)).collect();
    let mut truth = Vec::new();
    let mut exact_times = Vec::new();
    for query in &queries {
        let start = Instant::now();
        // The library supplies exact neighbors. With this fixed 1/3 scope, 128
        // global exact hits must contain ten scoped hits, or the oracle refuses.
        let hits = index.exact_search(query, 128)?;
        exact_times.push(start.elapsed().as_secs_f64() * 1000.);
        let keys: Vec<_> = hits
            .keys
            .into_iter()
            .filter(|key| key % 3 == 0)
            .take(10)
            .collect();
        if keys.len() != 10 {
            return Err(invalid("insufficient exact scoped oracle hits").into());
        }
        truth.push(keys);
    }
    exact_times.sort_by(f64::total_cmp);
    println!("exact_top128_p95_ms={:.3}", exact_times[28]);
    let mut passed = false;
    // Fixed before measurement: select the least expansion passing >=95% recall
    // against the library's exact search and <=500ms warm query p95.
    for expansion in [128, 512, 2048, 8192] {
        index.change_expansion_search(expansion);
        for query in queries.iter().take(3) {
            std::hint::black_box(index.filtered_search(query, 10, |key| key % 3 == 0)?);
        }
        let mut correct = 0;
        let mut times = Vec::new();
        for (query, expected) in queries.iter().zip(&truth) {
            let start = Instant::now();
            let hits = index.filtered_search(query, 10, |key| key % 3 == 0)?;
            times.push(start.elapsed().as_secs_f64() * 1000.);
            if hits.keys.iter().any(|key| key % 3 != 0) {
                return Err(invalid("foreign benchmark result").into());
            }
            correct += hits
                .keys
                .iter()
                .filter(|key| expected.contains(key))
                .count();
        }
        times.sort_by(f64::total_cmp);
        println!(
            "expansion={expansion} recall_hits={correct}/300 filtered_p95_ms={:.3}",
            times[28]
        );
        if correct >= 285 && times[28] <= 500. {
            passed = true;
            break;
        }
    }
    fs::create_dir_all(directory)?;
    let file = directory.join("vectors.usearch");
    index.save(
        file.to_str()
            .ok_or_else(|| invalid("non-UTF8 benchmark path"))?,
    )?;
    let bytes = fs::metadata(file)?.len();
    println!("index_bytes={bytes}");
    if !passed || bytes > 1024 * 1024 * 1024 {
        return Err(invalid("vector benchmark screen failed").into());
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err(invalid("expected vectors, fixtures and private output directory").into());
    }
    if args[0] == "--bench" {
        let count = args[1]
            .to_str()
            .ok_or_else(|| invalid("invalid count"))?
            .parse()?;
        return benchmark(count, Path::new(&args[2]));
    }
    let vectors: Vectors = read(Path::new(&args[0]))?;
    let fixture: Fixtures = read(Path::new(&args[1]))?;
    if vectors.documents.len() != fixture.documents.len()
        || vectors.queries.len() != fixture.queries.len()
        || fixture.documents.is_empty()
        || fixture.queries.is_empty()
    {
        return Err(invalid("fixture/vector mismatch").into());
    }
    let documents = floats(&vectors.documents)?;
    let queries = floats(&vectors.queries)?;
    let index = Index::new(&options())?;
    index.reserve(fixture.documents.len() + 1)?;
    let mut allowed = HashSet::new();
    for (document, vector) in fixture.documents.iter().zip(&documents) {
        index.add(document.id, vector)?;
        if document.account == "a" && document.folder == "INBOX" {
            allowed.insert(document.id);
        }
    }
    ranks(&index, &fixture, &queries, &allowed)?;
    lifecycle(
        &index,
        &fixture,
        &documents,
        &queries[0],
        &allowed,
        Path::new(&args[2]),
    )?;
    Ok(())
}
