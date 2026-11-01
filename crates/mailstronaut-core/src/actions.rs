//! Bounded foreground application of durable, idempotent UID flag intentions.

use core::num::NonZeroU32;

use mailstronaut_mail::imap::{self, Connection};
use tokio_util::sync::CancellationToken;

use crate::{
    store::{flags::Attempt, moves::MoveAttempt},
    sync::Error,
    worker,
};

/// One explicit account-local action batch; SMTP is never included.
pub struct Request<'a> {
    /// Stable account ID from the active configuration.
    pub account: &'a str,
    /// Fingerprint of the same authenticated configuration used by Connection.
    pub context: &'a str,
    /// Explicitly requeue previous flag failures before processing this batch.
    pub retry: bool,
}

/// Apply at most 20 UID assignments using one authenticated connection.
///
/// Queued intent commits before I/O; each result commits before the next claim. Every
/// target is reselected and its UIDVALIDITY checked before sending STORE.
///
/// # Errors
/// Returns static transport/storage failures and stops this batch. Lost flag
/// acknowledgments are explicitly retryable, without touching SMTP uncertainty.
pub async fn apply(
    store: &worker::Handle,
    connection: &mut Connection,
    request: &Request<'_>,
    cancel: &CancellationToken,
) -> Result<u32, Error> {
    if request.retry {
        store
            .retry_flags(request.account.into(), request.context.into())
            .await?;
    }
    let mut confirmed = 0;
    for _ in 0..20 {
        if cancel.is_cancelled() {
            return Err(imap::Error::Cancelled.into());
        }
        let Some(attempt) = store
            .begin_flag(request.account.into(), request.context.into())
            .await?
        else {
            break;
        };
        match perform(connection, &attempt, cancel).await {
            Ok(Some(observed)) => {
                store.confirm_flag(attempt, observed).await?;
                confirmed += 1;
            }
            Ok(None) => {
                store.fail_flag(attempt, true).await?;
                return Err(worker::Error::StaleMailbox.into());
            }
            Err(error) => {
                store.fail_flag(attempt, false).await?;
                return Err(error.into());
            }
        }
    }
    Ok(confirmed)
}

/// What applying queued moves did. A failed move is reported here rather than as an
/// error, because it must not stop the account's mail from syncing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Moves {
    /// Messages moved on the server.
    pub moved: u32,
    /// Why the batch stopped. Any failure may have retired the connection.
    pub failure: Option<imap::Error>,
}

/// Apply at most 20 queued moves using one authenticated connection.
///
/// A failure is recorded and stops the batch, leaving the message visible in its source
/// folder. It is tried again on a later pass, unless this server can never do it.
///
/// # Errors
/// Returns storage failures and a stale mailbox.
pub async fn apply_moves(
    store: &worker::Handle,
    connection: &mut Connection,
    request: &Request<'_>,
    cancel: &CancellationToken,
) -> Result<Moves, Error> {
    if request.retry {
        store.retry_moves(request.account.into()).await?;
    }
    let mut moved = 0;
    for _ in 0..20 {
        if cancel.is_cancelled() {
            return Err(imap::Error::Cancelled.into());
        }
        let Some(attempt) = store
            .begin_move(request.account.into(), request.context.into())
            .await?
        else {
            break;
        };
        match transfer(connection, &attempt, cancel).await {
            Ok(done) => {
                store.finish_move(attempt, done).await?;
                if done {
                    moved += 1;
                } else {
                    return Err(worker::Error::StaleMailbox.into());
                }
            }
            Err(imap::Error::MoveUnavailable) => {
                store.abandon_move(attempt).await?;
                return Ok(Moves {
                    moved,
                    failure: Some(imap::Error::MoveUnavailable),
                });
            }
            Err(imap::Error::Cancelled) => {
                store.finish_move(attempt, false).await?;
                return Err(imap::Error::Cancelled.into());
            }
            Err(error) => {
                store.finish_move(attempt, false).await?;
                return Ok(Moves {
                    moved,
                    failure: Some(error),
                });
            }
        }
    }
    Ok(Moves {
        moved,
        failure: None,
    })
}

async fn transfer(
    connection: &mut Connection,
    attempt: &MoveAttempt,
    cancel: &CancellationToken,
) -> Result<bool, imap::Error> {
    let selected = connection.select(&attempt.folder, cancel).await?;
    let validity = selected
        .uid_validity
        .and_then(NonZeroU32::new)
        .ok_or(imap::Error::Protocol)?;
    if validity != attempt.validity {
        return Ok(false);
    }
    // An interrupted earlier attempt may have moved it. A vanished UID is complete.
    if connection
        .metadata(&[attempt.uid], cancel)
        .await?
        .is_empty()
    {
        return Ok(true);
    }
    connection
        .move_message(attempt.uid, &attempt.destination, cancel)
        .await?;
    Ok(true)
}

/// Copy at most 20 accepted messages to the Sent folder. Never resubmits anything.
///
/// # Errors
/// Returns static transport/storage failures; the copy stays pending for a later cycle.
pub async fn append_sent(
    store: &worker::Handle,
    connection: &mut Connection,
    account: &str,
    sent_folder: &str,
    cancel: &CancellationToken,
) -> Result<u32, Error> {
    let mut copied = 0;
    for _ in 0..20 {
        let Some(copy) = store.pending_sent_copy(account.into()).await? else {
            break;
        };
        // A crash after APPEND must not leave a second copy on the next run.
        connection.select(sent_folder, cancel).await?;
        if !connection.has_message_id(&copy.message_id, cancel).await? {
            connection
                .append(sent_folder, &copy.original, cancel)
                .await?;
        }
        store.finish_sent_copy(account.into(), copy.id).await?;
        copied += 1;
    }
    Ok(copied)
}

async fn perform(
    connection: &mut Connection,
    attempt: &Attempt,
    cancel: &CancellationToken,
) -> Result<Option<imap::Metadata>, imap::Error> {
    let selected = connection.select(&attempt.folder, cancel).await?;
    let validity = selected
        .uid_validity
        .and_then(NonZeroU32::new)
        .ok_or(imap::Error::Protocol)?;
    if validity != attempt.validity {
        return Ok(None);
    }
    connection
        .set_flag(attempt.uid, attempt.flag, attempt.desired, cancel)
        .await
}
