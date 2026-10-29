//! UID-only read/star updates through the existing IMAP library.

use core::num::NonZeroU32;

use futures_util::TryStreamExt as _;
use tokio_util::sync::CancellationToken;

use super::{Connection, Error, Metadata, limited, protocol_error};

/// Supported idempotent system flag assignments. No arbitrary command is accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlagChange {
    /// IMAP Seen: opening/fetching alone still never changes it.
    Seen,
    /// IMAP Flagged: a user-selected star.
    Starred,
}

impl Connection {
    /// Assign exactly one system flag to one nonzero UID in the selected mailbox.
    /// The caller must first validate SELECT's UIDVALIDITY against the durable
    /// claim. A fresh UID FETCH confirms both flags; no sequence identity escapes.
    /// Other flags/keywords are preserved using +/-FLAGS.SILENT, not replacement.
    ///
    /// # Errors
    /// Retires a failed/cancelled session. An expunged UID returns None. A missing
    /// confirmation is never treated as successful merely because STORE said OK.
    pub async fn set_flag(
        &mut self,
        uid: NonZeroU32,
        flag: FlagChange,
        desired: bool,
        cancel: &CancellationToken,
    ) -> Result<Option<Metadata>, Error> {
        if self.session.is_none() {
            return Err(Error::Disconnected);
        }
        // The library represents both absent and explicitly empty PERMANENTFLAGS
        // with an empty vector. Require positive support, rather than confirming
        // a possible session-only change as a durable server assignment.
        let allowed = self
            .permanent_flags
            .is_some_and(|(seen, starred)| match flag {
                FlagChange::Seen => seen,
                FlagChange::Starred => starred,
            });
        if !allowed {
            return Err(Error::FlagUnavailable);
        }
        let query = match (flag, desired) {
            (FlagChange::Seen, true) => "+FLAGS.SILENT (\\Seen)",
            (FlagChange::Seen, false) => "-FLAGS.SILENT (\\Seen)",
            (FlagChange::Starred, true) => "+FLAGS.SILENT (\\Flagged)",
            (FlagChange::Starred, false) => "-FLAGS.SILENT (\\Flagged)",
        };
        let mut session = self.session.take().ok_or(Error::Disconnected)?;
        limited(cancel, async {
            let mut responses = session
                .uid_store(uid.to_string(), query)
                .await
                .map_err(protocol_error)?;
            let mut count = 0;
            while let Some(fetch) = responses.try_next().await.map_err(protocol_error)? {
                count += 1;
                if count > 200 || fetch.body().is_some() {
                    return Err(Error::ResponseLimit);
                }
            }
            Ok(())
        })
        .await?;
        self.session = Some(session);
        let mut found = self.metadata(&[uid], cancel).await?;
        let observed = found.pop();
        if let Some(observed) = &observed {
            let actual = match flag {
                FlagChange::Seen => observed.seen,
                FlagChange::Starred => observed.starred,
            };
            if actual != desired {
                self.session = None;
                return Err(Error::Protocol);
            }
        }
        Ok(observed)
    }
}
