//! Durable, account-local reminders independent of Seen/priority/server labels.

use super::{Error, Store};
use rusqlite::{OptionalExtension as _, params};
use uuid::Uuid;

pub(super) const SCHEMA:&str="
ALTER TABLE reminders ADD COLUMN account_id TEXT REFERENCES accounts(id);
ALTER TABLE reminders ADD COLUMN time_zone TEXT NOT NULL DEFAULT 'UTC';
ALTER TABLE reminders ADD COLUMN generation INTEGER NOT NULL DEFAULT 1 CHECK(generation>0);
ALTER TABLE reminders ADD COLUMN notification_id TEXT;
ALTER TABLE reminders ADD COLUMN attempt TEXT;
ALTER TABLE reminders ADD COLUMN delivered_at INTEGER;
ALTER TABLE reminders ADD COLUMN withdraw_pending INTEGER NOT NULL DEFAULT 0;
UPDATE reminders SET account_id=(SELECT account_id FROM messages WHERE messages.id=reminders.message_id)
 WHERE rowid=(SELECT MIN(r.rowid) FROM reminders r WHERE r.message_id=reminders.message_id);
CREATE UNIQUE INDEX one_message_reminder ON reminders(account_id,message_id) WHERE account_id IS NOT NULL;
CREATE INDEX reminder_due ON reminders(account_id,state,due_at,id);
";
/// Reminder lifecycle. Desktop acceptance is not proof of user presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    /// Waiting for its UTC instant, including overdue records while muted.
    Pending,
    /// One fenced notification call is in progress.
    Delivering,
    /// The desktop accepted the notification; entry remains visible in the app.
    Delivered,
    /// Delivery failed; explicit retry is required.
    Failed,
    /// Explicitly cancelled/dismissed.
    Cancelled,
    /// Legacy state or a missing target requires manual resolution.
    Unavailable,
}
impl State {
    fn parse(value: &str) -> Self {
        match value {
            "pending" => Self::Pending,
            "delivering" => Self::Delivering,
            "delivered" => Self::Delivered,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Unavailable,
        }
    }
}
/// Immutable local target and generation. Debug never emits message metadata.
#[derive(Clone, Eq, PartialEq)]
pub struct Reminder {
    /// Opaque local reminder identity.
    pub id: String,
    /// Stable configuration account ID.
    pub account: String,
    /// Immutable local message ID; may refer to a removed original.
    pub message: i64,
    /// Scheduled UTC Unix seconds.
    pub due_at: i64,
    /// Original IANA zone, preserved independently of later configuration changes.
    pub time_zone: String,
    /// Monotonic revision fencing old edits/actions/results.
    pub generation: i64,
    /// Durable lifecycle state.
    pub state: State,
    /// Stable application notification ID, allocated before desktop handoff.
    pub notification: String,
    /// Whether the same account still has the exact original target.
    pub available: bool,
    /// Cached subject for native display; never emitted by Debug.
    pub title: String,
}
impl core::fmt::Debug for Reminder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Reminder(<private target>)")
    }
}
/// Create-only or compare-and-swap rescheduling request.
pub struct Schedule {
    /// Receiving account, validated against the original message.
    pub account: String,
    /// Exact original message, never a subject/conversation guess.
    pub message: i64,
    /// UTC Unix seconds in the future.
    pub due_at: i64,
    /// User-selected IANA timezone.
    pub time_zone: String,
    /// Current wall-clock UTC seconds for validation.
    pub now: i64,
    /// None creates only; Some must match the current reminder generation.
    pub expected: Option<i64>,
}
/// Immutable desktop handoff claim; only its exact token can finish delivery.
#[derive(Clone)]
pub struct Attempt {
    /// Frozen local target/generation/notification identity.
    pub reminder: Reminder,
    token: String,
}
impl core::fmt::Debug for Attempt {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ReminderAttempt(<private target>)")
    }
}
const SELECT: &str = "SELECT r.id,r.account_id,r.message_id,r.due_at,r.time_zone,r.generation,r.state,r.notification_id,EXISTS(SELECT 1 FROM messages m WHERE m.account_id=r.account_id AND m.id=r.message_id),COALESCE((SELECT subject FROM messages m WHERE m.account_id=r.account_id AND m.id=r.message_id),'Message unavailable') FROM reminders r";
fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Reminder> {
    Ok(Reminder {
        id: row.get(0)?,
        account: row.get(1)?,
        message: row.get(2)?,
        due_at: row.get(3)?,
        time_zone: row.get(4)?,
        generation: row.get(5)?,
        state: State::parse(&row.get::<_, String>(6)?),
        notification: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
        available: row.get(8)?,
        title: row.get(9)?,
    })
}
impl Store {
    /// Return a bounded visible page. Disabled/unbound legacy accounts are excluded.
    /// # Errors
    /// Rejects excess account selections and storage errors.
    pub fn reminders(&self, accounts: &[String]) -> Result<Vec<Reminder>, Error> {
        if accounts.len() > 256 {
            return Err(Error::InvalidInput);
        }
        let sql = format!(
            "{SELECT} WHERE r.account_id IN (SELECT value FROM json_each(?1)) AND r.state!='cancelled' ORDER BY r.due_at,r.id LIMIT 100"
        );
        self.connection
            .prepare_cached(&sql)?
            .query_map([serde_json::to_string(accounts)?], read)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Error::from)
    }
    /// Find the exact reminder independently of the visible page.
    /// # Errors
    /// Returns storage errors; missing targets return None.
    pub fn reminder_for(&self, account: &str, message: i64) -> Result<Option<Reminder>, Error> {
        self.connection
            .query_row(
                &format!("{SELECT} WHERE r.account_id=?1 AND r.message_id=?2"),
                params![account, message],
                read,
            )
            .optional()
            .map_err(Error::from)
    }
    /// Resolve a current notification action without trusting its target payload.
    /// # Errors
    /// Rejects excessive scope and storage failures; stale/foreign actions return None.
    pub fn reminder_action(
        &self,
        accounts: &[String],
        notification: &str,
        generation: i64,
    ) -> Result<Option<Reminder>, Error> {
        if accounts.len() > 256 || notification.len() > 128 {
            return Err(Error::InvalidInput);
        }
        self.connection.query_row(&format!("{SELECT} WHERE r.account_id IN (SELECT value FROM json_each(?1)) AND r.notification_id=?2 AND r.generation=?3 AND r.state!='cancelled'"),params![serde_json::to_string(accounts)?,notification,generation],read).optional().map_err(Error::from)
    }
    /// Earliest pending instant, including records outside the visible page.
    /// # Errors
    /// Rejects excessive scope and storage failures.
    pub fn next_reminder(&self, accounts: &[String]) -> Result<Option<i64>, Error> {
        if accounts.len() > 256 {
            return Err(Error::InvalidInput);
        }
        self.connection.query_row("SELECT MIN(due_at) FROM reminders WHERE state='pending' AND account_id IN (SELECT value FROM json_each(?1))",[serde_json::to_string(accounts)?],|row|row.get(0)).map_err(Error::from)
    }
    /// Create/reschedule one message's stable reminder without changing mail flags.
    /// # Errors
    /// Rejects stale edits, invalid dates/zones and missing/foreign originals.
    pub fn schedule_reminder(&mut self, request: &Schedule) -> Result<Reminder, Error> {
        if request.due_at <= request.now
            || !(1..=253_402_300_799).contains(&request.due_at)
            || request.time_zone.parse::<chrono_tz::Tz>().is_err()
        {
            return Err(Error::InvalidInput);
        }
        let tx = self.connection.transaction()?;
        let valid: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE account_id=?1 AND id=?2)",
            params![request.account, request.message],
            |row| row.get(0),
        )?;
        if !valid {
            return Err(Error::MissingMessage);
        }
        let old: Option<(String, i64)> = tx
            .query_row(
                "SELECT id,generation FROM reminders WHERE account_id=?1 AND message_id=?2",
                params![request.account, request.message],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if old.as_ref().map(|(_, generation)| *generation) != request.expected {
            return Err(Error::OperationConflict);
        }
        let id = old
            .as_ref()
            .map_or_else(|| Uuid::new_v4().to_string(), |(id, _)| id.clone());
        let generation = old
            .map_or(Some(1), |(_, generation)| generation.checked_add(1))
            .ok_or(Error::InvalidInput)?;
        let notification = format!("reminder-{}", Uuid::new_v4());
        tx.execute("INSERT INTO reminders(id,message_id,due_at,state,account_id,time_zone,generation,notification_id) VALUES(?1,?2,?3,'pending',?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET due_at=excluded.due_at,state='pending',time_zone=excluded.time_zone,generation=excluded.generation,notification_id=COALESCE(reminders.notification_id,excluded.notification_id),attempt=NULL,delivered_at=NULL,withdraw_pending=CASE WHEN reminders.notification_id IS NULL THEN 0 ELSE 1 END",params![id,request.message,request.due_at,request.account,request.time_zone,generation,notification])?;
        tx.commit()?;
        self.reminder_for(&request.account, request.message)?
            .ok_or(Error::MissingMessage)
    }
    /// Cancel or retry the exact current revision, including unavailable originals.
    /// # Errors
    /// Rejects stale/cross-account edits; retry requires the original to exist.
    pub fn edit_reminder(
        &mut self,
        account: &str,
        id: &str,
        generation: i64,
        retry: bool,
    ) -> Result<(), Error> {
        let sql = if retry {
            "UPDATE reminders SET state='pending',generation=generation+1,attempt=NULL,delivered_at=NULL WHERE account_id=?1 AND id=?2 AND generation=?3 AND generation<9223372036854775807 AND EXISTS(SELECT 1 FROM messages m WHERE m.id=reminders.message_id AND m.account_id=reminders.account_id)"
        } else {
            "UPDATE reminders SET state='cancelled',generation=generation+1,attempt=NULL,withdraw_pending=(notification_id IS NOT NULL) WHERE account_id=?1 AND id=?2 AND generation=?3 AND generation<9223372036854775807"
        };
        if self
            .connection
            .execute(sql, params![account, id, generation])?
            != 1
        {
            return Err(Error::OperationConflict);
        }
        Ok(())
    }
    /// Claim the oldest due reminder inside enabled accounts before desktop I/O.
    /// # Errors
    /// Rejects excess scope and storage failures. Missing targets become unavailable.
    pub fn claim_reminder(
        &mut self,
        accounts: &[String],
        now: i64,
    ) -> Result<Option<Attempt>, Error> {
        if accounts.len() > 256 {
            return Err(Error::InvalidInput);
        }
        let tx = self.connection.transaction()?;
        let selected = serde_json::to_string(accounts)?;
        tx.execute("UPDATE reminders SET state='unavailable',attempt=NULL WHERE state='pending' AND account_id IN (SELECT value FROM json_each(?1)) AND NOT EXISTS(SELECT 1 FROM messages m WHERE m.account_id=reminders.account_id AND m.id=reminders.message_id)",[&selected])?;
        let record=tx.query_row(&format!("{SELECT} WHERE r.state='pending' AND r.withdraw_pending=0 AND r.due_at<=?2 AND r.account_id IN (SELECT value FROM json_each(?1)) ORDER BY r.due_at,r.id LIMIT 1"),params![selected,now],read).optional()?;
        let Some(mut reminder) = record else {
            tx.commit()?;
            return Ok(None);
        };
        if reminder.notification.is_empty() {
            reminder.notification = format!("reminder-{}", Uuid::new_v4());
        }
        let token = Uuid::new_v4().to_string();
        tx.execute(
            "UPDATE reminders SET state='delivering',attempt=?2,notification_id=?3 WHERE id=?1",
            params![reminder.id, token, reminder.notification],
        )?;
        reminder.state = State::Delivering;
        tx.commit()?;
        Ok(Some(Attempt { reminder, token }))
    }
    /// Commit only this exact desktop attempt. Failures remain visible for retry.
    /// # Errors
    /// Rejects late acknowledgments after cancel/reschedule or storage failure.
    pub fn finish_reminder(
        &self,
        attempt: &Attempt,
        accepted: bool,
        now: i64,
    ) -> Result<(), Error> {
        let state = if accepted { "delivered" } else { "failed" };
        if self.connection.execute("UPDATE reminders SET state=?5,delivered_at=?6,attempt=NULL WHERE account_id=?1 AND id=?2 AND generation=?3 AND attempt=?4 AND state='delivering'",params![attempt.reminder.account,attempt.reminder.id,attempt.reminder.generation,attempt.token,state,accepted.then_some(now)])?!=1{return Err(Error::OperationConflict);}
        Ok(())
    }
    /// Withdrawals survive crashes between reschedule/cancel and desktop removal.
    /// # Errors
    /// Rejects excessive scope and storage failures.
    pub fn reminder_withdrawals(&self, accounts: &[String]) -> Result<Vec<Reminder>, Error> {
        if accounts.len() > 256 {
            return Err(Error::InvalidInput);
        }
        self.connection.prepare_cached(&format!("{SELECT} WHERE r.withdraw_pending=1 AND r.account_id IN (SELECT value FROM json_each(?1)) ORDER BY r.due_at,r.id LIMIT 8"))?.query_map([serde_json::to_string(accounts)?],read)?.collect::<Result<Vec<_>,_>>().map_err(Error::from)
    }
    /// Acknowledge only this generation's completed withdrawal.
    /// # Errors
    /// Rejects stale acknowledgments and storage failures.
    pub fn withdrew_reminder(&self, record: &Reminder) -> Result<(), Error> {
        if self.connection.execute("UPDATE reminders SET withdraw_pending=0 WHERE account_id=?1 AND id=?2 AND generation=?3 AND notification_id=?4",params![record.account,record.id,record.generation,record.notification])?!=1{return Err(Error::OperationConflict);}
        Ok(())
    }
    /// Release a claim whose async waiter disappeared before receiving it.
    /// # Errors
    /// Rejects stale claims and storage failures.
    pub(crate) fn release_reminder(&self, attempt: &Attempt) -> Result<(), Error> {
        if self.connection.execute("UPDATE reminders SET state='pending',attempt=NULL WHERE id=?1 AND generation=?2 AND attempt=?3 AND state='delivering'",params![attempt.reminder.id,attempt.reminder.generation,attempt.token])?!=1{return Err(Error::OperationConflict);}
        Ok(())
    }
    /// Requeue interrupted attempts only after obtaining the exclusive profile lock.
    /// Stable notification IDs make possible post-acceptance retries replacements.
    /// # Errors
    /// Returns storage failures.
    pub(crate) fn recover_reminders(&self) -> Result<(), Error> {
        self.connection.execute(
            "UPDATE reminders SET state='pending',attempt=NULL WHERE state='delivering'",
            [],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
