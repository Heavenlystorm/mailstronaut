//! Bounded local inference behind the native search field. Never runs on the UI thread.
use mailstronaut_config::Search;
use mailstronaut_core::{
    store::{
        browse::{Scope, SearchFilter},
        semantic::{Batch, Item, PROJECTION_REVISION},
    },
    worker::Handle,
};
use mailstronaut_search::semantic::{ENCODER_POLICY, Embedding, Encoder, Index};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    runtime::Handle as RuntimeHandle,
    sync::{Notify, watch},
};
use tokio_util::sync::CancellationToken;

const BATCH: u32 = 8;
const HITS: usize = 20;
// Development-only floor preserves the multilingual positive screens. It failed
// the no-answer gate: present these only as explicitly requested related mail.
const RELATED_FLOOR: f32 = 0.371;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum State {
    Loading,
    Ready {
        queued: u64,
        indexed: u64,
        truncated: u64,
        revision: u64,
    },
    Unavailable(&'static str),
}
impl State {
    pub fn label(&self) -> String {
        match self {
            Self::Loading => "Preparing local meaning search…".into(),
            Self::Unavailable(message) => (*message).into(),
            Self::Ready {
                queued,
                indexed,
                truncated,
                ..
            } => {
                let label = if *queued == 0 {
                    format!("Meaning search ready · {indexed} indexed")
                } else {
                    format!("Meaning index: {indexed} indexed · {queued} pending")
                };
                if *truncated > 0 {
                    format!("{label} · {truncated} excerpt-only")
                } else {
                    label
                }
            }
        }
    }
}

pub struct Hits {
    pub ids: Vec<i64>,
    pub sources: BTreeMap<i64, String>,
}
struct Owner {
    encoder: Encoder,
    index: Index,
    query: Option<(String, Vec<f32>)>,
    checkpoint: Instant,
    dirty: bool,
    revision: u64,
}
struct Shared {
    owner: Mutex<Option<Owner>>,
    status: watch::Sender<State>,
    wake: Notify,
    cancel: CancellationToken,
    settings: Search,
    accounts: Vec<String>,
    max_input: usize,
}
#[derive(Clone)]
pub struct Meaning(Arc<Shared>);
impl Meaning {
    pub fn start(
        runtime: &RuntimeHandle,
        directory: PathBuf,
        settings: Search,
        accounts: Vec<String>,
        store: Handle,
        cancel: CancellationToken,
        max_input: u32,
    ) -> (Self, watch::Receiver<State>) {
        let (status, changed) = watch::channel(State::Loading);
        let shared = Arc::new(Shared {
            owner: Mutex::new(None),
            status,
            wake: Notify::new(),
            cancel,
            settings,
            accounts,
            max_input: usize::try_from(max_input).unwrap_or(32768).min(32768),
        });
        runtime.spawn(run(shared.clone(), directory, store));
        (Self(shared), changed)
    }
    pub fn configured(&self, settings: &Search, accounts: &[String], max_input: u32) -> bool {
        self.0.max_input == usize::try_from(max_input).unwrap_or(32768).min(32768)
            && self.0.settings == *settings
            && self.0.accounts == accounts
            && !matches!(*self.0.status.borrow(), State::Unavailable(_))
            && !self.0.cancel.is_cancelled()
    }
    pub fn cancel(&self) -> CancellationToken {
        self.0.cancel.clone()
    }
    pub fn stop(&self) {
        self.0.cancel.cancel();
    }
    pub fn wake(&self) {
        self.0.wake.notify_one();
    }
    pub fn label(&self) -> String {
        self.0.status.borrow().label()
    }
    pub fn state(&self) -> State {
        self.0.status.borrow().clone()
    }

    pub async fn hits(
        &self,
        query: String,
        store: Handle,
        accounts: Vec<String>,
        scope: Scope,
        filter: SearchFilter,
    ) -> Result<Hits, &'static str> {
        const CANCELLED: &str = "Meaning query cancelled.";
        const UNAVAILABLE: &str = "Meaning search is unavailable; use word search.";
        // Dropping a retired query future also retires work waiting for the model
        // mutex. Already-running single-window inference remains bounded.
        let pending = CancellationToken::new();
        let _retire = pending.clone().drop_guard();
        if self.0.cancel.is_cancelled() {
            return Err(CANCELLED);
        }
        let allowed = store
            .semantic_candidates(accounts, scope, filter)
            .await
            .map_err(|_| "Meaning search cannot read its local scope.")?;
        let shared = self.0.clone();
        tokio::task::spawn_blocking(move || {
            if shared.cancel.is_cancelled() || pending.is_cancelled() {
                return Err(CANCELLED);
            }
            let mut slot = shared.owner.lock().map_err(|_| UNAVAILABLE)?;
            if shared.cancel.is_cancelled() || pending.is_cancelled() {
                return Err(CANCELLED);
            }
            let owner = slot.as_mut().ok_or(UNAVAILABLE)?;
            if owner.query.as_ref().is_none_or(|(text, _)| *text != query) {
                let vector = owner.encoder.query(&query).map_err(
                    |_| "Meaning query cannot be encoded; shorten it or use word search.",
                )?;
                owner.query = Some((query, vector));
            }
            if shared.cancel.is_cancelled() || pending.is_cancelled() {
                return Err(CANCELLED);
            }
            let vector = &owner.query.as_ref().ok_or(UNAVAILABLE)?.1;
            let ids = owner
                .index
                .search_above(vector, &allowed, HITS, RELATED_FLOOR)
                .map_err(|_| UNAVAILABLE)?;
            drop(slot);
            let sources = ids
                .iter()
                .filter_map(|id| allowed.get(id).map(|source| (*id, source.clone())))
                .collect();
            if shared.cancel.is_cancelled() || pending.is_cancelled() {
                Err(CANCELLED)
            } else {
                Ok(Hits { ids, sources })
            }
        })
        .await
        .map_err(|_| UNAVAILABLE)?
    }
}

