//! Read-only, resumable work units. No scheduler, credential prompt or UI lives here.

use core::{error::Error as StdError, fmt, num::NonZeroU32};

use mailstronaut_mail::imap::{self, Connection, Metadata};
use tokio_util::sync::CancellationToken;

use crate::{
    store::mailbox::{Flags, Folder, GmailId, Membership, UidSnapshot},
    worker,
};

const UID_WINDOW: u32 = 128;
const BODY_BUDGET: u32 = 16 * 1024 * 1024;

/// A paging cursor is meaningful only inside its exact mailbox UID namespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Position {
    /// Local folder identity, scoped to the owning database.
    pub folder: i64,
    /// SELECT generation when this cursor was committed.
    pub uid_validity: NonZeroU32,
    /// Exclusive upper UID bound for the next page.
    pub before: NonZeroU32,
}

/// One explicitly requested account/folder page. Account workers serialize work
/// for their selected connection; callers bound the number of concurrent workers.
pub struct Request<'a> {
    /// Stable enabled configuration account ID.
    pub account: &'a str,
    /// Opaque server wire mailbox name.
    pub folder: &'a str,
    /// This folder participates in unified inbox queries.
    pub inbox: bool,
    /// Exclusive upper UID bound; None starts from SELECT's current UIDNEXT.
    pub before: Option<Position>,
    /// Skip downloading older originals, while still reconciling cached flags.
    pub earliest: i64,
    /// User's configured original-size budget, no greater than 8 MiB.
    pub max_original: u32,
}

/// Counts describe this page only. None means the UID namespace has been
/// scanned to its beginning, not that every message/attachment is cached.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Page {
    /// Continue strictly before this UID; only advanced after committed work.
    pub next_before: Option<Position>,
    /// Newly committed original messages.
    pub downloaded: u32,
    /// Existing originals whose observed flags were refreshed.
    pub refreshed: u32,
    /// Originals omitted because of age, size, unsupported MIME or concurrent expunge.
    pub skipped: u32,
}

/// Redacted transport/storage errors; no credentials, bodies or server diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Protocol, TLS, cancellation or network failure.
    Transport(imap::Error),
    /// Durable cache operation failed.
    Storage(worker::Error),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => error.fmt(f),
            Self::Storage(error) => error.fmt(f),
        }
    }
}
impl StdError for Error {}
impl From<imap::Error> for Error {
    fn from(value: imap::Error) -> Self {
        Self::Transport(value)
    }
}
impl From<worker::Error> for Error {
    fn from(value: worker::Error) -> Self {
        Self::Storage(value)
    }
}

/// Reconcile one closed UID window and fetch bounded PEEK originals serially.
///
/// No local/network transaction spans an await. A disconnected/cancelled page
/// can be retried: cached UID bindings make repeated originals idempotent.
///
/// # Errors
/// Returns redacted connection/storage failures. Its cursor is returned only
/// after every preceding storage operation commits. No persistent sync cursor
/// is claimed; callers may restart from newest safely after interruption.
pub async fn page(
    store: &worker::Handle,
    connection: &mut Connection,
    request: &Request<'_>,
    cancel: &CancellationToken,
) -> Result<Page, Error> {
    if request.max_original == 0 || request.max_original > 8 * 1024 * 1024 {
        return Err(imap::Error::Configuration.into());
    }
    let (folder, validity, upper) = prepare(store, connection, request, cancel).await?;
    let Some(last) = NonZeroU32::new(upper) else {
        return Ok(Page::default());
    };
    let first = NonZeroU32::new(upper.saturating_sub(UID_WINDOW - 1).max(1))
        .ok_or(imap::Error::Protocol)?;
    let uids = connection.search(first, last, None, cancel).await?;
    store
        .reconcile_span(
            request.account.into(),
            UidSnapshot {
                folder: folder.id,
                uid_validity: validity,
                first,
                last,
                present: uids.clone(),
            },
        )
        .await?;
    let mut page = Page {
        next_before: (first.get() > 1).then_some(Position {
            folder: folder.id,
            uid_validity: validity,
            before: first,
        }),
        ..Page::default()
    };
    let mut downloaded_bytes = 0;
    // Most recent UIDs first. Metadata is owned and bounded to 50 entries; no
    // borrowed protocol stream survives while the blocking store is awaited.
    let descending = uids.into_iter().rev().collect::<Vec<_>>();
    for batch in descending.chunks(50) {
        let mut metadata = connection.metadata(batch, cancel).await?;
        metadata.sort_unstable_by_key(|entry| core::cmp::Reverse(entry.uid));
        for observed in metadata {
            if cancel.is_cancelled() {
                return Err(imap::Error::Cancelled.into());
            }
            let binding = membership(folder.id, validity, &observed);
            if store
                .observe_membership(request.account.into(), binding)
                .await?
            {
                page.refreshed += 1;
                continue;
            }
            if observed.received_at < request.earliest
                || observed.size == 0
                || observed.size > request.max_original
            {
                page.skipped += 1;
                continue;
            }
            if downloaded_bytes + observed.size > BODY_BUDGET {
                // Include this unprocessed UID when the next page resumes.
                page.next_before = observed
                    .uid
                    .get()
                    .checked_add(1)
                    .and_then(NonZeroU32::new)
                    .map(|before| Position {
                        folder: folder.id,
                        uid_validity: validity,
                        before,
                    });
                return Ok(page);
            }
            let Some(message) = connection.peek(&observed, cancel).await? else {
                page.skipped += 1;
                continue;
            };
            downloaded_bytes += observed.size;
            match store
                .cache_message(
                    request.account.into(),
                    membership(folder.id, validity, &message.metadata),
                    message.metadata.received_at,
                    message.metadata.gmail_id.map(GmailId),
                    message.original,
                )
                .await
            {
                Ok(_) => page.downloaded += 1,
                Err(worker::Error::UnsupportedMessage) => page.skipped += 1,
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(page)
}
async fn prepare(
    store: &worker::Handle,
    connection: &mut Connection,
    request: &Request<'_>,
    cancel: &CancellationToken,
) -> Result<(Folder, NonZeroU32, u32), Error> {
    let selected = connection.select(request.folder, cancel).await?;
    let validity = selected
        .uid_validity
        .and_then(NonZeroU32::new)
        .ok_or(imap::Error::Protocol)?;
    let next = selected
        .uid_next
        .and_then(NonZeroU32::new)
        .ok_or(imap::Error::Protocol)?;
    store.ensure_account(request.account.into()).await?;
    let folder = store
        .ensure_folder(request.account.into(), request.folder.into(), request.inbox)
        .await?;
    store
        .select_folder(request.account.into(), folder.id, validity)
        .await?;
    let upper = request
        .before
        .filter(|position| position.folder == folder.id && position.uid_validity == validity)
        .map_or_else(
            || next.get(),
            |position| position.before.get().min(next.get()),
        )
        .saturating_sub(1);
    Ok((folder, validity, upper))
}

const fn membership(folder: i64, validity: NonZeroU32, metadata: &Metadata) -> Membership {
    Membership {
        folder,
        uid_validity: validity,
        uid: metadata.uid,
        flags: Flags {
            seen: metadata.seen,
            starred: metadata.starred,
        },
    }
}
