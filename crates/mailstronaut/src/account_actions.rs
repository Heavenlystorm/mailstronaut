//! Foreground account actions assembled from the existing encrypted/auth/mail APIs.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use mailstronaut_config::{Account, Auth, Config, Provider};
use mailstronaut_core::{
    actions,
    store::{flags, mailbox::FolderRole, outbox::Draft},
    sync,
    worker::Handle,
};
use mailstronaut_mail::{
    auth,
    credentials::{Key, Purpose, Store},
    imap, oauth,
    smtp::{Outcome, Transport},
};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

const CANCELLED: &str = "Account operation cancelled.";
const FETCH_STARTED: sync::Progress = sync::Progress {
    completed: 0,
    total: None,
};
/// Four active accounts at one connection each, per the architecture's initial target.
const MAX_CONNECTIONS: usize = 4;
/// A cycle examines at most this many newest UID windows per folder, while new mail keeps
/// arriving in them.
const CYCLE_PAGES: u32 = 8;
/// The newest windows every cycle examines per folder, even when nothing new is found: they
/// carry arrivals and flag changes.
const RECENT_WINDOWS: u32 = 2;
/// History windows per folder and cycle. A folder with more history continues in the next
/// cycle from its durable cursor, so one large folder cannot hold up the others.
const BACKFILL_WINDOWS: u32 = 16;
/// A dropped connection is retried on a new connection this many times in one folder's
/// cycle, before the cycle itself fails and waits.
const DROP_RETRIES: u32 = 4;
/// One request for older mail stops after this many new messages.
const OLDER_MESSAGES: u32 = 50;
/// Or after this many UID windows, so that a folder of cached mail cannot hold it for long.
const OLDER_WINDOWS: u32 = 64;

#[derive(Clone)]
struct Open {
    store: Store,
    session: auth::Session,
    automatic: bool,
}

/// Whether a send froze the draft. An unfrozen draft stays editable; a frozen one
/// lives in the outbox and is only ever resubmitted by an explicit user action.
pub enum Sent {
    NotQueued(String),
    Queued(Result<Outcome, String>),
}

/// What one automatic pass over an account did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Cycle {
    pub downloaded: u32,
    pub refreshed: u32,
    pub actions: u32,
    /// Why a queued move could not be done, with the message back in its folder.
    pub move_failure: Option<imap::Error>,
    /// Some folder still has history to fetch, and the next cycle should continue it soon.
    pub backfill_pending: bool,
    /// A sent message's copy could not be appended to the Sent folder. It stays queued and is
    /// tried again by a later cycle.
    pub sent_copy_pending: bool,
}

/// What one request for older mail did.
#[derive(Clone, Copy, Debug)]
pub struct Older {
    pub downloaded: u32,
    /// Where the next request continues, or `None` at the beginning of the folder.
    pub resume: Option<sync::Position>,
}

/// Whether trying again later could help without the user changing anything.
pub enum SyncFailure {
    Retry(String),
    Stop(String),
    Cancelled,
}
impl SyncFailure {
    /// Refused or unusable credentials and invalid trust must not be retried
    /// automatically: repeated failures can lock an account at the server.
    pub fn from_imap(error: imap::Error) -> Self {
        let text = error.to_string();
        match error {
            imap::Error::Cancelled => Self::Cancelled,
            imap::Error::Authentication
            | imap::Error::AuthenticationUnavailable
            | imap::Error::Configuration
            | imap::Error::StarttlsRequired
            | imap::Error::Tls
            | imap::Error::TrustStore => Self::Stop(text),
            _ => Self::Retry(text),
        }
    }
    fn from_sync(error: sync::Error) -> Self {
        match error {
            sync::Error::Transport(error) => Self::from_imap(error),
            sync::Error::Storage(error) => Self::Retry(error.to_string()),
        }
    }
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
            connections: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
        }
    }
}

