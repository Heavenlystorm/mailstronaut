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
use mailstronaut_priority::{Analysis, Input, POLICY_VERSION, analyze};
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
