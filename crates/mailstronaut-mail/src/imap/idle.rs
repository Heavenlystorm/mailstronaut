//! RFC 2177 IDLE: a signal that a folder changed, never a source of mail data.

use core::time::Duration;

use async_imap::extensions::idle::IdleResponse;
use tokio_util::sync::CancellationToken;

use super::{Connection, Error, limited, protocol_error, read};

/// The folder state that reveals a change made while this connection was not idling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Position {
    uid_validity: Option<u32>,
    uid_next: Option<u32>,
    exists: u32,
}

/// Why one IDLE period ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Wake {
    /// The server reported something about the folder; reconcile to learn what.
    Changed,
    /// The renewal period passed, so the caller should reissue IDLE.
    Renew,
}

impl Connection {
    /// Open a folder read-only for watching. EXAMINE cannot change any message or
    /// flag, and flag writes stay refused until a later SELECT.
    ///
    /// # Errors
    /// Returns a redacted error and retires the stream on any failed operation.
    pub async fn examine(
        &mut self,
        mailbox: &str,
        cancellation: &CancellationToken,
    ) -> Result<Position, Error> {
        read::validate_mailbox(mailbox)?;
        self.permanent_flags = None;
        let mut session = self.session.take().ok_or(Error::Disconnected)?;
        let examined = limited(cancellation, async {
            session.examine(mailbox).await.map_err(protocol_error)
        })
        .await?;
        self.session = Some(session);
        Ok(Position {
            uid_validity: examined.uid_validity,
            uid_next: examined.uid_next,
            exists: examined.exists,
        })
    }

    /// Wait in IDLE until the examined folder changes or `renew` passes, then end it
    /// with DONE. The deadline is independent of keepalives: a server's periodic
    /// "still here" resets the library's own timeout, which would then never renew,
    /// and renewal is what notices a connection that died without closing.
    ///
    /// # Errors
    /// A lost connection or a failed DONE retires the stream; the caller reconnects.
    pub async fn idle(
        &mut self,
        renew: Duration,
        cancellation: &CancellationToken,
    ) -> Result<Wake, Error> {
        let session = self.session.take().ok_or(Error::Disconnected)?;
        let mut handle = session.idle();
        limited(cancellation, async {
            handle.init().await.map_err(protocol_error)
        })
        .await?;
        let wake = {
            let (wait, _stop) = handle.wait_with_timeout(renew);
            cancellation
                .run_until_cancelled(tokio::time::timeout(renew, wait))
                .await
                .ok_or(Error::Cancelled)?
        };
        let wake = match wake {
            Ok(Ok(IdleResponse::NewData(_))) => Wake::Changed,
            Ok(Ok(IdleResponse::Timeout)) | Err(_) => Wake::Renew,
            // Nothing here interrupts manually: the library reports the end of the
            // stream this way, so the server has closed the connection.
            Ok(Ok(IdleResponse::ManualInterrupt)) => return Err(Error::Disconnected),
            Ok(Err(error)) => return Err(protocol_error(error)),
        };
        let session = limited(cancellation, async {
            handle.done().await.map_err(protocol_error)
        })
        .await?;
        self.session = Some(session);
        Ok(wake)
    }
}
