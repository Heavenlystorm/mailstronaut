//! A bounded channel to one dedicated blocking SQLite owner, not a daemon/RPC service.

use core::{error::Error as StdError, fmt};
use std::{path::PathBuf, thread};

use tokio::sync::{mpsc, oneshot};

use crate::{
    mime::{self, Projection},
    store::{
        self, Store,
        mailbox::{Cursor, Folder, GmailId, Membership, Summary},
    },
};

/// Maximum queued store operations. Full-size originals are separately bounded
/// to 8 MiB, so this queue cannot retain an unbounded mailbox in RAM.
pub const QUEUE_CAPACITY: usize = 16;

/// Redacted store-worker failures that can cross the UI/engine channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// The owner stopped or could not be started.
    Closed,
    /// A UID/provider identity conflicts with cached mail.
    IdentityConflict,
    /// A malformed or oversized request was rejected.
    InvalidRequest,
    /// The original is missing or failed integrity checks.
    MessageUnavailable,
    /// The original cannot be projected within the MIME budget.
    UnsupportedMessage,
    /// The database requires a different application/schema or migration repair.
    Schema,
    /// The mailbox generation changed; resynchronize before retrying.
    StaleMailbox,
    /// Local filesystem/database access failed.
    Storage,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Closed => "mail storage worker unavailable",
            Self::IdentityConflict => "server identity conflicts with cached mail",
            Self::InvalidRequest => "invalid or oversized mail storage request",
            Self::MessageUnavailable => "cached message unavailable or damaged",
            Self::UnsupportedMessage => "message exceeds supported MIME limits",
            Self::Schema => "mail database upgrade or recovery required",
            Self::StaleMailbox => "mailbox generation changed; resynchronization required",
            Self::Storage => "mail storage operation failed",
        })
    }
}
impl StdError for Error {}
impl From<store::Error> for Error {
    fn from(error: store::Error) -> Self {
        match error {
            store::Error::IdentityConflict => Self::IdentityConflict,
            store::Error::InvalidInput => Self::InvalidRequest,
            store::Error::MissingMessage | store::Error::CorruptBlob => Self::MessageUnavailable,
            store::Error::StaleMailbox => Self::StaleMailbox,
            store::Error::UnsupportedSchema | store::Error::Migration(_) => Self::Schema,
            store::Error::Mime(_) => Self::UnsupportedMessage,
            store::Error::Io(_) | store::Error::Sql(_) | store::Error::Metadata(_) => Self::Storage,
        }
    }
}

type Reply<T> = oneshot::Sender<Result<T, Error>>;

enum Command {
    Account(String, Reply<()>),
    Folder(String, String, bool, Reply<Folder>),
    Select(String, i64, core::num::NonZeroU32, Reply<()>),
    Cache {
        account: String,
        membership: Membership,
        received_at: i64,
        gmail: Option<GmailId>,
        original: Vec<u8>,
        reply: Reply<i64>,
    },
    Inbox(Vec<String>, Option<Cursor>, u32, Reply<Vec<Summary>>),
    Message(String, i64, Reply<Projection>),
    Shutdown(oneshot::Sender<()>),
}

/// Cloneable request handle; no SQLite connection or filesystem work runs here.
#[derive(Clone)]
pub struct Handle(mpsc::Sender<Command>);
impl Handle {
    async fn request<T>(&self, command: impl FnOnce(Reply<T>) -> Command) -> Result<T, Error> {
        let (reply, response) = oneshot::channel();
        self.0
            .send(command(reply))
            .await
            .map_err(|_| Error::Closed)?;
        response.await.map_err(|_| Error::Closed)?
    }

    /// Register an active account without deleting disabled accounts' data.
    ///
    /// # Errors
    /// Returns a redacted worker or storage error.
    pub async fn ensure_account(&self, account: String) -> Result<(), Error> {
        self.request(|reply| Command::Account(account, reply)).await
    }
    /// Cache a discovered folder, preserving a previously selected generation.
    ///
    /// # Errors
    /// Returns a redacted worker, identity or storage error.
    pub async fn ensure_folder(
        &self,
        account: String,
        name: String,
        inbox: bool,
    ) -> Result<Folder, Error> {
        self.request(|reply| Command::Folder(account, name, inbox, reply))
            .await
    }
    /// Apply SELECT's UID namespace before body/flag work is accepted.
    ///
    /// # Errors
    /// Returns a redacted worker, identity or storage error.
    pub async fn select_folder(
        &self,
        account: String,
        folder: i64,
        validity: core::num::NonZeroU32,
    ) -> Result<(), Error> {
        self.request(|reply| Command::Select(account, folder, validity, reply))
            .await
    }
    /// Enqueue one bounded original. Backpressure suspends the caller without
    /// blocking the GUI or retaining an unbounded number of message bodies.
    ///
    /// # Errors
    /// Returns a redacted size, UID generation, identity or storage error.
    pub async fn cache_message(
        &self,
        account: String,
        membership: Membership,
        received_at: i64,
        gmail: Option<GmailId>,
        original: Vec<u8>,
    ) -> Result<i64, Error> {
        if original.len() > mime::MAX_INPUT_BYTES {
            return Err(Error::InvalidRequest);
        }
        self.request(|reply| Command::Cache {
            account,
            membership,
            received_at,
            gmail,
            original,
            reply,
        })
        .await
    }
    /// Obtain an immutable, bounded page for currently enabled accounts.
    ///
    /// # Errors
    /// Returns a redacted request, storage or worker error.
    pub async fn inbox(
        &self,
        accounts: Vec<String>,
        after: Option<Cursor>,
        limit: u32,
    ) -> Result<Vec<Summary>, Error> {
        self.request(|reply| Command::Inbox(accounts, after, limit, reply))
            .await
    }
    /// Read a verified original for exactly this account/message identity.
    ///
    /// # Errors
    /// Returns a redacted integrity, storage or worker error.
    pub async fn message(&self, account: String, id: i64) -> Result<Projection, Error> {
        self.request(|reply| Command::Message(account, id, reply))
            .await
    }
}

