//! Keeps the search index in step with the store, and answers queries from it.
//!
//! Search works from the first moment: until the index opens, or if it never does, the
//! workspace falls back to the store's literal search.

use std::{
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use mailstronaut_core::{store::search::SearchBatch, worker::Handle};
use mailstronaut_search::{Change, Document, Index};
use tokio::{runtime::Runtime, sync::Notify};
use tokio_util::sync::CancellationToken;

/// Small enough not to hold up the store's owner, and to keep indexing in the background.
const BATCH: u32 = 64;
const BETWEEN_BATCHES: Duration = Duration::from_millis(20);
const AFTER_FAILURE: Duration = Duration::from_mins(1);
/// Matching messages considered for one search, in every page of its results.
const MAX_HITS: usize = 1000;

#[derive(Default)]
struct Shared {
    index: OnceLock<Index>,
    queued: AtomicU64,
    wake: Notify,
}

/// The running indexer. Cloning shares it; cancelling its token stops indexing.
#[derive(Clone)]
pub struct MailSearch(Arc<Shared>);

impl MailSearch {
    pub fn start(
        runtime: &Runtime,
        directory: PathBuf,
        store: Handle,
        cancel: CancellationToken,
    ) -> Self {
        let shared = Arc::new(Shared::default());
        runtime.spawn(run(shared.clone(), directory, store, cancel));
        Self(shared)
    }

    /// New mail may have been cached.
    pub fn wake(&self) {
        self.0.wake.notify_one();
    }

    /// Messages not yet searchable through the index.
    pub fn queued(&self) -> u64 {
        self.0.queued.load(Ordering::Relaxed)
    }

    /// Matching messages in order, or `None` if the store's literal search should answer.
    pub async fn hits(&self, query: String, accounts: Vec<String>) -> Option<Vec<i64>> {
        let shared = self.0.clone();
        tokio::task::spawn_blocking(move || {
            shared.index.get()?.search(&query, &accounts, MAX_HITS).ok()
        })
        .await
        .ok()
        .flatten()
    }
}

async fn run(shared: Arc<Shared>, directory: PathBuf, store: Handle, cancel: CancellationToken) {
    let opened = tokio::task::spawn_blocking(move || Index::open(&directory)).await;
    let Ok(Ok(opened)) = opened else {
        return;
    };
    if opened.fresh && store.requeue_search().await.is_err() {
        return;
    }
    if shared.index.set(opened.index).is_err() {
        return;
    }
    loop {
        let wait = match index_batch(&shared, &store).await {
            Ok(true) => Some(BETWEEN_BATCHES),
            Ok(false) => None,
            Err(()) => Some(AFTER_FAILURE),
        };
        tokio::select! {
            () = cancel.cancelled() => return,
            () = shared.wake.notified() => {}
            () = tokio::time::sleep(wait.unwrap_or_default()), if wait.is_some() => {}
        }
    }
}

/// Returns whether anything was indexed. Only a committed batch is acknowledged, so an
/// interruption repeats it, which is harmless.
async fn index_batch(shared: &Arc<Shared>, store: &Handle) -> Result<bool, ()> {
    let batch = store.search_pending(BATCH).await.map_err(|_| ())?;
    shared.queued.store(batch.queued, Ordering::Relaxed);
    let Some(through) = batch.through else {
        return Ok(false);
    };
    let done = u64::try_from(batch.items.len()).unwrap_or_default();
    let remaining = batch.queued.saturating_sub(done);
    let changes = changes(batch);
    let indexing = shared.clone();
    tokio::task::spawn_blocking(move || {
        indexing
            .index
            .get()
            .ok_or(())?
            .apply(changes)
            .map_err(|_| ())
    })
    .await
    .map_err(|_| ())??;
    store.search_indexed(through).await.map_err(|_| ())?;
    shared.queued.store(remaining, Ordering::Relaxed);
    Ok(true)
}

fn changes(batch: SearchBatch) -> Vec<Change> {
    batch
        .items
        .into_iter()
        .map(|item| match item.text {
            Some(text) => Change::Put(Document {
                id: item.id,
                account: text.account,
                received_at: text.received_at,
                subject: text.subject,
                sender: text.sender,
                addresses: text.addresses,
                body: text.body,
            }),
            None => Change::Remove(item.id),
        })
        .collect()
}
