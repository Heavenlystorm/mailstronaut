//! Durable archive, trash and move intentions bound to exact IMAP UID namespaces.

use core::num::NonZeroU32;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{OptionalExtension as _, Row, TransactionBehavior, params};

use super::{Error, Store};

/// Hide a message as soon as its move is queued, and show it again if the move fails.
pub(super) const SCHEMA: &str = "
CREATE TABLE pending_moves (
 id TEXT PRIMARY KEY,
 account_id TEXT NOT NULL REFERENCES accounts(id),
 message_id INTEGER NOT NULL,
 folder_id INTEGER NOT NULL,
 uidvalidity INTEGER NOT NULL,
 uid INTEGER NOT NULL,
 destination TEXT NOT NULL,
 context TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('pending','submitted','failed')),
 attempted INTEGER NOT NULL DEFAULT 0 CHECK(attempted IN (0,1)),
 attempt_id TEXT,
 UNIQUE(account_id,folder_id,uidvalidity,uid)
);
CREATE INDEX message_moves ON pending_moves(account_id,message_id,state);
DROP VIEW effective_memberships;
CREATE VIEW effective_memberships AS
 SELECT s.account_id,s.folder_id,s.uidvalidity,s.uid,s.message_id,
  COALESCE(seen.desired,s.seen) AS seen,COALESCE(starred.desired,s.starred) AS starred
 FROM memberships s
 LEFT JOIN pending_operations seen ON seen.account_id=s.account_id AND seen.folder_id=s.folder_id AND seen.uidvalidity=s.uidvalidity AND seen.uid=s.uid AND seen.message_id=s.message_id AND seen.kind='seen' AND seen.state IN ('pending','submitted','failed')
 LEFT JOIN pending_operations starred ON starred.account_id=s.account_id AND starred.folder_id=s.folder_id AND starred.uidvalidity=s.uidvalidity AND starred.uid=s.uid AND starred.message_id=s.message_id AND starred.kind='starred' AND starred.state IN ('pending','submitted','failed')
 WHERE NOT EXISTS(SELECT 1 FROM pending_moves m WHERE m.account_id=s.account_id AND m.folder_id=s.folder_id AND m.uidvalidity=s.uidvalidity AND m.uid=s.uid AND m.state IN ('pending','submitted'));
";

/// Holds a move back for its Undo window, whatever starts a sync pass during it.
pub(super) const HOLD_SCHEMA: &str =
    "ALTER TABLE pending_moves ADD COLUMN not_before INTEGER NOT NULL DEFAULT 0;";

/// A claimed move. The token fences a late callback from an earlier attempt.
#[derive(Clone, Debug)]
pub struct MoveAttempt {
    id: String,
    token: String,
    /// Source folder wire name.
    pub folder: String,
    /// Nonzero SELECT namespace required before moving.
    pub validity: NonZeroU32,
    /// Nonzero UID in that namespace, never a sequence number.
    pub uid: NonZeroU32,
    /// Destination folder wire name.
    pub destination: String,
}

fn read_attempt(row: &Row<'_>) -> rusqlite::Result<MoveAttempt> {
    Ok(MoveAttempt {
        id: row.get(0)?,
        token: uuid::Uuid::new_v4().to_string(),
        folder: row.get(1)?,
        validity: NonZeroU32::new(row.get(2)?).ok_or(rusqlite::Error::InvalidQuery)?,
        uid: NonZeroU32::new(row.get(3)?).ok_or(rusqlite::Error::InvalidQuery)?,
        destination: row.get(4)?,
    })
}

impl Store {
    /// Queue moving this original and the rest of its conversation out of `source`, or
    /// out of the inbox when `source` is None. Copies in other folders stay where they
    /// are, so moving a thread out of the inbox does not touch its Sent ancestors. No
    /// network I/O occurs, and the rows disappear from local queries immediately.
    ///
    /// # Errors
    /// Rejects unknown destinations, messages with nothing to move and stale context.
    pub fn queue_move(
        &mut self,
        account: &str,
        message: i64,
        source: Option<i64>,
        destination: &str,
        context: &str,
    ) -> Result<u32, Error> {
        self.queue_move_after(account, message, source, destination, context, 0)
    }