impl Access {
    async fn background(&self, cancel: &CancellationToken) -> Result<Open, String> {
        let mut current = cancel
            .run_until_cancelled(self.opened.lock())
            .await
            .ok_or(CANCELLED)?;
        if let Some(opened) = current.as_ref() {
            let mut opened = opened.clone();
            opened.automatic = true;
            return Ok(opened);
        }
        // A sandbox keeps its keyring behind the Secret portal, which needs no prompt, so
        // automatic work may open that store silently. Its file-backed store cannot replace
        // items natively, so automatic OAuth refresh stays refused before provider contact.
        let store = if ashpd::is_sandboxed() {
            cancel
                .run_until_cancelled(Store::connect_sandboxed())
                .await
                .ok_or(CANCELLED)?
                .map_err(|error| error.to_string())?
        } else {
            cancel
                .run_until_cancelled(Store::connect_native_background())
                .await
                .ok_or(CANCELLED)?
                .map_err(|error| error.to_string())?
        };
        let opened = Open {
            session: auth::Session::new(store.clone()),
            store,
            automatic: false,
        };
        *current = Some(opened.clone());
        drop(current);
        let mut automatic = opened;
        automatic.automatic = true;
        Ok(automatic)
    }

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
                automatic: false,
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
        sync::discover(
            store,
            &mut connection,
            &account.id,
            &context,
            inbox,
            &account.folders,
            cancel,
        )
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
        let earliest = if older {
            0
        } else {
            history_floor(config, unix_seconds())
        };
        let request = sync::Request {
            account: &account.id,
            context: &context,
            folder: inbox,
            inbox: true,
            before: if older { before } else { None },
            earliest,
            max_original: imap::MAX_ORIGINAL,
        };
        sync::page(store, &mut connection, &request, cancel)
            .await
            .map_err(|error| error.to_string())
    }
}

impl Access {
    async fn connect(
        opened: &Open,
        account: &Account,
        cancel: &CancellationToken,
    ) -> Result<imap::Connection, SyncFailure> {
        let credential = if opened.automatic {
            opened
                .session
                .resolve_automatic(account, false, cancel)
                .await
        } else {
            opened.session.resolve(account, false, cancel).await
        }
        .map_err(|error| SyncFailure::Stop(error.to_string()))?;
        let endpoint = account
            .incoming_endpoint()
            .ok_or_else(|| SyncFailure::Stop("Incoming server is not configured.".into()))?;
        match imap::Connection::connect(
            &endpoint,
            &account.tls,
            &account.user_name,
            credential.authentication(),
            cancel,
        )
        .await
        {
            Ok(connection) => Ok(connection),
            Err(error) => {
                if error == imap::Error::Authentication {
                    opened.session.invalidate(account, false).await;
                }
                Err(SyncFailure::from_imap(error))
            }
        }
    }

    /// Fetch mail from before the automatic sync window, for an explicit request. It walks
    /// back through the whole folder rather than starting below the oldest cached UID,
    /// since UID order is not date order in mail that has been moved or imported.
    pub async fn fetch_older(
        &self,
        store: &Handle,
        account: &Account,
        folder: &str,
        inbox: bool,
        resume: Option<sync::Position>,
        cancel: &CancellationToken,
    ) -> Result<Older, String> {
        let failed = |failure| match failure {
            SyncFailure::Retry(error) | SyncFailure::Stop(error) => error,
            SyncFailure::Cancelled => CANCELLED.into(),
        };
        let opened = self.foreground(cancel).await?;
        let _permit = cancel
            .run_until_cancelled(self.connections.acquire())
            .await
            .ok_or(CANCELLED)?
            .map_err(|_| "Mail connection queue is unavailable.")?;
        let context = flags::context(account).map_err(|error| error.to_string())?;
        let mut connection = Self::connect(&opened, account, cancel)
            .await
            .map_err(failed)?;
        let mut request = sync::Request {
            account: &account.id,
            context: &context,
            folder,
            inbox,
            before: resume,
            earliest: 0,
            max_original: imap::MAX_ORIGINAL,
        };
        let mut older = Older {
            downloaded: 0,
            resume,
        };
        for _ in 0..OLDER_WINDOWS {
            let page = sync::page(store, &mut connection, &request, cancel)
                .await
                .map_err(|error| error.to_string())?;
            older.downloaded += page.downloaded;
            older.resume = page.next_before;
            request.before = page.next_before;
            if page.next_before.is_none() || older.downloaded >= OLDER_MESSAGES {
                break;
            }
            // One connection's receive budget is smaller than several windows of mail.
            if page.downloaded > 0 {
                connection = Self::connect(&opened, account, cancel)
                    .await
                    .map_err(failed)?;
            }
        }
        Ok(older)
    }

