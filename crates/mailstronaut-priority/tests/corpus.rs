//! The rules against labelled synthetic messages. These were written alongside the rules,
//! so this guards against regressions and is not a held-out quality measurement.

use chrono::{NaiveDate, TimeZone as _};
use chrono_tz::Europe::Paris;
use mailstronaut_priority::{Input, analyze};
use serde::Deserialize;

#[derive(Deserialize)]
struct Labelled {
    lang: String,
    subject: String,
    body: String,
    direct: bool,
    priority: bool,
    deadline: Option<NaiveDate>,
}

#[test]
fn labelled_messages_meet_the_release_targets_in_every_language() {
    let corpus: Vec<Labelled> =
        serde_json::from_str(include_str!("fixtures/corpus.json")).expect("corpus");
    // Wednesday, 4 November 2026.
    let written = Paris
        .with_ymd_and_hms(2026, 11, 4, 9, 0, 0)
        .unwrap()
        .timestamp();
    for language in ["en", "fr", "it"] {
        let (mut correct, mut predicted, mut actual) = (0_u32, 0_u32, 0_u32);
        let (mut deadlines_right, mut deadlines_found) = (0_u32, 0_u32);
        let mut wrong = Vec::new();
        for message in corpus.iter().filter(|message| message.lang == language) {
            let result = analyze(&Input {
                subject: &message.subject,
                body: Some(&message.body),
                direct: message.direct,
                authored: Some(written),
                received_at: written,
                zone: Paris,
                max_ttl_hours: 168,
            });
            predicted += u32::from(result.priority);
            actual += u32::from(message.priority);
            correct += u32::from(result.priority && message.priority);
            if result.priority != message.priority {
                wrong.push(format!("priority: {}", message.subject));
            }
            let found = result
                .deadlines
                .iter()
                .find(|deadline| deadline.ends_at.is_some())
                .map(|deadline| deadline.date);
            if let Some(found) = found {
                deadlines_found += 1;
                if message.deadline == Some(found) {
                    deadlines_right += 1;
                } else {
                    wrong.push(format!("deadline {found}: {}", message.subject));
                }
            }
        }
        let precision = f64::from(correct) / f64::from(predicted);
        let recall = f64::from(correct) / f64::from(actual);
        let deadline_precision = f64::from(deadlines_right) / f64::from(deadlines_found);
        println!(
            "{language}: precision {precision:.2}, recall {recall:.2}, \
             deadline precision {deadline_precision:.2} over {deadlines_found}"
        );
        assert!(precision >= 0.85, "{language}: {wrong:#?}");
        assert!(recall >= 0.65, "{language}: {wrong:#?}");
        assert!(deadline_precision >= 0.95, "{language}: {wrong:#?}");
    }
}
