//! Applies the local priority rules to unread inbox mail, in the background.
//!
//! Nothing here opens a message or sets Seen, and nothing leaves the machine. The store
//! decides what is in the priority group; this only supplies results.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono_tz::Tz;
use mailstronaut_config::Config;
use mailstronaut_core::{
    store::analysis::{AnalysisInput, AnalysisResult},
    worker::Handle,
};
use mailstronaut_priority::{Analysis, Input, POLICY_VERSION, Reason, analyze};
use tokio::{
    runtime::Runtime,
    sync::{Notify, mpsc},
};
use tokio_util::sync::CancellationToken;

const BATCH: u32 = 64;
const BETWEEN_BATCHES: Duration = Duration::from_millis(20);
const AFTER_FAILURE: Duration = Duration::from_mins(1);
/// Older unread mail is backlog, which analysis does not go looking through.
const LOOKBACK_SECONDS: i64 = 30 * 24 * 3600;

/// What the rules need from the configuration.
struct Rules {
    accounts: Vec<String>,
    own: HashMap<String, HashSet<String>>,
    zone: Tz,
    max_ttl_hours: u32,
}

impl Rules {
    fn new(config: &Config) -> Self {
        let own = config
            .accounts
            .iter()
            .map(|account| {
                let addresses = std::iter::once(&account.address)
                    .chain(&account.aliases)
                    .map(|address| address.to_lowercase())
                    .collect();
                (account.id.clone(), addresses)
            })
            .collect();
        Self {
            accounts: config
                .accounts
                .iter()
                .map(|account| account.id.clone())
                .collect(),
            own,
            zone: crate::mailbox::local_zone(config.time_zone.as_deref())
                .parse()
                .unwrap_or(chrono_tz::UTC),
            max_ttl_hours: config.priority.max_ttl_hours,
        }
    }

    fn result(&self, input: &AnalysisInput) -> Option<AnalysisResult> {
        let direct = self.own.get(&input.account).is_some_and(|own| {
            input
                .to
                .iter()
                .any(|address| own.contains(&address.to_lowercase()))
        });
        let analysis = analyze(&Input {
            subject: &input.subject,
            body: input.body.as_deref(),
            direct,
            authored: input.authored,
            received_at: input.received_at,
            zone: self.zone,
            max_ttl_hours: self.max_ttl_hours,
        });
        Some(AnalysisResult {
            id: input.id,
            policy: analysis.policy,
            expires_at: analysis.expires_at,
            due: due(&analysis),
            data: serde_json::to_string(&analysis).ok()?,
        })
    }
}

/// The deadline that sets the expiry, as local date and optional time.
fn due(analysis: &Analysis) -> Option<String> {
    let expires_at = analysis.expires_at?;
    let deadline = analysis
        .deadlines
        .iter()
        .find(|deadline| deadline.ends_at == Some(expires_at))?;
    Some(deadline.time.map_or_else(
        || deadline.date.to_string(),
        |time| format!("{}T{}", deadline.date, time.format("%H:%M")),
    ))
}

/// Why a message was put in the priority group, in words, if it was.
pub fn explanation(analysis: &Analysis) -> Option<String> {
    if !analysis.priority {
        return None;
    }
    let reasons: Vec<_> = analysis
        .reasons
        .iter()
        .filter_map(|reason| match reason {
            Reason::DirectRecipient => Some("addressed to you"),
            Reason::ActionRequested => Some("asks you for something"),
            Reason::Urgent => Some("says it is urgent"),
            Reason::Deadline => Some("states a deadline"),
            Reason::NoActionNeeded | Reason::BulkMail | Reason::AlreadyExpired => None,
        })
        .collect();
    (!reasons.is_empty()).then(|| format!("Needs attention: {}.", reasons.join(", ")))
}

/// Each date found, with the exact words it came from and anything assumed about it.
/// Uncertain ones say so: they are shown for the reader to judge, and decide nothing.
pub fn deadline_notes(analysis: &Analysis) -> Vec<String> {
    analysis
        .deadlines
        .iter()
        .map(|deadline| {
            let date = deadline.date.format("%-d %B %Y");
            let when = deadline.time.map_or_else(
                || date.to_string(),
                |time| format!("{date}, {}", time.format("%H:%M")),
            );
            let zone = if deadline.zone_assumed {
                format!("{}, assumed", deadline.zone)
            } else {
                "as stated".to_owned()
            };
            if deadline.ends_at.is_some() {
                format!("Deadline: {when} ({zone}), from “{}”.", deadline.quote)
            } else {
                format!(
                    "Possibly a deadline: “{}” could not be read with certainty, so it decides nothing.",
                    deadline.quote
                )
            }
        })
        .collect()
}

