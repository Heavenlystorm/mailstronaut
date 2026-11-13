//! Explicit integration with vectors exported by the real production encoder.
use mailstronaut_models::POLICY;
use mailstronaut_search::semantic::{Embedding, Index};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::PathBuf};

#[derive(Deserialize)]
struct Document {
    id: i64,
    account: String,
    folder: String,
}
#[derive(Deserialize)]
struct Query {
    expected: i64,
}
#[derive(Deserialize)]
struct Fixtures {
    documents: Vec<Document>,
    queries: Vec<Query>,
}
#[derive(Deserialize)]
struct Vectors {
    documents: Vec<Vec<u32>>,
    queries: Vec<Vec<u32>>,
}

#[test]
#[ignore = "Requires explicitly exported, independently checked production model vectors"]
fn real_model_vectors_preserve_frozen_relevance_through_private_index_and_restart() {
    let output = PathBuf::from(
        std::env::var_os("MAILSTRONAUT_TEST_MODEL_OUTPUT").expect("explicit model output"),
    );
    for (name, text, minimum) in [
        (
            "fixed",
            include_str!("../../mailstronaut-models/tests/fixtures/relevance.json"),
            (8, 10),
        ),
        (
            "heldout",
            include_str!("../../mailstronaut-models/tests/fixtures/relevance-heldout.json"),
            (20, 23),
        ),
    ] {
        let fixtures: Fixtures = serde_json::from_str(text).expect("frozen corpus");
        let vectors: Vectors = serde_json::from_slice(
            &fs::read(output.join(format!("{name}.json"))).expect("real vectors"),
        )
        .expect("vector encoding");
        assert_eq!(vectors.documents.len(), fixtures.documents.len());
        assert_eq!(vectors.queries.len(), fixtures.queries.len());
        let directory = tempfile::tempdir().expect("private vector cache");
        let mut index = Index::open(directory.path(), POLICY).expect("index");
        let source = "a".repeat(64);
        let mut allowed = BTreeMap::new();
        for (document, vector) in fixtures.documents.iter().zip(vectors.documents) {
            index
                .put(Embedding {
                    id: document.id,
                    source: source.clone(),
                    vectors: vec![vector.into_iter().map(f32::from_bits).collect()],
                    truncated: false,
                })
                .expect("transactional embedding");
            if document.account == "a" && document.folder == "INBOX" {
                allowed.insert(document.id, source.clone());
            }
        }
        let mut first = 0;
        let mut three = 0;
        let mut before = Vec::new();
        for (query, vector) in fixtures.queries.iter().zip(&vectors.queries) {
            let vector: Vec<_> = vector.iter().copied().map(f32::from_bits).collect();
            let hits = index
                .search(&vector, &allowed, 3)
                .expect("scoped semantic retrieval");
            let rank = hits.iter().position(|id| *id == query.expected);
            first += usize::from(rank == Some(0));
            three += usize::from(rank.is_some());
            before.push(hits);
        }
        println!(
            "{name}: first={first}/{} top3={three}/{}",
            fixtures.queries.len(),
            fixtures.queries.len()
        );
        assert!(first >= minimum.0 && three >= minimum.1);
        index.checkpoint().expect("checkpoint");
        drop(index);
        let index = Index::open(directory.path(), POLICY).expect("restart");
        for (vector, expected) in vectors.queries.iter().zip(before) {
            let vector: Vec<_> = vector.iter().copied().map(f32::from_bits).collect();
            assert_eq!(
                index.search(&vector, &allowed, 3).expect("reopened query"),
                expected
            );
        }
    }
}
