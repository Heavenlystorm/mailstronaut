//! Pinned upstream component corpus; application policy is tested separately.

use std::{fs, path::PathBuf};

use mail_threading::{Message, ThreadingOptions, thread_messages_with};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    input: Vec<Input>,
    expected: Expected,
    #[serde(default)]
    options: Options,
}
#[derive(Deserialize)]
struct Input {
    id: String,
    message_id: Option<String>,
    in_reply_to: Option<String>,
    references: Vec<String>,
    subject: String,
    date: String,
}
#[derive(Deserialize)]
struct Expected {
    threads: Vec<ExpectedThread>,
}
#[derive(Deserialize)]
struct ExpectedThread {
    root: String,
    messages: Vec<String>,
}
#[derive(Default, Deserialize)]
struct Options {
    subject_merge: Option<bool>,
    prune_phantoms: Option<bool>,
    subject_prefixes: Option<Vec<String>>,
}

#[test]
fn pinned_upstream_rfc_corpus_matches_its_declared_component_options() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mail-threading-0.1.3/conformance");
    let mut paths = fs::read_dir(root)
        .expect("vendored synthetic corpus")
        .map(|entry| entry.expect("fixture entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    assert!(
        !paths.is_empty(),
        "the conformance corpus must actually run"
    );
    for path in paths {
        let case: Case = serde_json::from_slice(&fs::read(path).expect("fixture bytes"))
            .expect("upstream case schema");
        let mut options = ThreadingOptions::default();
        if let Some(value) = case.options.subject_merge {
            options.subject_merge = value;
        }
        if let Some(value) = case.options.prune_phantoms {
            options.prune_phantoms = value;
        }
        if let Some(value) = case.options.subject_prefixes {
            options.subject_prefixes = value;
        }
        let input = case
            .input
            .into_iter()
            .map(|message| Message {
                id: message.id,
                message_id: message.message_id,
                in_reply_to: message.in_reply_to,
                references: message.references,
                subject: message.subject,
                date: chrono::DateTime::parse_from_rfc3339(&message.date)
                    .expect("fixture date")
                    .with_timezone(&chrono::Utc),
            })
            .collect::<Vec<_>>();
        let actual = thread_messages_with(&input, &options)
            .into_iter()
            .map(|thread| (thread.root_message_id, thread.messages))
            .collect::<Vec<_>>();
        let expected = case
            .expected
            .threads
            .into_iter()
            .map(|thread| (thread.root, thread.messages))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "upstream case {}", case.name);
    }
}
