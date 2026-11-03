//! One bounded polling loop per configured account while the application runs, plus
//! one IDLE connection that wakes it as soon as the inbox changes.
//!
//! IDLE only says "look now": every change still goes through the ordinary cycle, and
//! polling still covers the other folders and any server that does not offer IDLE.
//!
//! Failures stay per account and cached mail stays readable. Refused credentials stop
//! that account's retries, so a wrong password cannot lock it at the server.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use mailstronaut_config::{Account, Config};
use mailstronaut_core::{sync::Progress, worker::Handle};
use mailstronaut_mail::imap::Wake;
use tokio::{
    runtime::Runtime,
    sync::{Notify, mpsc},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::account_actions::{Access, Cycle, SyncFailure};
use gpui::{Context, EventEmitter};

const FIRST_RETRY: Duration = Duration::from_secs(15);
const MAX_RETRY: Duration = Duration::from_secs(300);
/// RFC 2177 allows 29 minutes. Common NAT timeouts are shorter, and renewing is how a
/// connection that died without closing, such as across a suspend, gets noticed.
const IDLE_RENEW: Duration = Duration::from_mins(9);
/// One delivery usually produces several notifications; one cycle covers them all.
const COALESCE: Duration = Duration::from_secs(1);
/// While a folder still has history to fetch, its account comes back this often instead of
/// at the polling interval, so the history moves steadily without waiting a whole poll.
const BACKFILL_PAUSE: Duration = Duration::from_secs(30);

/// Progress of one folder's history pass: `completed` of `total` messages reviewed.
/// `total` is None until the server reports how many messages the folder holds.
#[derive(Clone)]
pub struct Fetch {
    pub folder: String,
    pub completed: u32,
    pub total: Option<u32>,
}

#[derive(Clone)]
pub enum Update {
    Started(String),
    Finished(String, Result<Cycle, String>),
    /// Fetch progress for one folder of an account, while its cycle runs.
    Progress(String, Fetch),
}

/// Application-owned polling lifetime. Window views subscribe to updates, but
/// their destruction does not by itself stop the account loops.
pub struct Session {
    runtime: Arc<Runtime>,
    access: Access,
    lifetime: CancellationToken,
    engine: Option<Engine>,
    generation: u64,
    #[cfg(test)]
    observed_failure: bool,
}

/// A sync update delivered to a currently attached window.
pub struct Event(pub Update);
impl EventEmitter<Event> for Session {}

impl Session {
    /// Construct a retained sync owner without starting network activity.
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self {
            runtime,
            access: Access::default(),
            lifetime: CancellationToken::new(),
            engine: None,
            generation: 0,
            #[cfg(test)]
            observed_failure: false,
        }
    }

    /// Share the same credential access state with foreground account actions.
    pub fn access(&self) -> Access {
        self.access.clone()
    }

    /// Replace the active account loops when the declaration changes.
    pub fn configure(&mut self, config: &Arc<Config>, store: Option<Handle>, cx: &Context<Self>) {
        self.engine = None;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let Some(store) = store.filter(|_| config.sync.automatic) else {
            return;
        };
        let (engine, mut updates) = Engine::start(
            &self.runtime,
            &self.access,
            &store,
            config,
            self.lifetime.child_token(),
        );
        self.engine = Some(engine);
        cx.spawn(async move |this, cx| {
            while let Some(update) = updates.recv().await {
                if this
                    .update(cx, |this, cx| {
                        if generation == this.generation {
                            #[cfg(test)]
                            {
                                this.observed_failure |=
                                    matches!(&update, Update::Finished(_, Err(_)));
                            }
                            cx.emit(Event(update));
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Request a pass for all configured accounts.
    pub fn wake_all(&self) {
        if let Some(engine) = &self.engine {
            engine.wake_all();
        }
    }

    /// Include a requested folder in the next account pass.
    pub fn include_folder(&self, account: &str, folder: String) {
        if let Some(engine) = &self.engine {
            engine.include_folder(account, folder);
        }
    }

    #[cfg(test)]
    pub(crate) const fn observed_failure(&self) -> bool {
        self.observed_failure
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.lifetime.cancel();
    }
}

/// How long until the next cycle: history continues quickly, everything else at the poll.
fn next_wait(poll: Duration, backfill_pending: bool) -> Duration {
    if backfill_pending {
        poll.min(BACKFILL_PAUSE)
    } else {
        poll
    }
}

struct Entry {
    wake: Arc<Notify>,
    extra: Arc<Mutex<Vec<String>>>,
}

/// Dropping the engine stops every account loop.
pub struct Engine {
    accounts: HashMap<String, Entry>,
    cancel: CancellationToken,
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl Engine {
    pub fn start(
        runtime: &Runtime,
        access: &Access,
        store: &Handle,
        config: &Arc<Config>,
        cancel: CancellationToken,
    ) -> (Self, mpsc::Receiver<Update>) {
        let (updates, receiver) = mpsc::channel(16);
        let mut accounts = HashMap::new();
        for account in &config.accounts {
            let entry = Entry {
                wake: Arc::default(),
                extra: Arc::default(),
            };
            runtime.spawn(run(
                Loop {
                    access: access.clone(),
                    store: store.clone(),
                    config: config.clone(),
                    account: account.clone(),
                    wake: entry.wake.clone(),
                    extra: entry.extra.clone(),
                    updates: updates.clone(),
                },
                cancel.clone(),
            ));
            accounts.insert(account.id.clone(), entry);
        }
        (Self { accounts, cancel }, receiver)
    }

    /// Check every account now, including one stopped on refused credentials: a
    /// wake is an explicit request, normally after the user changed something.
    pub fn wake_all(&self) {
        for account in self.accounts.values() {
            account.wake.notify_one();
        }
    }

    /// Also fetch this folder on the account's next cycle, and start that cycle now.
    pub fn include_folder(&self, account: &str, folder: String) {
        if let Some(account) = self.accounts.get(account) {
            account
                .extra
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(folder);
            account.wake.notify_one();
        }
    }
}

struct Loop {
    access: Access,
    store: Handle,
    config: Arc<Config>,
    account: Account,
    wake: Arc<Notify>,
    extra: Arc<Mutex<Vec<String>>>,
    updates: mpsc::Sender<Update>,
}

async fn run(work: Loop, cancel: CancellationToken) {
    let poll = Duration::from_secs(u64::from(work.config.sync.poll_seconds));
    let mut wait = Some(Duration::ZERO);
    let mut retry = FIRST_RETRY;
    let mut watcher: Option<JoinHandle<()>> = None;
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = work.wake.notified() => {}
            () = tokio::time::sleep(wait.unwrap_or_default()), if wait.is_some() => {}
        }
        let id = work.account.id.clone();
        if !crate::cancellable_send::send(&work.updates, Update::Started(id.clone()), &cancel).await
        {
            return;
        }
        let extra =
            core::mem::take(&mut *work.extra.lock().unwrap_or_else(PoisonError::into_inner));
        let updates = work.updates.clone();
        let progress_account = id.clone();
        let emit = move |folder: &str, progress: Progress| {
            // A full channel drops this report; the next window reports again.
            _ = updates.try_send(Update::Progress(
                progress_account.clone(),
                Fetch {
                    folder: folder.to_owned(),
                    completed: progress.completed,
                    total: progress.total,
                },
            ));
        };
        let result = work
            .access
            .sync_account(
                &work.store,
                &work.config,
                &work.account,
                &extra,
                &emit,
                &cancel,
            )
            .await;
        let report = match result {
            Ok(cycle) => {
                retry = FIRST_RETRY;
                wait = Some(next_wait(poll, cycle.backfill_pending));
                // Only after a good cycle: the credentials are known to work, and a
                // watcher stopped on refused ones comes back once they are fixed.
                if watcher.as_ref().is_none_or(JoinHandle::is_finished) {
                    watcher = Some(tokio::spawn(watch(
                        work.access.clone(),
                        work.account.clone(),
                        work.wake.clone(),
                        cancel.clone(),
                    )));
                }
                Ok(cycle)
            }
            Err(SyncFailure::Retry(error)) => {
                wait = Some(retry);
                retry = next_retry(retry);
                Err(error)
            }
            Err(SyncFailure::Stop(error)) => {
                wait = None;
                Err(error)
            }
        };
        if !crate::cancellable_send::send(&work.updates, Update::Finished(id, report), &cancel)
            .await
        {
            return;
        }
    }
}

/// Keep an IDLE connection open on the inbox, and reconnect it when it fails. A
/// reconnect may have missed anything, so it always asks for one cycle.
async fn watch(access: Access, account: Account, wake: Arc<Notify>, cancel: CancellationToken) {
    let mut retry = FIRST_RETRY;
    let mut reconnected = false;
    loop {
        let began = Instant::now();
        match watch_connection(&access, &account, &wake, reconnected, &cancel).await {
            Ok(()) => {
                // No IDLE on this server: polling is the whole story, and this task
                // stays parked so that the account loop does not start another.
                cancel.cancelled().await;
                return;
            }
            Err(SyncFailure::Stop(_)) => return,
            Err(SyncFailure::Retry(_)) => {}
        }
        if began.elapsed() >= MAX_RETRY {
            retry = FIRST_RETRY;
        }
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(retry) => {}
        }
        retry = next_retry(retry);
        reconnected = true;
    }
}

/// Returns `Ok` only when the server does not offer IDLE.
async fn watch_connection(
    access: &Access,
    account: &Account,
    wake: &Notify,
    reconnected: bool,
    cancel: &CancellationToken,
) -> Result<(), SyncFailure> {
    let inbox = account.folders.inbox.as_deref().unwrap_or("INBOX");
    let mut connection = access.connect_watcher(account, cancel).await?;
    if !connection.supports("IDLE") {
        return Ok(());
    }
    if reconnected {
        wake.notify_one();
    }
    let mut last = None;
    loop {
        let position = connection
            .examine(inbox, cancel)
            .await
            .map_err(SyncFailure::from_imap)?;
        // A change between two IDLE periods produces no notification, but it does
        // move the folder's position. The first look has nothing to compare with.
        if last.is_some_and(|last| last != position) {
            wake.notify_one();
        }
        last = Some(position);
        match connection
            .idle(IDLE_RENEW, cancel)
            .await
            .map_err(SyncFailure::from_imap)?
        {
            Wake::Renew => {}
            Wake::Changed => {
                cancel
                    .run_until_cancelled(tokio::time::sleep(COALESCE))
                    .await;
                wake.notify_one();
                // The folder is expected to differ now: this change has been reported.
                last = None;
            }
        }
    }
}

fn next_retry(current: Duration) -> Duration {
    (current * 2).min(MAX_RETRY)
}

#[cfg(test)]
mod tests {
    use super::{BACKFILL_PAUSE, FIRST_RETRY, MAX_RETRY, next_retry, next_wait};
    use core::time::Duration;

    #[test]
    fn history_continues_quickly_and_everything_else_waits_for_the_poll() {
        let poll = Duration::from_secs(300);
        assert_eq!(next_wait(poll, true), BACKFILL_PAUSE);
        assert_eq!(next_wait(poll, false), poll);
        // A poll shorter than the pause is never lengthened.
        assert_eq!(
            next_wait(Duration::from_secs(10), true),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn retries_back_off_to_the_five_minute_ceiling() {
        let mut delay = FIRST_RETRY;
        let mut steps = Vec::new();
        for _ in 0..6 {
            steps.push(delay.as_secs());
            delay = next_retry(delay);
        }
        assert_eq!(steps, [15, 30, 60, 120, 240, 300]);
        assert_eq!(next_retry(MAX_RETRY), MAX_RETRY);
    }
}
