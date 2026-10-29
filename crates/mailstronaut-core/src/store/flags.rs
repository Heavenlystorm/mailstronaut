//! Durable, revisioned read/star intentions bound to exact IMAP UID namespaces.

use core::num::NonZeroU32;

use mailstronaut_config::Account;
use mailstronaut_mail::imap::{FlagChange, Metadata};
use rusqlite::{OptionalExtension as _, Transaction, TransactionBehavior, params};
use sha2::{Digest as _, Sha256};

use super::{Error, Store};

pub(super) const SCHEMA: &str = "
ALTER TABLE accounts ADD COLUMN imap_context TEXT;
ALTER TABLE pending_operations ADD COLUMN message_id INTEGER;
ALTER TABLE pending_operations ADD COLUMN kind TEXT CHECK(kind IN ('seen','starred'));
ALTER TABLE pending_operations ADD COLUMN desired INTEGER CHECK(desired IN (0,1));
ALTER TABLE pending_operations ADD COLUMN revision INTEGER NOT NULL DEFAULT 1;
ALTER TABLE pending_operations ADD COLUMN attempt_id TEXT;
ALTER TABLE pending_operations ADD COLUMN attempt_revision INTEGER;
ALTER TABLE pending_operations ADD COLUMN attempt_desired INTEGER CHECK(attempt_desired IN (0,1));
ALTER TABLE pending_operations ADD COLUMN context TEXT;
ALTER TABLE pending_operations ADD COLUMN error TEXT;
ALTER TABLE pending_operations ADD COLUMN attempted INTEGER NOT NULL DEFAULT 0 CHECK(attempted IN (0,1));
CREATE UNIQUE INDEX one_flag_intent ON pending_operations(account_id,folder_id,uidvalidity,uid,kind);
CREATE UNIQUE INDEX one_flag_attempt ON pending_operations(account_id) WHERE kind IS NOT NULL AND state='submitted';
CREATE INDEX message_flag_intents ON pending_operations(account_id,message_id,state);
";

/// A claimed idempotent flag operation. The attempt token fences late callbacks.
#[derive(Clone, Debug)]
pub struct Attempt {
    id: String,
    token: String,
    revision: i64,
    /// Configuration account ID, never inferred from a message header.
    pub account: String,
    /// Exact server mailbox wire name.
    pub folder: String,
    /// Nonzero SELECT namespace required before any STORE.
    pub validity: NonZeroU32,
    /// Nonzero UID in this namespace, never a sequence number.
    pub uid: NonZeroU32,
    /// The only permitted system flag to change.
    pub flag: FlagChange,
    /// Frozen value for this attempt, unaffected by later local edits.
    pub desired: bool,
}

/// Visible counts distinguish queued/failed work from retired server identities.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Counts {
    /// Pending or currently submitted intentions.
    pub pending: u32,
    /// Network/server failures requiring explicit retry.
    pub failed: u32,
    /// Retired configuration, UID namespace or expunged membership.
    pub conflicts: u32,
}

/// Bind UID operations to the configured server, principal and authentication
/// metadata. It contains no credential value and is not an authorization token.
///
/// # Errors
/// Rejects missing endpoints or invalid public serialization.
pub fn context(account: &Account) -> Result<String, Error> {
    let endpoint = account.incoming_endpoint().ok_or(Error::InvalidInput)?;
    let public = serde_json::to_vec(&(
        &account.address,
        &account.user_name,
        endpoint,
        &account.auth,
        &account.tls,
    ))?;
    Ok(format!("{:x}", Sha256::digest(public)))
}