/// Dropping this does not stop analysis; cancelling its token does.
pub struct PriorityEngine {
    wake: Arc<Notify>,
}

impl PriorityEngine {
    /// The receiver hears whenever the priority group may have changed.
    pub fn start(
        runtime: &Runtime,
        store: Handle,
        config: &Config,
        cancel: CancellationToken,
    ) -> (Self, mpsc::Receiver<()>) {
        let wake: Arc<Notify> = Arc::default();
        let (changed, receiver) = mpsc::channel(1);
        runtime.spawn(run(
            Rules::new(config),
            store,
            wake.clone(),
            changed,
            cancel,
        ));
        (Self { wake }, receiver)
    }

    /// New mail may have been cached.
    pub fn wake(&self) {
        self.wake.notify_one();
    }
}

async fn run(
    rules: Rules,
    store: Handle,
    wake: Arc<Notify>,
    changed: mpsc::Sender<()>,
    cancel: CancellationToken,
) {
    loop {
        let wait = match batch(&rules, &store).await {
            Ok(0) => None,
            Ok(prioritized) => {
                if prioritized > 0 {
                    // A full channel already says the same thing.
                    _ = changed.try_send(());
                }
                Some(BETWEEN_BATCHES)
            }
            Err(()) => Some(AFTER_FAILURE),
        };
        tokio::select! {
            () = cancel.cancelled() => return,
            () = wake.notified() => {}
            () = tokio::time::sleep(wait.unwrap_or_default()), if wait.is_some() => {}
        }
    }
}

/// Returns how many messages were analyzed, plus one if any of them qualified: an empty
/// batch and a batch of nothing interesting must be told apart.
async fn batch(rules: &Rules, store: &Handle) -> Result<u32, ()> {
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or_default();
    let inputs = store
        .analysis_pending(
            rules.accounts.clone(),
            POLICY_VERSION,
            now - LOOKBACK_SECONDS,
            BATCH,
        )
        .await
        .map_err(|_| ())?;
    if inputs.is_empty() {
        return Ok(0);
    }
    let results: Vec<_> = inputs
        .iter()
        .filter_map(|input| rules.result(input))
        .collect();
    if results.is_empty() {
        return Err(());
    }
    let prioritized = results
        .iter()
        .filter(|result| result.expires_at.is_some())
        .count();
    store.store_analysis(results).await.map_err(|_| ())?;
    Ok(u32::try_from(prioritized).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    fn result(body: &str, direct: bool) -> Analysis {
        let written = chrono_tz::Europe::Paris
            .with_ymd_and_hms(2026, 11, 4, 9, 0, 0)
            .single()
            .expect("date")
            .timestamp();
        analyze(&Input {
            subject: "Synthetic",
            body: Some(body),
            direct,
            authored: Some(written),
            received_at: written,
            zone: chrono_tz::Europe::Paris,
            max_ttl_hours: 168,
        })
    }

    #[test]
    fn a_priority_message_says_why_in_words_and_quotes_its_evidence() {
        let analysis = result("Merci de répondre avant le 15 novembre.", true);
        assert_eq!(
            explanation(&analysis).as_deref(),
            Some("Needs attention: addressed to you, asks you for something, states a deadline.")
        );
        assert_eq!(
            deadline_notes(&analysis),
            ["Deadline: 15 November 2026 (Europe/Paris, assumed), from “avant le 15 novembre”."]
        );
        let timed = result("Please reply by 15 November at 17:30 UTC.", true);
        assert_eq!(
            deadline_notes(&timed),
            [
                "Deadline: 15 November 2026, 17:30 (as stated), from “reply by 15 November at 17:30 UTC”."
            ]
        );
    }

    #[test]
    fn an_uncertain_date_is_shown_as_uncertain_and_an_ordinary_message_claims_nothing() {
        let ambiguous = result("Due 03/04/2027.", true);
        assert_eq!(explanation(&ambiguous), None);
        assert_eq!(
            deadline_notes(&ambiguous),
            [
                "Possibly a deadline: “Due 03/04/2027” could not be read with certainty, so it decides nothing."
            ]
        );
        let ordinary = result("Here are the notes from yesterday.", true);
        assert_eq!(explanation(&ordinary), None);
        assert_eq!(deadline_notes(&ordinary), Vec::<String>::new());
    }
}