/// Lifetime of the single blocking database owner.
///
/// Retain this in the engine;
/// give only its Handle to consumers. Call shutdown before the Tokio driver ends.
/// Dropping the last handle closes the channel and lets the thread finish queued
/// work; explicit shutdown also waits for that thread off the GUI thread.
pub struct Worker {
    handle: Handle,
    thread: thread::JoinHandle<()>,
}
impl Worker {
    /// Open/migrate the store on its owning thread. No raw connection crosses
    /// the async boundary; a failed startup is joined before returning the error.
    ///
    /// # Errors
    /// Returns a redacted open/migration/thread error without resetting data.
    pub async fn open(root: PathBuf) -> Result<Self, Error> {
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
        let (ready, opened) = oneshot::channel();
        let thread = thread::Builder::new()
            .name("mailstronaut-store".into())
            .spawn(move || match Store::open(&root) {
                Ok(mut store) => {
                    if ready.send(Ok(())).is_ok() {
                        run(&mut store, receiver);
                    }
                }
                Err(error) => {
                    let _sent = ready.send(Err(Error::from(error)));
                }
            })
            .map_err(|_| Error::Closed)?;
        match opened.await.unwrap_or(Err(Error::Closed)) {
            Ok(()) => Ok(Self {
                handle: Handle(sender),
                thread,
            }),
            Err(error) => {
                let _joined = tokio::task::spawn_blocking(move || thread.join()).await;
                Err(error)
            }
        }
    }
    /// A lightweight clone for the engine and immutable view queries.
    #[must_use]
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }
    /// Stop after previously queued commands and await the owner thread. Any
    /// consumer requests queued after Shutdown fail closed when its receiver drops.
    ///
    /// # Errors
    /// Returns Closed if the owner panicked or had already exited.
    pub async fn shutdown(self) -> Result<(), Error> {
        let (reply, response) = oneshot::channel();
        let sent = self.handle.0.send(Command::Shutdown(reply)).await.is_ok();
        let acknowledged = sent && response.await.is_ok();
        tokio::task::spawn_blocking(move || self.thread.join())
            .await
            .map_err(|_| Error::Closed)?
            .map_err(|_| Error::Closed)?;
        if acknowledged {
            Ok(())
        } else {
            Err(Error::Closed)
        }
    }
}

fn respond<T>(reply: Reply<T>, operation: impl FnOnce() -> Result<T, store::Error>) {
    // A discarded read/cache request need not perform expensive parsing or I/O.
    // Once a transaction starts it completes atomically even if its caller goes.
    if !reply.is_closed() {
        let _sent = reply.send(operation().map_err(Error::from));
    }
}
fn run(store: &mut Store, mut receiver: mpsc::Receiver<Command>) {
    while let Some(command) = receiver.blocking_recv() {
        match command {
            Command::Account(account, reply) => respond(reply, || store.ensure_account(&account)),
            Command::Folder(account, name, inbox, reply) => {
                respond(reply, || store.ensure_folder(&account, &name, inbox));
            }
            Command::Select(account, folder, validity, reply) => {
                respond(reply, || store.select_folder(&account, folder, validity));
            }
            Command::Cache {
                account,
                membership,
                received_at,
                gmail,
                original,
                reply,
            } => respond(reply, || {
                store.cache_message(&account, membership, received_at, gmail, &original)
            }),
            Command::Inbox(accounts, cursor, limit, reply) => {
                respond(reply, || store.inbox(&accounts, cursor, limit));
            }
            Command::Message(account, id, reply) => respond(reply, || store.message(&account, id)),
            Command::Shutdown(reply) => {
                let _sent = reply.send(());
                break;
            }
        }
    }
}