async fn run(shared: Arc<Shared>, directory: PathBuf, store: Handle) {
    run_inner(shared.clone(), directory, store).await;
    // Release heavy tensors and the writer lock off the UI/async executor thread,
    // including failed indexing and retained GUI status handles.
    _ = tokio::task::spawn_blocking(move || {
        let mut slot = shared
            .owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let retired = slot.take();
        drop(slot);
        if let Some(mut owner) = retired
            && owner.dirty
        {
            _ = owner.index.checkpoint();
        }
    })
    .await;
}

fn load_owner(shared: &Shared, directory: &std::path::Path) -> Result<Owner, &'static str> {
    let path = shared
        .settings
        .model_directory
        .as_ref()
        .ok_or("Install the pinned local meaning-search model in Settings.")?;
    let policy = format!(
        "{ENCODER_POLICY}:{PROJECTION_REVISION}:input{}",
        shared.max_input
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    let index = loop {
        if shared.cancel.is_cancelled() {
            return Err("Meaning search was cancelled.");
        }
        match Index::open(directory, &policy) {
            Ok(index) => break index,
            Err(mailstronaut_search::Error::Busy) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                return Err("Local meaning index is unavailable; lexical search remains usable.");
            }
        }
    };
    let encoder = Encoder::load(std::path::Path::new(path))
        .map_err(|_| "Local meaning-search model is unavailable or fails verification.")?;
    Ok::<_, &'static str>(Owner {
        encoder,
        index,
        query: None,
        checkpoint: Instant::now(),
        dirty: false,
        revision: 0,
    })
}

async fn run_inner(shared: Arc<Shared>, directory: PathBuf, store: Handle) {
    let loading = shared.clone();
    let opened = tokio::task::spawn_blocking(move || load_owner(&loading, &directory)).await;
    let owner = match opened {
        Ok(Ok(owner)) if !shared.cancel.is_cancelled() => owner,
        Ok(Err(message)) => {
            shared.status.send_replace(State::Unavailable(message));
            return;
        }
        _ => return,
    };
    match shared.owner.lock() {
        Ok(mut slot) => *slot = Some(owner),
        Err(_) => return,
    }
    if store.requeue_semantic().await.is_err() {
        shared.status.send_replace(State::Unavailable(
            "Meaning-index queue is unavailable; lexical search remains usable.",
        ));
        return;
    }
    loop {
        if shared.cancel.is_cancelled() {
            return;
        }
        let Ok(batch) = store.semantic_pending(BATCH).await else {
            shared.status.send_replace(State::Unavailable(
                "Meaning-index queue is unavailable; lexical search remains usable.",
            ));
            return;
        };
        let Batch {
            items,
            through,
            queued,
        } = batch;
        let idle = through.is_none();
        for item in items {
            let indexing = shared.clone();
            let indexed = tokio::task::spawn_blocking(move || ingest(&indexing, item)).await;
            if !matches!(indexed, Ok(Ok(()))) {
                shared.status.send_replace(State::Unavailable(
                    "Local meaning indexing stopped; lexical search remains usable.",
                ));
                return;
            }
            if shared.cancel.is_cancelled() {
                return;
            }
        }
        if let Some(through) = through
            && store.semantic_indexed(through).await.is_err()
        {
            shared.status.send_replace(State::Unavailable(
                "Meaning-index queue is unavailable; lexical search remains usable.",
            ));
            return;
        }
        let reporting = shared.clone();
        let status = tokio::task::spawn_blocking(move || {
            let mut slot = reporting.owner.lock().ok()?;
            let owner = slot.as_mut()?;
            if owner.dirty && (idle || owner.checkpoint.elapsed() >= Duration::from_mins(1)) {
                owner.index.checkpoint().ok()?;
                owner.checkpoint = Instant::now();
                owner.dirty = false;
            }
            let (indexed, truncated) = owner.index.coverage().ok()?;
            let revision = owner.revision;
            drop(slot);
            Some(State::Ready {
                queued: if idle {
                    0
                } else {
                    queued.saturating_sub(u64::from(BATCH))
                },
                indexed,
                truncated,
                revision,
            })
        })
        .await
        .ok()
        .flatten();
        let Some(status) = status else {
            shared.status.send_replace(State::Unavailable(
                "Local meaning index is unavailable; lexical search remains usable.",
            ));
            return;
        };
        shared.status.send_replace(status);
        tokio::select! {
            () = shared.cancel.cancelled() => return,
            () = shared.wake.notified() => {},
            () = tokio::time::sleep(if idle { Duration::from_secs(30) } else { Duration::from_millis(20) }) => {},
        }
    }
}

