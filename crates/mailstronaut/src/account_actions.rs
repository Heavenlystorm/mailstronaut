//! Foreground account actions assembled from the existing encrypted/auth/mail APIs.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use mailstronaut_config::{Account, Auth, Config, Provider};
use mailstronaut_core::{actions, store::flags, store::outbox::Draft, sync, worker::Handle};
use mailstronaut_mail::{
    auth,
    credentials::{Key, Purpose, Store},
    imap, oauth,
    smtp::{Outcome, Transport},
};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

const CANCELLED: &str = "Account operation cancelled.";

#[derive(Clone)]
struct Open {
    store: Store,
    session: auth::Session,
}

/// Whether a send froze the draft. An unfrozen draft stays editable; a frozen one
/// lives in the outbox and is only ever resubmitted by an explicit user action.
pub enum Sent {
    NotQueued(String),
    Queued(Result<Outcome, String>),
}

/// One encrypted backend and shared IMAP/SMTP refresh session for the workspace.
#[derive(Clone)]
pub struct Access {
    opened: Arc<Mutex<Option<Open>>>,
    connections: Arc<Semaphore>,
}
impl Default for Access {
    fn default() -> Self {
        Self {
            opened: Arc::default(),
            connections: Arc::new(Semaphore::new(2)),
        }
    }
}

impl Access {
    async fn foreground(&self, cancel: &CancellationToken) -> Result<Open, String> {
        let mut current = cancel
            .run_until_cancelled(self.opened.lock())
            .await
            .ok_or(CANCELLED)?;
        let opened = if let Some(opened) = current.as_ref() {
            opened.clone()
        } else {
            let store = cancel
                .run_until_cancelled(Store::connect_foreground())
                .await
                .ok_or(CANCELLED)?
                .map_err(|error| error.to_string())?;
            Open {
                session: auth::Session::new(store.clone()),
                store,
            }
        };
        cancel
            .run_until_cancelled(opened.store.unlock_foreground())
            .await
            .ok_or(CANCELLED)?
            .map_err(|error| error.to_string())?;
        *current = Some(opened.clone());
        drop(current);
        Ok(opened)
    }

