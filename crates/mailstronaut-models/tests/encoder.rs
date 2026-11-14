//! Explicit real-weight acceptance; routine tests never fetch a model.
use mailstronaut_models::{DIMENSIONS, Encoder, Error, MAX_WINDOWS};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

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
    documents: Vec<Vec<u32>>,
    queries: Vec<Vec<u32>>,
}

#[test]
#[ignore = "Requires explicitly installed pinned MiniLM files and private output directory"]
fn real_encoder_exports_frozen_reference_cases_and_reports_window_coverage() {
    let directory = PathBuf::from(
        std::env::var_os("MAILSTRONAUT_TEST_MODEL_DIR").expect("explicit model directory"),
    );
    let output = PathBuf::from(
        std::env::var_os("MAILSTRONAUT_TEST_MODEL_OUTPUT").expect("private output directory"),
    );
    fs::create_dir_all(&output).expect("private output");
    let encoder = Encoder::load(&directory).expect("verified encoder");
    for (name, text) in [
        ("fixed", include_str!("fixtures/relevance.json")),
        ("heldout", include_str!("fixtures/relevance-heldout.json")),
    ] {
        let fixtures: Fixtures = serde_json::from_str(text).expect("frozen fixtures");
        let documents = fixtures
            .documents
            .iter()
            .map(|row| {
                let projection = encoder.document(&row.text).expect("document");
                assert!(!projection.truncated);
                assert_eq!(projection.vectors.len(), 1);
                projection.vectors[0]
                    .iter()
                    .map(|value| value.to_bits())
                    .collect()
            })
            .collect();
        let queries = fixtures
            .queries
            .iter()
            .map(|row| {
                encoder
                    .query(&row.text)
                    .expect("query")
                    .into_iter()
                    .map(f32::to_bits)
                    .collect()
            })
            .collect();
        fs::write(
            output.join(format!("{name}.json")),
            serde_json::to_vec(&Output { documents, queries }).expect("vectors"),
        )
        .expect("export");
    }
    assert!(matches!(
        encoder.document_cancellable("A retired document", || true),
        Err(Error::Cancelled)
    ));
    assert!(matches!(encoder.query(""), Err(Error::Input)));
    assert!(matches!(encoder.query("line\ncontrol"), Err(Error::Input)));
    assert!(matches!(
        encoder.query(&"word ".repeat(150)),
        Err(Error::Input)
    ));
    let projection = encoder
        .document(&"Avis important. Réinscription au doctorat. ".repeat(1500))
        .expect("long Unicode document");
    assert_eq!(projection.vectors.len(), MAX_WINDOWS);
    assert!(projection.truncated);
    assert!(
        projection
            .vectors
            .iter()
            .all(|vector| vector.len() == DIMENSIONS)
    );
    let projection = encoder
        .document(&"Ordinary bounded multilingual message. ".repeat(30))
        .expect("overlap");
    assert!(projection.vectors.len() > 1);
    assert!(!projection.truncated);
}