fn ingest(shared: &Shared, item: Item) -> Result<(), ()> {
    if shared.cancel.is_cancelled() {
        return Err(());
    }
    let mut slot = shared.owner.lock().map_err(|_| ())?;
    let owner = slot.as_mut().ok_or(())?;
    let source = item
        .source
        .filter(|source| shared.accounts.contains(&source.account));
    if let Some(source) = source {
        if owner
            .index
            .contains(item.id, &source.revision)
            .map_err(|_| ())?
        {
            return Ok(());
        }
        let mut text = format!("{}\n\n{}", source.subject, source.body);
        let clipped = text.len() > shared.max_input;
        text.truncate(text.floor_char_boundary(shared.max_input.min(text.len())));
        if text.trim().is_empty() {
            owner.index.remove(item.id).map_err(|_| ())?;
        } else {
            let projection = owner
                .encoder
                .document_cancellable(&text, || shared.cancel.is_cancelled())
                .map_err(|_| ())?;
            if shared.cancel.is_cancelled() {
                return Err(());
            }
            owner
                .index
                .put(Embedding {
                    id: item.id,
                    source: source.revision,
                    vectors: projection.vectors,
                    truncated: source.truncated || clipped || projection.truncated,
                })
                .map_err(|_| ())?;
        }
    } else {
        if !owner.index.has_original(item.id).map_err(|_| ())? {
            return Ok(());
        }
        owner.index.remove(item.id).map_err(|_| ())?;
    }
    owner.dirty = true;
    owner.revision = owner.revision.wrapping_add(1);
    drop(slot);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::num::NonZeroU32;
    use mailstronaut_core::{
        store::{
            browse::FilteredPage,
            mailbox::{Flags, Membership},
        },
        worker::Worker,
    };

    async fn seed(store: &Handle, account: &str, uid: u32, body: &str) -> i64 {
        store.ensure_account(account.into()).await.expect("account");
        let folder = store
            .ensure_folder(account.into(), "INBOX".into(), true)
            .await
            .expect("folder");
        let namespace = NonZeroU32::new(42).expect("namespace");
        store
            .select_folder(account.into(), folder.id, namespace)
            .await
            .expect("select");
        store.cache_message(account.into(),Membership { folder: folder.id, uid_validity: namespace, uid: NonZeroU32::new(uid).expect("UID"), flags: Flags::default() },i64::from(uid),None,
            format!("From: École <ecole@example.test>\r\nSubject: Avis {uid}\r\nMessage-ID: <{account}-{uid}@example.test>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}").into_bytes()).await.expect("cached")
    }
    fn settings(path: &str) -> Search {
        serde_json::from_value(serde_json::json!({"semantic":true,"modelDirectory":path}))
            .expect("search preferences")
    }
    async fn wait(
        changed: &mut watch::Receiver<State>,
        predicate: impl Fn(&State) -> bool + Send + Sync,
    ) -> State {
        tokio::time::timeout(Duration::from_secs(180), async {
            loop {
                let state = changed.borrow().clone();
                if predicate(&state) {
                    return state;
                }
                assert!(!matches!(state, State::Unavailable(_)), "{state:?}");
                changed.changed().await.expect("status notification");
            }
        })
        .await
        .expect("bounded model operation")
    }
    #[tokio::test]
    async fn unavailable_model_is_explicit_retryable_and_never_consumes_canonical_work() {
        let root = tempfile::tempdir().expect("private profile");
        let worker = Worker::open(root.path().join("mail")).await.expect("owner");
        let store = worker.handle();
        seed(&store, "a", 1, "A local cached original.").await;
        let settings = settings(root.path().join("missing-model").to_str().expect("path"));
        let (engine, mut changed) = Meaning::start(
            &RuntimeHandle::current(),
            root.path().join("meaning"),
            settings.clone(),
            vec!["a".into()],
            store.clone(),
            CancellationToken::new(),
            32768,
        );
        wait(&mut changed, |state| matches!(state, State::Unavailable(_))).await;
        assert!(!engine.configured(&settings, &["a".into()], 32768));
        assert_eq!(
            store
                .semantic_pending(1)
                .await
                .expect("queue untouched")
                .queued,
            1
        );
        assert!(
            engine
                .hits(
                    "cached original".into(),
                    store.clone(),
                    vec!["a".into()],
                    Scope::Inbox,
                    SearchFilter::default()
                )
                .await
                .is_err()
        );
        engine.stop();
        worker.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    #[ignore = "Requires explicitly installed pinned MiniLM; local CPU acceptance only"]
    #[expect(
        clippy::too_many_lines,
        reason = "One synthetic lifecycle retains the same worker/model across scoped retrieval, deletion and cancellation."
    )]
    async fn real_meaning_worker_indexes_french_mail_scopes_results_and_handles_removal_without_seen()
     {
        let model = std::env::var("MAILSTRONAUT_TEST_MODEL_DIR").expect("explicit pinned model");
        let root = tempfile::tempdir().expect("private profile");
        let worker = Worker::open(root.path().join("mail")).await.expect("owner");
        let store = worker.handle();
        let french = "Rappel : la campagne de réinscription en doctorat est ouverte. Les doctorants doivent renouveler leur inscription auprès de l’école doctorale avant vendredi et transmettre les pièces demandées.";
        let expected = seed(&store, "a", 1, french).await;
        seed(
            &store,
            "a",
            2,
            "La bibliothèque ferme demain pour travaux. Les prêts sont prolongés.",
        )
        .await;
        let foreign = seed(&store, "b", 3, french).await;
        let accounts = vec!["a".into(), "b".into()];
        let unread = store.unread_counts(accounts.clone()).await.expect("unread");
        let (engine, mut changed) = Meaning::start(
            &RuntimeHandle::current(),
            root.path().join("meaning"),
            settings(&model),
            accounts.clone(),
            store.clone(),
            CancellationToken::new(),
            32768,
        );
        wait(&mut changed, |state| {
            matches!(
                state,
                State::Ready {
                    queued: 0,
                    indexed: 3,
                    ..
                }
            )
        })
        .await;
        let phrase = "reminder about doctoral school re-enrollment";
        assert_eq!(
            engine
                .hits(
                    "x".repeat(1025),
                    store.clone(),
                    accounts.clone(),
                    Scope::Inbox,
                    SearchFilter::default()
                )
                .await
                .err(),
            Some("Meaning query cannot be encoded; shorten it or use word search.")
        );
        let found = engine
            .hits(
                phrase.into(),
                store.clone(),
                accounts.clone(),
                Scope::Account("a".into()),
                SearchFilter::default(),
            )
            .await
            .expect("semantic retrieval");
        assert_eq!(found.ids[0], expected);
        assert!(!found.ids.contains(&foreign));
        let rows = store
            .revisioned_conversation_page(
                FilteredPage {
                    accounts: accounts.clone(),
                    scope: Scope::Account("a".into()),
                    query: phrase.into(),
                    hits: Some(found.ids),
                    filter: SearchFilter::default(),
                    after: None,
                    limit: 100,
                },
                found.sources,
            )
            .await
            .expect("canonical presentation");
        assert_eq!(rows[0].latest.id, expected);
        assert_eq!(
            store
                .unread_counts(accounts.clone())
                .await
                .expect("still unread"),
            unread
        );
        let mail = root.path().join("mail");
        tokio::task::spawn_blocking(move || {
            mailstronaut_core::store::Store::open(&mail)
                .expect("independent cache owner")
                .clear_cache("a")
                .expect("canonical removal")
        })
        .await
        .expect("remove cached mail");
        assert_eq!(
            engine
                .hits(
                    phrase.into(),
                    store.clone(),
                    accounts.clone(),
                    Scope::Account("a".into()),
                    SearchFilter::default()
                )
                .await
                .expect("immediate canonical exclusion")
                .ids,
            Vec::<i64>::new()
        );
        engine.wake();
        wait(&mut changed, |state| {
            matches!(
                state,
                State::Ready {
                    queued: 0,
                    indexed: 1,
                    ..
                }
            )
        })
        .await;
        engine.stop();
        assert!(
            engine
                .hits(
                    phrase.into(),
                    store,
                    accounts,
                    Scope::Inbox,
                    SearchFilter::default()
                )
                .await
                .is_err()
        );
        worker.shutdown().await.expect("shutdown");
    }
}