    /// Like [`Self::queue_move`], held back until a moment: that is the Undo window, and no
    /// sync pass may claim the move during it, whatever started that pass.
    ///
    /// # Errors
    /// As [`Self::queue_move`].
    pub fn queue_move_after(
        &mut self,
        account: &str,
        message: i64,
        source: Option<i64>,
        destination: &str,
        context: &str,
        not_before: i64,
    ) -> Result<u32, Error> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let bound: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE id=?1 AND imap_context=?2)",
            params![account, context],
            |row| row.get(0),
        )?;
        if !bound {
            return Err(Error::StaleMailbox);
        }
        let target: Option<i64> = tx
            .query_row(
                "SELECT id FROM folders WHERE account_id=?1 AND name=?2",
                params![account, destination],
                |row| row.get(0),
            )
            .optional()?;
        let target = target.ok_or(Error::InvalidInput)?;
        let selected = {
            let mut query = tx.prepare(
                "SELECT DISTINCT s.message_id,s.folder_id,s.uidvalidity,s.uid
                 FROM memberships s JOIN folders f ON f.id=s.folder_id AND f.account_id=s.account_id AND f.uidvalidity=s.uidvalidity
                 WHERE s.account_id=?1 AND s.folder_id!=?4 AND (?3 IS NULL AND f.is_inbox=1 OR s.folder_id=?3)
                  AND (s.message_id=?2 OR s.message_id IN (
                   SELECT other.message_id FROM conversation_members mine
                   JOIN conversation_members other ON other.account_id=mine.account_id AND other.conversation_id=mine.conversation_id
                   JOIN accounts a ON a.id=mine.account_id AND a.projected_thread_revision=a.thread_revision
                   WHERE mine.account_id=?1 AND mine.message_id=?2))
                 LIMIT 257",
            )?;
            query
                .query_map(params![account, message, source, target], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, u32>(2)?,
                        row.get::<_, u32>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        if selected.is_empty() {
            return Err(Error::MissingMessage);
        }
        if selected.len() > 256 {
            return Err(Error::InvalidInput);
        }
        for (message, folder, validity, uid) in &selected {
            tx.execute(
                "INSERT INTO pending_moves(id,account_id,message_id,folder_id,uidvalidity,uid,destination,context,state,not_before)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'pending',?9)
                 ON CONFLICT(account_id,folder_id,uidvalidity,uid) DO UPDATE SET destination=excluded.destination,context=excluded.context,not_before=excluded.not_before,state=CASE WHEN pending_moves.state='submitted' THEN 'submitted' ELSE 'pending' END",
                params![uuid::Uuid::new_v4().to_string(), account, message, folder, validity, uid, destination, context, not_before],
            )?;
        }
        let count = u32::try_from(selected.len()).map_err(|_| Error::InvalidInput)?;
        tx.commit()?;
        Ok(count)
    }

    /// Cancel this conversation's moves if none has reached the server yet.
    ///
    /// # Errors
    /// Rejects moves already attempted, since the message may be in its destination.
    pub fn undo_move(&mut self, account: &str, message: i64) -> Result<(), Error> {
        const SCOPE: &str = "account_id=?1 AND (message_id=?2 OR message_id IN (
           SELECT other.message_id FROM conversation_members mine
           JOIN conversation_members other ON other.account_id=mine.account_id AND other.conversation_id=mine.conversation_id
           WHERE mine.account_id=?1 AND mine.message_id=?2))";
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let attempted: bool = tx.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM pending_moves WHERE attempted=1 AND {SCOPE})"),
            params![account, message],
            |row| row.get(0),
        )?;
        if attempted {
            return Err(Error::OperationConflict);
        }
        let removed = tx.execute(
            &format!("DELETE FROM pending_moves WHERE state='pending' AND {SCOPE}"),
            params![account, message],
        )?;
        if removed == 0 {
            return Err(Error::OperationConflict);
        }
        tx.commit()?;
        Ok(())
    }

    /// Claim the next move, committing its token before the network handoff. A move
    /// whose UID namespace or configuration changed is dropped: its message reappears
    /// through normal synchronization rather than moving the wrong mail.
    ///
    /// # Errors
    /// Rejects a concurrent claim and storage failures.
    pub fn begin_move(
        &mut self,
        account: &str,
        context: &str,
    ) -> Result<Option<MoveAttempt>, Error> {
        let now = i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        )
        .unwrap_or(i64::MAX);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM pending_moves WHERE account_id=?1 AND state IN ('pending','failed') AND (context!=?2 OR NOT EXISTS(
               SELECT 1 FROM memberships s JOIN folders f ON f.id=s.folder_id AND f.account_id=s.account_id
               WHERE s.account_id=pending_moves.account_id AND s.folder_id=pending_moves.folder_id AND s.uidvalidity=pending_moves.uidvalidity AND s.uid=pending_moves.uid AND s.message_id=pending_moves.message_id AND f.uidvalidity=s.uidvalidity))",
            params![account, context],
        )?;
        let busy: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM pending_moves WHERE account_id=?1 AND state='submitted')",
            [account],
            |row| row.get(0),
        )?;
        if busy {
            return Err(Error::OperationConflict);
        }
        let selected = tx
            .query_row(
                "SELECT p.id,f.name,p.uidvalidity,p.uid,p.destination FROM pending_moves p
                 JOIN folders f ON f.id=p.folder_id AND f.account_id=p.account_id
                 WHERE p.account_id=?1 AND p.state='pending' AND p.not_before<=?2 ORDER BY p.rowid LIMIT 1",
                params![account, now],
                read_attempt,
            )
            .optional()?;
        if let Some(attempt) = &selected {
            tx.execute(
                "UPDATE pending_moves SET state='submitted',attempted=1,attempt_id=?2 WHERE id=?1",
                params![attempt.id, attempt.token],
            )?;
        }
        tx.commit()?;
        Ok(selected)
    }

    /// Record the server's answer for the exact attempt. Success forgets the source
    /// copy; the destination copy arrives through normal synchronization. A failure
    /// shows the message in its source folder again, for an explicit retry.
    ///
    /// # Errors
    /// Rejects a stale attempt token and storage failures.
    pub fn finish_move(&mut self, attempt: &MoveAttempt, moved: bool) -> Result<(), Error> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let source: Option<(String, i64, u32, u32)> = tx
            .query_row(
                "SELECT account_id,folder_id,uidvalidity,uid FROM pending_moves WHERE id=?1 AND attempt_id=?2 AND state='submitted'",
                params![attempt.id, attempt.token],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let (account, folder, validity, uid) = source.ok_or(Error::OperationConflict)?;
        if moved {
            tx.execute(
                "DELETE FROM memberships WHERE account_id=?1 AND folder_id=?2 AND uidvalidity=?3 AND uid=?4",
                params![account, folder, validity, uid],
            )?;
            tx.execute("DELETE FROM pending_moves WHERE id=?1", [&attempt.id])?;
        } else {
            tx.execute(
                "UPDATE pending_moves SET state='failed',attempt_id=NULL WHERE id=?1",
                [&attempt.id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Give up on a move this server can never do: the message goes back to its folder,
    /// and no later pass tries again.
    ///
    /// # Errors
    /// Rejects a stale attempt token and storage failures.
    pub fn abandon_move(&mut self, attempt: &MoveAttempt) -> Result<(), Error> {
        let changed = self.connection.execute(
            "DELETE FROM pending_moves WHERE id=?1 AND attempt_id=?2 AND state='submitted'",
            params![attempt.id, attempt.token],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(Error::OperationConflict)
        }
    }

    /// Explicitly requeue failed moves for this unchanged account.
    ///
    /// # Errors
    /// Returns storage failures.
    pub fn retry_moves(&mut self, account: &str) -> Result<(), Error> {
        self.connection.execute(
            "UPDATE pending_moves SET state='pending' WHERE account_id=?1 AND state='failed'",
            [account],
        )?;
        Ok(())
    }

    /// An interrupted move may have happened; the retry checks the UID still exists.
    pub(crate) fn recover_moves(&self) -> Result<(), Error> {
        self.connection.execute(
            "UPDATE pending_moves SET state='pending',attempt_id=NULL WHERE state='submitted'",
            [],
        )?;
        Ok(())
    }
}