    pub async fn save_password(
        &self,
        account: &Account,
        outgoing: bool,
        value: String,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        if value.is_empty() || value.len() > mailstronaut_mail::credentials::MAX_SECRET_BYTES {
            return Err("Enter a nonempty password within the supported size limit.".into());
        }
        let Auth::Password { secret_ref } = selected_auth(account, outgoing) else {
            return Err("This connection uses browser sign-in.".into());
        };
        let key = Key::new(&account.id, secret_ref, Purpose::Password)
            .map_err(|error| error.to_string())?;
        let secret = value.into();
        let opened = self.foreground(cancel).await?;
        if cancel.is_cancelled() {
            return Err(CANCELLED.into());
        }
        // Once issued, finish the bounded encrypted write instead of aborting it.
        opened
            .store
            .put(&key, &secret)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn sign_in(
        &self,
        account: &Account,
        outgoing: bool,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        if matches!(selected_auth(account, outgoing), Auth::Password { .. }) {
            return Err("This connection uses a password, not browser sign-in.".into());
        }
        let provider = cancel
            .run_until_cancelled(oauth::Provider::prepare(
                selected_auth(account, outgoing),
                &account.tls,
            ))
            .await
            .ok_or(CANCELLED)?
            .map_err(|error| error.to_string())?;
        let opened = self.foreground(cancel).await?;
        let flow = cancel
            .run_until_cancelled(provider.begin(&account.address))
            .await
            .ok_or(CANCELLED)?
            .map_err(|error| error.to_string())?;
        let uri = flow
            .authorization_url()
            .as_str()
            .parse()
            .map_err(|_| "Invalid browser authorization request.")?;
        // The standard desktop portal opens the external browser. Neither the URL
        // nor portal diagnostics (which can contain it) enter application errors.
        let request = cancel
            .run_until_cancelled(
                ashpd::desktop::open_uri::OpenFileRequest::default().send_uri(&uri),
            )
            .await
            .ok_or(CANCELLED)?
            .map_err(|_| "Browser portal is unavailable or refused the request.")?;
        request
            .response()
            .map_err(|_| "Browser sign-in was cancelled or refused.")?;
        let grant = flow
            .finish(cancel)
            .await
            .map_err(|error| error.to_string())?;
        // A completed grant must be committed even if the view closes now.
        opened
            .session
            .accept(account, outgoing, grant)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn sync(
        &self,
        store: &Handle,
        config: &Config,
        account: &Account,
        before: Option<sync::Position>,
        older: bool,
        cancel: &CancellationToken,
    ) -> Result<sync::Page, String> {
        if older && before.is_none() {
            return Err("Sync newest mail before requesting an older page.".into());
        }
        let opened = self.foreground(cancel).await?;
        let _permit = cancel
            .run_until_cancelled(self.connections.acquire())
            .await
            .ok_or(CANCELLED)?
            .map_err(|_| "Mail connection queue is unavailable.")?;
        let credential = opened
            .session
            .resolve(account, false, cancel)
            .await
            .map_err(|error| error.to_string())?;
        let endpoint = account
            .incoming_endpoint()
            .ok_or("Incoming server is not configured.")?;
        let mut connection = match imap::Connection::connect(
            &endpoint,
            &account.tls,
            &account.user_name,
            credential.authentication(),
            cancel,
        )
        .await
        {
            Ok(connection) => connection,
            Err(error) => {
                if error == imap::Error::Authentication {
                    opened.session.invalidate(account, false).await;
                }
                return Err(error.to_string());
            }
        };
        let context = flags::context(account).map_err(|error| error.to_string())?;
        let inbox = account.folders.inbox.as_deref().unwrap_or("INBOX");
        sync::discover(store, &mut connection, &account.id, &context, inbox, cancel)
            .await
            .map_err(|error| error.to_string())?;
        actions::apply(
            store,
            &mut connection,
            &actions::Request {
                account: &account.id,
                context: &context,
                retry: true,
            },
            cancel,
        )
        .await
        .map_err(|error| error.to_string())?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let earliest = if older {
            0
        } else {
            now.saturating_sub(u64::from(config.sync.initial_days) * 86400)
        };
        let request = sync::Request {
            account: &account.id,
            context: &context,
            folder: inbox,
            inbox: true,
            before: if older { before } else { None },
            earliest: i64::try_from(earliest)
                .map_err(|_| "System time is outside the supported range.")?,
            max_original: config.sync.max_body_bytes.min(8 * 1024 * 1024),
        };
        sync::page(store, &mut connection, &request, cancel)
            .await
            .map_err(|error| error.to_string())
    }
}

impl Access {
    /// Unlock before freezing, so a locked keyring leaves the draft editable.
    pub async fn send_draft(
        &self,
        store: &Handle,
        account: &Account,
        draft: Draft,
        cancel: &CancellationToken,
    ) -> Sent {
        let opened = match self.foreground(cancel).await {
            Ok(opened) => opened,
            Err(error) => return Sent::NotQueued(error),
        };
        let saved = async {
            store.ensure_account(draft.account.clone()).await?;
            store.save_draft(draft.clone()).await
        };
        if let Err(error) = saved.await {
            return Sent::NotQueued(error.to_string());
        }
        if cancel.is_cancelled() {
            return Sent::NotQueued("Sending cancelled. The draft was kept.".into());
        }
        let queued = store
            .queue_draft(
                draft.account.clone(),
                draft.id.clone(),
                draft.revision,
                matches!(account.provider, Provider::Google),
            )
            .await;
        match queued {
            Ok(queued) => Sent::Queued(
                submit_frozen(store, account, &queued.id, &opened.session, cancel).await,
            ),
            Err(error) => Sent::NotQueued(error.to_string()),
        }
    }

    /// Submit an existing outbox entry. `retry` is `None` for a pending entry, and
    /// otherwise records whether the user accepted a possible duplicate.
    pub async fn submit_queued(
        &self,
        store: &Handle,
        account: &Account,
        id: String,
        retry: Option<bool>,
        cancel: &CancellationToken,
    ) -> Result<Outcome, String> {
        let opened = self.foreground(cancel).await?;
        if let Some(confirm_unknown) = retry {
            store
                .retry_submission(account.id.clone(), id.clone(), confirm_unknown)
                .await
                .map_err(|error| error.to_string())?;
        }
        submit_frozen(store, account, &id, &opened.session, cancel).await
    }
}

async fn submit_frozen(
    store: &Handle,
    account: &Account,
    id: &str,
    session: &auth::Session,
    cancel: &CancellationToken,
) -> Result<Outcome, String> {
    let resolved = session.resolve(account, true, cancel).await;
    if cancel.is_cancelled() {
        store
            .cancel_submission(account.id.clone(), id.into())
            .await
            .map_err(|error| error.to_string())?;
        return Ok(Outcome::CancelledBeforeSubmission);
    }
    let credential = resolved.map_err(|error| error.to_string())?;
    let endpoint = account
        .outgoing_endpoint()
        .ok_or("Outgoing server is not configured.")?;
    let transport = Transport::prepare(
        &endpoint,
        &account.tls,
        account.outgoing_user_name(),
        credential.authentication(),
    )
    .await
    .map_err(|error| error.to_string())?;
    drop(credential);
    if cancel.is_cancelled() {
        store
            .cancel_submission(account.id.clone(), id.into())
            .await
            .map_err(|error| error.to_string())?;
        return Ok(Outcome::CancelledBeforeSubmission);
    }
    let submission = store
        .begin_submission(account.id.clone(), id.into())
        .await
        .map_err(|error| error.to_string())?;
    let allowed = std::iter::once(&account.address)
        .chain(&account.aliases)
        .any(|address| submission.message.sender_is(address));
    let outcome = if allowed {
        transport.submit(&submission.message, cancel).await
    } else {
        Outcome::CancelledBeforeSubmission
    };
    store
        .finish_submission(submission, outcome)
        .await
        .map_err(|error| error.to_string())?;
    if outcome == Outcome::AuthenticationRequired {
        session.invalidate(account, true).await;
    }
    if !allowed {
        return Err("Sender identity is no longer enabled. Nothing was submitted.".into());
    }
    Ok(outcome)
}

pub fn selected_auth(account: &Account, outgoing: bool) -> &Auth {
    if outgoing {
        account.smtp_auth.as_ref().unwrap_or(&account.auth)
    } else {
        &account.auth
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use mailstronaut_core::worker::Worker;

    use super::*;

    #[tokio::test]
    #[ignore = "requires the explicitly isolated credentials.sh bus and XDG profile"]
    async fn private_cancellation_and_validation_never_unlock() {
        let root =
            PathBuf::from(std::env::var_os("MAILSTRONAUT_SECRET_TEST_ROOT").expect("private root"));
        assert!(root.is_absolute());
        assert_eq!(
            std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("private bus"),
            format!("unix:path={}/runtime/bus", root.display())
        );
        let config: Config = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "accounts": [{
                "id": "cancel-only", "provider": "icloud", "address": "cancel@example.test", "displayName": "Cancellation fixture",
                "userName": "synthetic", "auth": {"kind": "password", "secretRef": "cancel"}
            }]
        }))
        .expect("fixture configuration");
        assert_eq!(config.validate(), []);
        let account = &config.accounts[0];
        let access = Access::default();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            access
                .save_password(account, false, "synthetic-not-stored".into(), &cancel)
                .await
                .err()
                .as_deref(),
            Some(CANCELLED)
        );
        let active = CancellationToken::new();
        assert!(
            access
                .save_password(account, false, String::new(), &active)
                .await
                .is_err()
        );
        assert!(access.sign_in(account, false, &active).await.is_err());
        let worker = Worker::open(root.join("cancel-store"))
            .await
            .expect("private store");
        let handle = worker.handle();
        assert!(
            access
                .sync(&handle, &config, account, None, true, &active)
                .await
                .is_err()
        );
        assert_eq!(
            access
                .sync(&handle, &config, account, None, false, &cancel)
                .await
                .err()
                .as_deref(),
            Some(CANCELLED)
        );
        assert!(access.opened.lock().await.is_none());

        // Poll a real blocked operation before cancellation. No keyring is opened
        // while the foreground mutex is held, even if the implementation regresses.
        let guard = access.opened.lock().await;
        let cancel = CancellationToken::new();
        let pending = access.save_password(account, false, "synthetic-not-stored".into(), &cancel);
        tokio::pin!(pending);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut pending)
                .await
                .is_err()
        );
        cancel.cancel();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), pending)
                .await
                .expect("cancelled queue wait")
                .err()
                .as_deref(),
            Some(CANCELLED)
        );
        drop(guard);
        assert!(access.opened.lock().await.is_none());
        println!("Cancelled/invalid account actions left the keyring unopened and mailbox empty.");
    }
}
