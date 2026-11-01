//! One bounded polling loop per configured account while the application runs.
//!
//! Failures stay per account and cached mail stays readable. Refused credentials stop
//! that account's retries, so a wrong password cannot lock it at the server.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use mailstronaut_config::{Account, Config};
use mailstronaut_core::worker::Handle;
use tokio::{
    runtime::Runtime,
    sync::{Notify, mpsc},
};
use tokio_util::sync::CancellationToken;

use crate::account_actions::{Access, Cycle, SyncFailure};

const FIRST_RETRY: Duration = Duration::from_secs(15);
const MAX_RETRY: Duration = Duration::from_secs(300);

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
