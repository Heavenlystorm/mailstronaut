//! Export actual model observations alongside the unchanged rule/date projection.
use super::{Observation, Pair, Result, infer, invalid, memory, rules};
use candle_transformers::models::xlm_roberta::XLMRobertaForSequenceClassification;
use chrono::DateTime;
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{fs, path::Path, time::Instant};
use tokenizers::Tokenizer;

const HYPOTHESES: [&str; 4] = [
    "This email asks the recipient to take a concrete action or reply.",
    "This email contains a time-sensitive obligation for the recipient.",
    "This email is an informational newsletter or advertisement without a personal obligation.",
    "This email requires attention within the next day.",
];

#[derive(Deserialize)]
struct Corpus {
    synthetic: bool,
    #[serde(rename = "anchorZone")]
    anchor_zone: String,
    messages: Vec<Mail>,
}
#[derive(Deserialize)]
struct Mail {
    id: String,
    group: String,
    split: String,
    lang: String,
    subject: String,
    body: String,
    direct: bool,
    authored: String,
    received: String,
    expected_priority: bool,
    expected_deadline: Option<String>,
}
#[derive(Serialize)]
struct Row {
    id: String,
    group: String,
    lang: String,
    expected_priority: bool,
    expected_deadline: Option<String>,
    as_of: i64,
    rules: rules::Analysis,
    observations: Vec<Observation>,
    micros: u64,
}
#[derive(Serialize)]
struct Report {
    split: String,
    hypotheses: [&'static str; 4],
    projection_policy: &'static str,
    rules_policy: u32,
    rules_source_sha256: String,
    model_manifest_sha256: String,
    rows: Vec<Row>,
}

fn corpus(text: &str, expected: &str) -> Result<Corpus> {
    if format!("{:x}", Sha256::digest(text.as_bytes())) != expected {
        return Err(invalid("quality corpus changed after freezing").into());
    }
    let result: Corpus = serde_json::from_str(text)?;
    if !result.synthetic {
        return Err(invalid("expected synthetic corpus").into());
    }
    Ok(result)
}

pub fn run(
    model: &XLMRobertaForSequenceClassification,
    tokenizer: &mut Tokenizer,
    split: &str,
    output: &Path,
) -> Result<()> {
    if !["development", "heldout"].contains(&split) {
        return Err(invalid("unknown split").into());
    }
    let corpora = [
        corpus(
            include_str!("priority-corpus.json"),
            "a2ff10ab4315b03c8a185301baff00f90f8f15ae4038c308ead8b59e268306ca",
        )?,
        corpus(
            include_str!("semantic-challenges.json"),
            "8cf9d92868dd23c17574d8caddbda0f064151e33c9539f815d73d528222eaab4",
        )?,
    ];
    let mut rows = Vec::new();
    for corpus in corpora {
        let zone: Tz = corpus.anchor_zone.parse()?;
        // Filter before projection, inference or logging; development never encodes held-out text.
        for mail in corpus
            .messages
            .into_iter()
            .filter(|mail| mail.split == split)
        {
            let start = Instant::now();
            let received = DateTime::parse_from_rfc3339(&mail.received)?.timestamp();
            let authored = DateTime::parse_from_rfc3339(&mail.authored)?.timestamp();
            let input = rules::Input {
                subject: &mail.subject,
                body: Some(&mail.body),
                direct: mail.direct,
                authored: Some(authored),
                received_at: received,
                zone,
                max_ttl_hours: 168,
            };
            let baseline = rules::analyze(&input);
            let authored_text = rules::deadline::authored(&mail.body)
                .into_iter()
                .map(|range| &mail.body[range])
                .collect::<Vec<_>>()
                .join("\n");
            // Synthetic sender/group IDs must not leak the expected label into the model.
            let premise = format!("{}\n\n{authored_text}", mail.subject);
            let mut observations = Vec::new();
            for (number, hypothesis) in HYPOTHESES.iter().enumerate() {
                observations.push(infer(
                    model,
                    tokenizer,
                    Pair {
                        id: format!("{}:{number}", mail.id),
                        premise: premise.clone(),
                        hypothesis: (*hypothesis).into(),
                        pad_to: None,
                    },
                )?);
            }
            rows.push(Row {
                id: mail.id,
                group: mail.group,
                lang: mail.lang,
                expected_priority: mail.expected_priority,
                expected_deadline: mail.expected_deadline,
                as_of: received,
                rules: baseline,
                observations,
                micros: start.elapsed().as_micros().try_into()?,
            });
        }
    }
    let expected = if split == "development" { 108 } else { 90 };
    if rows.len() != expected {
        return Err(invalid("unexpected split coverage").into());
    }
    memory("quality_complete")?;
    println!(
        "quality_observations split={split} messages={} (no acceptance claim)",
        rows.len()
    );
    let report = Report {
        split: split.into(),
        hypotheses: HYPOTHESES,
        projection_policy: "authored-subject-first512-pair-v1",
        rules_policy: rules::POLICY_VERSION,
        rules_source_sha256: format!(
            "{:x}",
            Sha256::new()
                .chain_update(include_bytes!("rules/mod.rs"))
                .chain_update(include_bytes!("rules/deadline.rs"))
                .finalize()
        ),
        model_manifest_sha256: format!(
            "{:x}",
            Sha256::digest(include_bytes!("nli-model-manifest.json"))
        ),
        rows,
    };
    fs::write(output, serde_json::to_vec(&report)?)?;
    Ok(())
}