impl Store {
    /// Bind cached UID namespaces to an authenticated account configuration.
    /// On a change (including legacy unknown provenance), retire old operations
    /// and memberships, retaining originals and all user-owned message state.
    /// Call only after connecting/authenticating the supplied configuration.
    ///
    /// # Errors
    /// Rejects unknown accounts, invalid contexts and storage failures.
    pub fn bind_mailbox_context(&mut self, account: &str, context: &str) -> Result<bool, Error> {
        validate_context(context)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<Option<String>> = tx
            .query_row(
                "SELECT imap_context FROM accounts WHERE id=?1",
                [account],
                |row| row.get(0),
            )
            .optional()?;
        let previous = previous.ok_or(Error::InvalidInput)?;
        let changed = previous.as_deref() != Some(context);
        if changed {
            tx.execute("UPDATE pending_operations SET state='conflict',attempt_id=NULL,attempt_revision=NULL,attempt_desired=NULL,error='identity_changed' WHERE account_id=?1 AND kind IS NOT NULL AND state IN ('pending','submitted','failed')",[account])?;
            tx.execute("DELETE FROM memberships WHERE account_id=?1", [account])?;
            // Provider IDs belong to that authenticated mailbox too. Reusing
            // one on a different server/principal must not replace an old
            // original or transfer its reminders/read history to unrelated mail.
            tx.execute("DELETE FROM provider_ids WHERE account_id=?1", [account])?;
            tx.execute(
                "UPDATE folders SET uidvalidity=NULL WHERE account_id=?1",
                [account],
            )?;
            tx.execute(
                "UPDATE accounts SET imap_context=?2 WHERE id=?1",
                params![account, context],
            )?;
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Queue/coalesce a read/star intention for every inbox copy of this canonical
    /// message. Local inbox queries overlay it immediately, including offline.
    /// No network I/O occurs and another account can never acquire its UIDs.
    ///
    /// # Errors
    /// Rejects missing/oversized targets, invalid context and stale storage.
    pub fn set_flag(
        &mut self,
        account: &str,
        message: i64,
        flag: FlagChange,
        desired: bool,
        context: &str,
    ) -> Result<(), Error> {
        validate_context(context)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !bound(&tx, account, context)? {
            return Err(Error::StaleMailbox);
        }
        let targets = {
            let mut query = tx.prepare("SELECT s.folder_id,s.uidvalidity,s.uid FROM memberships s JOIN folders f ON f.id=s.folder_id AND f.account_id=s.account_id WHERE s.account_id=?1 AND s.message_id=?2 AND f.is_inbox=1 AND f.uidvalidity=s.uidvalidity LIMIT 257")?;
            query
                .query_map(params![account, message], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, u32>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        if targets.is_empty() {
            return Err(Error::MissingMessage);
        }
        if targets.len() > 256 {
            return Err(Error::InvalidInput);
        }
        for (folder, validity, uid) in targets {
            if validity == 0 || uid == 0 {
                return Err(Error::StaleMailbox);
            }
            // A new intention supersedes old failures/conflicts, but never changes
            // the immutable snapshot of an already running STORE.
            tx.execute("INSERT INTO pending_operations(id,account_id,folder_id,uidvalidity,uid,state,message_id,kind,desired,context) VALUES(?1,?2,?3,?4,?5,'pending',?6,?7,?8,?9) ON CONFLICT(account_id,folder_id,uidvalidity,uid,kind) DO UPDATE SET attempted=CASE WHEN pending_operations.context!=excluded.context OR pending_operations.message_id!=excluded.message_id THEN 0 ELSE pending_operations.attempted END, message_id=excluded.message_id, desired=excluded.desired, revision=pending_operations.revision+1, context=excluded.context, error=NULL, state=CASE WHEN pending_operations.state='submitted' THEN 'submitted' ELSE 'pending' END",params![uuid::Uuid::new_v4().to_string(),account,folder,validity,uid,message,kind(flag),desired,context])?;
        }
        if flag == FlagChange::Seen && desired {
            tx.execute(
                "UPDATE messages SET read_cleared=1 WHERE account_id=?1 AND id=?2",
                params![account, message],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Undo this message's local flag edits only before any target was attempted.
    /// Read-cleared priority history remains monotonic. Failed/in-flight writes
    /// need reconciliation, and cannot be represented as an unsent undo.
    ///
    /// # Errors
    /// Rejects absent/already-attempted work and storage failures.
    pub fn undo_flags(&mut self, account: &str, message: i64) -> Result<(), Error> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let attempted:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM pending_operations WHERE account_id=?1 AND message_id=?2 AND kind IS NOT NULL AND state IN ('pending','submitted','failed') AND attempted=1)",params![account,message],|row|row.get(0))?;
        if attempted {
            return Err(Error::OperationConflict);
        }
        let changed=tx.execute("DELETE FROM pending_operations WHERE account_id=?1 AND message_id=?2 AND kind IS NOT NULL AND state='pending' AND attempted=0",params![account,message])?;
        if changed == 0 {
            return Err(Error::OperationConflict);
        }
        tx.commit()?;
        Ok(())
    }

    /// Claim one queued operation and commit its token before network handoff.
    /// Stale configuration/expunged/changed namespace targets become conflicts.
    /// Only one caller may have a submitted flag operation per account.
    ///
    /// # Errors
    /// Rejects invalid context, concurrent claims and storage failures.
    pub fn begin_flag(&mut self, account: &str, context: &str) -> Result<Option<Attempt>, Error> {
        validate_context(context)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        retire(&tx, account, context)?;
        let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM pending_operations WHERE account_id=?1 AND kind IS NOT NULL AND state='submitted')",[account],|row|row.get(0))?;
        if busy {
            return Err(Error::OperationConflict);
        }
        let selected = tx.query_row(
            "SELECT p.id,p.revision,p.account_id,f.name,p.uidvalidity,p.uid,p.kind,p.desired FROM pending_operations p JOIN folders f ON f.id=p.folder_id AND f.account_id=p.account_id WHERE p.account_id=?1 AND p.kind IS NOT NULL AND p.state='pending' ORDER BY p.rowid LIMIT 1",
            [account], read_attempt,
        ).optional()?;
        if let Some(attempt) = &selected {
            tx.execute("UPDATE pending_operations SET state='submitted',attempted=1,attempt_id=?2,attempt_revision=revision,attempt_desired=desired WHERE id=?1",params![attempt.id,attempt.token])?;
        }
        tx.commit()?;
        Ok(selected)
    }

    /// Confirm the matching attempt with a fresh UID FETCH observation. A stale
    /// acknowledgment cannot delete or overwrite a newer local intention.
    ///
    /// # Errors
    /// Rejects stale attempts, missing/mismatched confirmations and storage errors.
    pub fn confirm_flag(&mut self, attempt: &Attempt, observed: &Metadata) -> Result<(), Error> {
        if observed.uid != attempt.uid
            || match attempt.flag {
                FlagChange::Seen => observed.seen,
                FlagChange::Starred => observed.starred,
            } != attempt.desired
        {
            return Err(Error::InvalidInput);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = check_attempt(&tx, attempt)?;
        let changed=tx.execute("UPDATE memberships SET seen=?4,starred=?5 WHERE account_id=?1 AND folder_id=(SELECT folder_id FROM pending_operations WHERE id=?6) AND uidvalidity=?2 AND uid=?3 AND message_id=(SELECT message_id FROM pending_operations WHERE id=?6) AND EXISTS(SELECT 1 FROM folders f WHERE f.id=memberships.folder_id AND f.account_id=?1 AND f.uidvalidity=?2)",params![attempt.account,attempt.validity.get(),attempt.uid.get(),observed.seen,observed.starred,attempt.id])?;
        if changed != 1 {
            return Err(Error::StaleMailbox);
        }
        if observed.seen {
            tx.execute("UPDATE messages SET read_cleared=1 WHERE account_id=?1 AND id=(SELECT message_id FROM pending_operations WHERE id=?2)",params![attempt.account,attempt.id])?;
        }
        if current == attempt.revision {
            tx.execute("DELETE FROM pending_operations WHERE id=?1", [&attempt.id])?;
        } else {
            tx.execute("UPDATE pending_operations SET state='pending',attempt_id=NULL,attempt_revision=NULL,attempt_desired=NULL,error=NULL WHERE id=?1",[&attempt.id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Retain a failed or retired intention without implicitly retrying it. Flag
    /// STORE is idempotent, but a lost acknowledgment can already have applied it.
    ///
    /// # Errors
    /// Rejects stale callbacks and storage errors.
    pub fn fail_flag(&mut self, attempt: &Attempt, conflict: bool) -> Result<(), Error> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = check_attempt(&tx, attempt)?;
        let state = if conflict {
            "conflict"
        } else if current != attempt.revision {
            "pending"
        } else {
            "failed"
        };
        tx.execute("UPDATE pending_operations SET state=?2,error=?3,attempt_id=NULL,attempt_revision=NULL,attempt_desired=NULL WHERE id=?1",params![attempt.id,state,if conflict {"identity_changed"} else {"not_confirmed"}])?;
        tx.commit()?;
        Ok(())
    }

    /// Explicitly retry failed idempotent flags for this unchanged account. This
    /// never touches SMTP/outbox state. Retired namespaces remain conflicts.
    ///
    /// # Errors
    /// Rejects invalid context or unavailable storage.
    pub fn retry_flags(&mut self, account: &str, context: &str) -> Result<(), Error> {
        validate_context(context)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        retire(&tx, account, context)?;
        tx.execute("UPDATE pending_operations SET state='pending',error=NULL WHERE account_id=?1 AND kind IS NOT NULL AND state='failed'",[account])?;
        tx.commit()?;
        Ok(())
    }

    /// Query counts for enabled accounts only, with no header/body content.
    ///
    /// # Errors
    /// Rejects excessive account selections and storage failures.
    pub fn flag_counts(&self, accounts: &[String]) -> Result<Counts, Error> {
        if accounts.len() > 256 {
            return Err(Error::InvalidInput);
        }
        let selected = serde_json::to_string(accounts)?;
        self.connection.query_row("SELECT COALESCE(SUM(state IN ('pending','submitted')),0),COALESCE(SUM(state='failed'),0),COALESCE(SUM(state='conflict'),0) FROM pending_operations WHERE kind IS NOT NULL AND account_id IN (SELECT value FROM json_each(?1))",[selected],|row|Ok(Counts{pending:row.get(0)?,failed:row.get(1)?,conflicts:row.get(2)?})).map_err(Error::from)
    }

    /// Only the exclusive application owner recovers interrupted flag attempts.
    /// Repeating the same UID flag assignment is idempotent; SMTP recovery keeps
    /// its separate delivery-unknown behavior.
    pub(crate) fn recover_flags(&self) -> Result<(), Error> {
        self.connection.execute("UPDATE pending_operations SET state='pending',attempt_id=NULL,attempt_revision=NULL,attempt_desired=NULL WHERE kind IS NOT NULL AND state='submitted'",[])?;
        Ok(())
    }
}

const fn kind(flag: FlagChange) -> &'static str {
    match flag {
        FlagChange::Seen => "seen",
        FlagChange::Starred => "starred",
    }
}
fn read_attempt(row: &rusqlite::Row<'_>) -> rusqlite::Result<Attempt> {
    let flag = match row.get::<_, String>(6)?.as_str() {
        "seen" => FlagChange::Seen,
        "starred" => FlagChange::Starred,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(Attempt {
        id: row.get(0)?,
        token: uuid::Uuid::new_v4().to_string(),
        revision: row.get(1)?,
        account: row.get(2)?,
        folder: row.get(3)?,
        validity: NonZeroU32::new(row.get(4)?).ok_or(rusqlite::Error::InvalidQuery)?,
        uid: NonZeroU32::new(row.get(5)?).ok_or(rusqlite::Error::InvalidQuery)?,
        flag,
        desired: row.get(7)?,
    })
}
fn validate_context(context: &str) -> Result<(), Error> {
    if context.len() != 64 || !context.bytes().all(|value| value.is_ascii_hexdigit()) {
        return Err(Error::InvalidInput);
    }
    Ok(())
}
fn retire(tx: &Transaction<'_>, account: &str, context: &str) -> Result<(), Error> {
    tx.execute("UPDATE pending_operations SET state='conflict',attempt_id=NULL,attempt_revision=NULL,attempt_desired=NULL,error='identity_changed' WHERE account_id=?1 AND kind IS NOT NULL AND state IN ('pending','failed') AND (context!=?2 OR NOT EXISTS(SELECT 1 FROM accounts a WHERE a.id=?1 AND a.imap_context=?2) OR NOT EXISTS(SELECT 1 FROM memberships s JOIN folders f ON f.id=s.folder_id AND f.account_id=s.account_id WHERE s.account_id=pending_operations.account_id AND s.folder_id=pending_operations.folder_id AND s.uidvalidity=pending_operations.uidvalidity AND s.uid=pending_operations.uid AND s.message_id=pending_operations.message_id AND f.uidvalidity=s.uidvalidity))",params![account,context])?;
    Ok(())
}
fn check_attempt(tx: &Transaction<'_>, attempt: &Attempt) -> Result<i64, Error> {
    tx.query_row("SELECT revision FROM pending_operations WHERE id=?1 AND account_id=?2 AND state='submitted' AND attempt_id=?3 AND attempt_revision=?4 AND attempt_desired=?5 AND uidvalidity=?6 AND uid=?7 AND kind=?8",params![attempt.id,attempt.account,attempt.token,attempt.revision,attempt.desired,attempt.validity.get(),attempt.uid.get(),kind(attempt.flag)],|row|row.get(0)).optional()?.ok_or(Error::OperationConflict)
}

fn bound(tx: &Transaction<'_>, account: &str, context: &str) -> Result<bool, Error> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE id=?1 AND imap_context=?2)",
        params![account, context],
        |row| row.get(0),
    )
    .map_err(Error::from)
}
