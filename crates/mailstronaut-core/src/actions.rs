//! Bounded foreground application of durable, idempotent UID flag intentions.

use core::num::NonZeroU32;

use mailstronaut_mail::imap::{self, Connection};
use tokio_util::sync::CancellationToken;

use crate::{store::flags::Attempt, sync::Error, worker};

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