    /// Download one attachment that stayed on the server, for an explicit request. Its
    /// UID is only trusted if the folder's UID namespace is still the one it came from.
    pub async fn fetch_attachment(
        &self,
        store: &Handle,
        account: &Account,
        message: i64,
        section: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, String> {
        let (folder, validity, uid) = store
            .location(account.id.clone(), message)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("This message is no longer on the server.")?;
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
        let mut connection = imap::Connection::connect_for_download(
            &endpoint,
            &account.tls,
            &account.user_name,
            credential.authentication(),
            cancel,
        )
        .await
        .map_err(|error| error.to_string())?;
        let position = connection
            .examine(&folder, cancel)
            .await
            .map_err(|error| error.to_string())?;
        if position.uid_validity() != Some(validity.get()) {
            return Err("This folder changed on the server. Sync, then try again.".into());
        }
        let (mime, content) = connection
            .part(uid, section, cancel)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("This attachment is no longer on the server.")?;
        tokio::task::spawn_blocking(move || {
            mailstronaut_core::outline::decode_part(&mime, &content)
        })
        .await
        .ok()
        .flatten()
        .ok_or_else(|| "This attachment could not be decoded.".to_owned())
    }

    /// A connection for watching a folder. The permit covers only sign-in: an idling
    /// connection can wait for hours and must not hold back another account's sync.
    pub async fn connect_watcher(
        &self,
        account: &Account,
        cancel: &CancellationToken,
    ) -> Result<imap::Connection, SyncFailure> {
        let opened = self.background(cancel).await.map_err(SyncFailure::Stop)?;
        let _permit = cancel
            .run_until_cancelled(self.connections.acquire())
            .await
            .ok_or_else(|| SyncFailure::Retry(CANCELLED.into()))?
            .map_err(|_| SyncFailure::Retry("Mail connection queue is unavailable.".into()))?;
        Self::connect(&opened, account, cancel).await
    }

    /// Discover folders, send queued flag changes, then fetch recent mail and continue
    /// each folder's history: the inbox, the standard role folders and any folder the user
    /// just opened. `progress` hears each history window as it commits.
    pub async fn sync_account(
        &self,
        store: &Handle,
        config: &Config,
        account: &Account,
        extra: &[String],
        progress: &(dyn Fn(&str, sync::Progress) + Sync),
        cancel: &CancellationToken,
    ) -> Result<Cycle, SyncFailure> {
        let opened = self.background(cancel).await.map_err(SyncFailure::Stop)?;
        let _permit = cancel
            .run_until_cancelled(self.connections.acquire())
            .await
            .ok_or_else(|| SyncFailure::Retry(CANCELLED.into()))?
            .map_err(|_| SyncFailure::Retry("Mail connection queue is unavailable.".into()))?;
        let context =
            flags::context(account).map_err(|error| SyncFailure::Stop(error.to_string()))?;
        let inbox = account.folders.inbox.as_deref().unwrap_or("INBOX");
        let mut connection = Self::connect(&opened, account, cancel).await?;
        let roles = sync::discover(
            store,
            &mut connection,
            &account.id,
            &context,
            inbox,
            &account.folders,
            cancel,
        )
        .await
        .map_err(SyncFailure::from_sync)?;
        let mut cycle = Cycle {
            actions: actions::apply(
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
            .map_err(SyncFailure::from_sync)?,
            ..Cycle::default()
        };
        let moves = actions::apply_moves(
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
        .map_err(SyncFailure::from_sync)?;
        cycle.actions += moves.moved;
        cycle.move_failure = moves.failure;
        // A failed operation retires its session, and mail still has to be fetched.
        if moves.failure.is_some() {
            connection = Self::connect(&opened, account, cancel).await?;
        }
        if let Some((sent, _)) = roles.iter().find(|(_, role)| *role == FolderRole::Sent) {
            connection = Self::append_sent_copy(
                &opened, account, store, connection, sent, &mut cycle, cancel,
            )
            .await?;
        }
        let now = unix_seconds();
        let earliest = history_floor(config, now);
        let others = roles
            .iter()
            .filter(|(_, role)| {
                matches!(
                    role,
                    FolderRole::Sent | FolderRole::Archive | FolderRole::Drafts
                )
            })
            .map(|(name, _)| name.as_str())
            .chain(extra.iter().map(String::as_str))
            .filter(|name| *name != inbox);
        let folders: Vec<(&str, bool)> = core::iter::once((inbox, true))
            .chain(others.map(|name| (name, false)))
            .collect();
        // Arrivals and flags of every folder come before any history, so a long backfill
        // never delays new mail. The remaining server work can be cancelled.
        for &(folder, is_inbox) in &folders {
            progress(folder, FETCH_STARTED);
            let request = folder_request(&account.id, &context, folder, is_inbox, earliest);
            connection = Self::recent_pages(
                &opened, account, store, connection, request, &mut cycle, cancel,
            )
            .await?;
        }
        for &(folder, is_inbox) in &folders {
            let pass = FolderPass {
                request: folder_request(&account.id, &context, folder, is_inbox, earliest),
                cap_days: config.sync.initial_days,
                now,
                progress,
            };
            connection = Self::backfill_folder(
                &opened, account, store, connection, pass, &mut cycle, cancel,
            )
            .await?;
        }
        Ok(cycle)
    }

    /// Append the copy of a sent message to the Sent folder. A refused copy stays queued for a
    /// later cycle: mail must still be fetched, and the message was sent already, so nothing on
    /// this path may send it again.
    async fn append_sent_copy(
        opened: &Open,
        account: &Account,
        store: &Handle,
        mut connection: imap::Connection,
        sent: &str,
        cycle: &mut Cycle,
        cancel: &CancellationToken,
    ) -> Result<imap::Connection, SyncFailure> {
        if let Err(error) =
            actions::append_sent(store, &mut connection, &account.id, sent, cancel).await
        {
            if cancel.is_cancelled() {
                return Err(SyncFailure::from_sync(error));
            }
            cycle.sent_copy_pending = true;
            connection = Self::connect(opened, account, cancel).await?;
        }
        Ok(connection)
    }

    /// Fetch the newest windows of a folder. The first `RECENT_WINDOWS` are always examined;
    /// older ones only while they still bring new mail. Reconnects after any window that
    /// downloaded: one connection's receive budget is smaller than several windows.
    async fn recent_pages(
        opened: &Open,
        account: &Account,
        store: &Handle,
        mut connection: imap::Connection,
        mut request: sync::Request<'_>,
        cycle: &mut Cycle,
        cancel: &CancellationToken,
    ) -> Result<imap::Connection, SyncFailure> {
        let mut drops = 0;
        for index in 0..CYCLE_PAGES {
            let page = match sync::page(store, &mut connection, &request, cancel).await {
                Ok(page) => page,
                Err(error) if drops < DROP_RETRIES && dropped(error, cancel) => {
                    drops += 1;
                    connection = Self::connect(opened, account, cancel).await?;
                    continue;
                }
                Err(error) => return Err(SyncFailure::from_sync(error)),
            };
            cycle.downloaded += page.downloaded;
            cycle.refreshed += page.refreshed;
            if page.downloaded > 0 {
                connection = Self::connect(opened, account, cancel).await?;
            }
            request.before = page.next_before;
            let settled = index + 1 >= RECENT_WINDOWS && page.downloaded == 0;
            if page.next_before.is_none() || settled {
                break;
            }
        }
        Ok(connection)
    }

    /// Advance one folder's history pass by up to `BACKFILL_WINDOWS` windows. A window that
    /// downloaded nothing does not end the pass: cached mail and UID gaps are normal, and
    /// the durable cursor carries the pass into the next cycle when this one runs out.
    async fn backfill_folder(
        opened: &Open,
        account: &Account,
        store: &Handle,
        mut connection: imap::Connection,
        pass: FolderPass<'_>,
        cycle: &mut Cycle,
        cancel: &CancellationToken,
    ) -> Result<imap::Connection, SyncFailure> {
        let mut drops = 0;
        for _ in 0..BACKFILL_WINDOWS {
            let step = match sync::backfill_step(
                store,
                &mut connection,
                &pass.request,
                pass.cap_days,
                pass.now,
                cancel,
            )
            .await
            {
                Ok(step) => step,
                Err(error) if drops < DROP_RETRIES && dropped(error, cancel) => {
                    drops += 1;
                    connection = Self::connect(opened, account, cancel).await?;
                    continue;
                }
                Err(error) => return Err(SyncFailure::from_sync(error)),
            };
            cycle.downloaded += step.downloaded;
            (pass.progress)(pass.request.folder, step.progress);
            if step.finished {
                return Ok(connection);
            }
            if step.downloaded > 0 {
                connection = Self::connect(opened, account, cancel).await?;
            }
        }
        cycle.backfill_pending = true;
        Ok(connection)
    }

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
    let checked = transport.check(cancel).await;
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
    let outcome = match (allowed, checked) {
        (false, _) => Outcome::CancelledBeforeSubmission,
        (true, Err(outcome)) => outcome,
        (true, Ok(())) => transport.submit(&submission.message, cancel).await,
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

/// One folder's place in an automatic cycle: its request, its history cap and the clock.
struct FolderPass<'a> {
    request: sync::Request<'a>,
    cap_days: Option<u32>,
    now: i64,
    progress: &'a (dyn Fn(&str, sync::Progress) + Sync),
}

const fn folder_request<'a>(
    account: &'a str,
    context: &'a str,
    folder: &'a str,
    inbox: bool,
    earliest: i64,
) -> sync::Request<'a> {
    sync::Request {
        account,
        context,
        folder,
        inbox,
        before: None,
        earliest,
        max_original: imap::MAX_ORIGINAL,
    }
}

/// Unix seconds now, to compare with when mail arrived.
fn unix_seconds() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or(i64::MAX)
}

/// The oldest arrival time that sync fetches. Without a declared age limit the complete
/// history is wanted, so nothing is older than the floor.
pub fn history_floor(config: &Config, now: i64) -> i64 {
    config.sync.initial_days.map_or(i64::MIN, |days| {
        now.saturating_sub(i64::from(days) * 86_400)
    })
}

/// Whether a failed step only lost its connection, so that it can be repeated on a new one.
/// Each window's cursor is saved only after it committed, so repeating loses and duplicates
/// nothing. Cancellation is never retried.
fn dropped(error: sync::Error, cancel: &CancellationToken) -> bool {
    !cancel.is_cancelled()
        && matches!(
            error,
            sync::Error::Transport(error)
                if matches!(SyncFailure::from_imap(error), SyncFailure::Retry(_))
        )
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

    #[test]
    fn only_a_lost_connection_is_repeated_and_cancellation_never_is() {
        let open = CancellationToken::new();
        assert!(dropped(
            sync::Error::Transport(imap::Error::Disconnected),
            &open
        ));
        assert!(dropped(sync::Error::Transport(imap::Error::Network), &open));
        assert!(!dropped(
            sync::Error::Transport(imap::Error::Authentication),
            &open
        ));
        assert!(!dropped(sync::Error::Transport(imap::Error::Tls), &open));
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(!dropped(
            sync::Error::Transport(imap::Error::Disconnected),
            &cancelled
        ));
    }

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
    #[tokio::test]
    #[ignore = "requires the isolated real keyring from credentials.sh"]
    async fn private_automatic_access_never_unlocks() {
        let root =
            PathBuf::from(std::env::var_os("MAILSTRONAUT_SECRET_TEST_ROOT").expect("private root"));
        assert!(root.is_absolute());
        assert_eq!(
            std::env::var("DBUS_SESSION_BUS_ADDRESS").expect("private bus"),
            format!("unix:path={}/runtime/bus", root.display())
        );
        assert_eq!(
            PathBuf::from(std::env::var_os("XDG_DATA_HOME").expect("private data")),
            root.join("data")
        );
        let config: Config = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1, "accounts": [{
                "id": "quiet-only", "provider": "icloud", "address": "quiet@example.test", "displayName": "Quiet access fixture",
                "userName": "synthetic", "auth": {"kind": "password", "secretRef": "quiet"}
            }]
        })).expect("fixture declaration");
        assert_eq!(config.validate(), []);
        let account = &config.accounts[0];
        let cancel = CancellationToken::new();
        let foreground = Access::default();
        foreground
            .save_password(account, false, "synthetic-quiet-password".into(), &cancel)
            .await
            .expect("explicit foreground save");
        let cached = Access::default();
        let opened = cached
            .background(&cancel)
            .await
            .expect("quiet native initialization");
        opened
            .session
            .resolve(account, false, &cancel)
            .await
            .expect("existing password resolves without network");
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let unopened = Access::default();
        assert_eq!(
            unopened.background(&cancelled).await.err().as_deref(),
            Some(CANCELLED)
        );
        assert!(unopened.opened.lock().await.is_none());

        let connection = zbus::Connection::session().await.expect("private bus");
        let service = zbus::Proxy::new(
            &connection,
            "org.freedesktop.secrets",
            "/org/freedesktop/secrets",
            "org.freedesktop.Secret.Service",
        )
        .await
        .expect("private service");
        let collection: zbus::zvariant::OwnedObjectPath = service
            .call("ReadAlias", &("default",))
            .await
            .expect("existing collection");
        let (_, prompt): (
            Vec<zbus::zvariant::OwnedObjectPath>,
            zbus::zvariant::OwnedObjectPath,
        ) = service
            .call("Lock", &(vec![collection],))
            .await
            .expect("lock private keyring");
        assert_eq!(prompt.as_str(), "/");
        let store = Worker::open(root.join("quiet-store"))
            .await
            .expect("private worker");
        let handle = store.handle();
        let outcome = tokio::time::timeout(
            Duration::from_secs(3),
            cached.sync_account(&handle, &config, account, &[], &|_, _| {}, &cancel),
        )
        .await
        .expect("cached automatic access must not prompt");
        assert!(matches!(outcome, Err(SyncFailure::Stop(message)) if message.contains("locked")));
        let fresh = Access::default();
        let outcome = tokio::time::timeout(
            Duration::from_secs(3),
            fresh.connect_watcher(account, &cancel),
        )
        .await
        .expect("fresh watcher must not prompt");
        assert!(matches!(outcome, Err(SyncFailure::Stop(message)) if message.contains("locked")));
        assert!(fresh.opened.lock().await.is_none());
        store.shutdown().await.expect("stop private worker");
        println!(
            "Automatic credentials: quiet native initialization, cancellation, cached-sync and fresh-IDLE locked denial passed without network or unlock."
        );
    }
}
