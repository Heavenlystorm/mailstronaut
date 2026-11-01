//! Moving and appending whole messages through the existing IMAP library.

use core::num::NonZeroU32;

use futures_util::TryStreamExt as _;
use tokio_util::sync::CancellationToken;

use super::{Connection, Error, limited, protocol_error, read::validate_mailbox};

impl Connection {
    /// Move one UID out of the selected mailbox. Without MOVE, this copies then
    /// removes that exact UID with UIDPLUS `UID EXPUNGE`. A server with neither is
    /// refused: a plain EXPUNGE would also remove other clients' deleted messages.
    ///
    /// # Errors
    /// Returns a redacted protocol or capability error and retires a failed session.
    pub async fn move_message(
        &mut self,
        uid: NonZeroU32,
        destination: &str,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        validate_mailbox(destination)?;
        let direct = self.supports("MOVE");
        if !direct && !self.supports("UIDPLUS") {
            return Err(Error::MoveUnavailable);
        }
        let mut session = self.session.take().ok_or(Error::Disconnected)?;
        let set = uid.to_string();
        limited(cancel, async {
            if direct {
                return session
                    .uid_mv(&set, destination)
                    .await
                    .map_err(protocol_error);
            }
            session
                .uid_copy(&set, destination)
                .await
                .map_err(protocol_error)?;
            {
                let mut responses = session
                    .uid_store(&set, "+FLAGS.SILENT (\\Deleted)")
                    .await
                    .map_err(protocol_error)?;
                while responses
                    .try_next()
                    .await
                    .map_err(protocol_error)?
                    .is_some()
                {}
            }
            let mut expunged =
                core::pin::pin!(session.uid_expunge(&set).await.map_err(protocol_error)?);
            while expunged.try_next().await.map_err(protocol_error)?.is_some() {}
            Ok(())
        })
        .await?;
        self.session = Some(session);
        Ok(())
    }

    /// Whether the selected mailbox already holds this exact Message-ID.
    ///
    /// # Errors
    /// Rejects an identifier that cannot be quoted safely; retires a failed session.
    pub async fn has_message_id(
        &mut self,
        message_id: &str,
        cancel: &CancellationToken,
    ) -> Result<bool, Error> {
        if !crate::compose::valid_id(message_id) || message_id.contains('"') {
            return Err(Error::Configuration);
        }
        let mut session = self.session.take().ok_or(Error::Disconnected)?;
        let found = limited(cancel, async {
            session
                .uid_search(format!("HEADER Message-ID \"<{message_id}>\""))
                .await
                .map_err(protocol_error)
        })
        .await?;
        self.session = Some(session);
        Ok(!found.is_empty())
    }

    /// Store exactly these bytes as a read message in `mailbox`.
    ///
    /// # Errors
    /// Returns a redacted protocol error and retires a failed session.
    pub async fn append(
        &mut self,
        mailbox: &str,
        original: &[u8],
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        validate_mailbox(mailbox)?;
        let mut session = self.session.take().ok_or(Error::Disconnected)?;
        limited(cancel, async {
            session
                .append(mailbox, Some("(\\Seen)"), None, original)
                .await
                .map_err(protocol_error)
        })
        .await?;
        self.session = Some(session);
        Ok(())
    }
}
