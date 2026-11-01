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
use mailstronaut_core::worker::Handle;
use mailstronaut_mail::imap::Wake;
use tokio::{
    runtime::Runtime,
    sync::{Notify, mpsc},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::account_actions::{Access, Cycle, SyncFailure};

const FIRST_RETRY: Duration = Duration::from_secs(15);
const MAX_RETRY: Duration = Duration::from_secs(300);
/// RFC 2177 allows 29 minutes. Common NAT timeouts are shorter, and renewing is how a
/// connection that died without closing, such as across a suspend, gets noticed.
const IDLE_RENEW: Duration = Duration::from_mins(9);
/// One delivery usually produces several notifications; one cycle covers them all.
const COALESCE: Duration = Duration::from_secs(1);

pub enum Update {
    Started(String),
    Finished(String, Result<Cycle, String>),
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
        if work
            .updates
            .send(Update::Started(id.clone()))
            .await
            .is_err()
        {
            return;
        }
        let extra =
            core::mem::take(&mut *work.extra.lock().unwrap_or_else(PoisonError::into_inner));
        let result = work
            .access
            .sync_account(&work.store, &work.config, &work.account, &extra, &cancel)
            .await;
        let report = match result {
            Ok(cycle) => {
                retry = FIRST_RETRY;
                wait = Some(poll);
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
        if cancel.is_cancelled()
            || work
                .updates
                .send(Update::Finished(id, report))
                .await
                .is_err()
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
    use super::{FIRST_RETRY, MAX_RETRY, next_retry};

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
