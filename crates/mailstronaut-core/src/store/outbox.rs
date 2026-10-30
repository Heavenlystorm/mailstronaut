//! Transactional editable drafts, frozen submission intent and attempt fencing.

pub mod attachments;

use std::time::{SystemTime, UNIX_EPOCH};

use mailstronaut_mail::{
    compose::{Content, Frozen},
    smtp::Outcome,
};
use rusqlite::{OptionalExtension as _, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::{Error, Store, write_blob};

pub(super) const SCHEMA:&str="
ALTER TABLE drafts ADD COLUMN account_id TEXT REFERENCES accounts(id);
ALTER TABLE drafts ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE drafts ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE drafts ADD COLUMN closed INTEGER NOT NULL DEFAULT 0 CHECK(closed IN (0,1));
ALTER TABLE outbox ADD COLUMN account_id TEXT REFERENCES accounts(id);
ALTER TABLE outbox ADD COLUMN draft_id TEXT REFERENCES drafts(id);
ALTER TABLE outbox ADD COLUMN envelope TEXT;
ALTER TABLE outbox ADD COLUMN message_id TEXT;
ALTER TABLE outbox ADD COLUMN blob_hash TEXT;
ALTER TABLE outbox ADD COLUMN attempt_id TEXT;
ALTER TABLE outbox ADD COLUMN last_error TEXT;
ALTER TABLE outbox ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE outbox ADD COLUMN sent_copy TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE outbox ADD COLUMN subject TEXT NOT NULL DEFAULT '';
ALTER TABLE outbox ADD COLUMN uncertain INTEGER NOT NULL DEFAULT 0 CHECK(uncertain IN (0,1));
UPDATE outbox SET uncertain=1 WHERE phase='delivery_unknown';
CREATE INDEX editable_drafts ON drafts(account_id,closed,updated_at DESC);
CREATE INDEX account_outbox ON outbox(account_id,updated_at DESC);
CREATE UNIQUE INDEX one_live_draft_submission ON outbox(draft_id) WHERE phase IN ('pending','submitting','delivery_unknown','accepted');
";

/// Maximum encoded draft JSON, separate from the original MIME byte budget.
pub const MAX_DRAFT_BYTES: usize = 1024 * 1024;

/// Versioned local editing state. Content's Debug is redacted.
#[derive(Clone, Debug)]
pub struct Draft {
    /// UUID allocated once for this editing session.
    pub id: String,
    /// Stable owning account.
    pub account: String,
    /// Monotonically increasing edit revision, used to reject stale autosave.
    pub revision: u64,
    /// Editable content; incomplete recipients are allowed until queueing.
    pub content: Content,
    /// Private profile snapshots, loaded and changed through attachment operations.
    pub attachments: Vec<attachments::StoredAttachment>,
}
impl Draft {
    /// Start a new local draft without contacting the account or generating MIME.
    #[must_use]
    pub fn new(account: String, from: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            account,
            revision: 1,
            attachments: Vec::new(),
            content: Content {
                from,
                ..Content::default()
            },
        }
    }
}

/// Immutable sidebar/list state, without body text.
#[derive(Clone, Debug)]
pub struct DraftSummary {
    /// Editing session identity.
    pub id: String,
    /// Owning account.
    pub account: String,
    /// Decoded subject shown in the draft list.
    pub subject: String,
    /// Last committed editor revision.
    pub revision: u64,
}

/// Durable SMTP state; only Pending can begin a network attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    /// Frozen and eligible for an explicit submit.
    Pending,
    /// A unique attempt was committed before network submission.
    Submitting,
    /// Server acknowledged DATA. Never send again merely to repair Sent APPEND.
    Accepted,
    /// Server may have accepted DATA; user reconciliation is required.
    DeliveryUnknown,
    /// Definite refusal/failure before acceptance; explicit retry remains possible.
    Failed,
    /// Cancelled before submission began.
    Cancelled,
}
impl Phase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Submitting => "submitting",
            Self::Accepted => "accepted",
            Self::DeliveryUnknown => "delivery_unknown",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
    fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "submitting" => Ok(Self::Submitting),
            "accepted" => Ok(Self::Accepted),
            "delivery_unknown" => Ok(Self::DeliveryUnknown),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(Error::InvalidInput),
        }
    }
}

/// One frozen outbox row. No mail body/envelope is exposed in list diagnostics.
#[derive(Clone, Debug)]
pub struct Summary {
    /// Subject of the frozen message, for explicit outgoing-intent selection.
    pub subject: String,
    /// Unique explicit sending intent, separate from its source draft.
    pub id: String,
    /// Stable account ID.
    pub account: String,
    /// Current durable phase.
    pub phase: Phase,
    /// Static redacted failure/reconciliation guidance.
    pub error: Option<String>,
    /// Whether a separately retriable Sent copy remains to be completed.
    pub sent_copy_pending: bool,
    /// A previous attempt might have delivered this message, even during retry.
    pub uncertain: bool,
}

/// A claimed network attempt. Its token fences callbacks from earlier attempts.
#[derive(Clone, Debug)]
pub struct Submission {
    /// Sending intent ID.
    pub id: String,
    /// Owning account.
    pub account: String,
    /// Unique attempt ID, required when committing its outcome.
    pub attempt: String,
    /// Exact original and envelope; Debug is redacted.
    pub message: Frozen,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    sender: String,
    recipients: Vec<String>,
}

impl Store {
    /// Recover a definitely rejected message as a new editable draft. The old
    /// sending intent becomes Cancelled and cannot later retry the old content.
    ///
    /// # Errors
    /// Rejects accepted, active or ambiguous deliveries and damaged stored data.
    pub fn recover_draft(&mut self, account: &str, id: &str) -> Result<Draft, Error> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record:Option<(Vec<u8>,String)>=tx.query_row("SELECT d.content,d.id FROM outbox o JOIN drafts d ON d.id=o.draft_id WHERE o.id=?1 AND o.account_id=?2 AND o.phase='failed' AND o.uncertain=0",params![id,account],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        let (content, source) = record.ok_or(Error::OperationConflict)?;
        if content.len() > MAX_DRAFT_BYTES {
            return Err(Error::InvalidInput);
        }
        let content: Content = serde_json::from_slice(&content)?;
        let draft = Draft {
            id: uuid::Uuid::new_v4().to_string(),
            account: account.into(),
            revision: 1,
            attachments: attachments::read_refs(&tx, &source)?,
            content,
        };
        tx.execute(
            "INSERT INTO drafts(id,account_id,revision,content,updated_at) VALUES(?1,?2,1,?3,?4)",
            params![
                draft.id,
                account,
                serde_json::to_vec(&draft.content)?,
                now()
            ],
        )?;
        attachments::copy_refs(&tx, &source, &draft.id)?;
        tx.execute(
            "UPDATE outbox SET phase='cancelled',updated_at=?3 WHERE id=?1 AND account_id=?2",
            params![id, account, now()],
        )?;
        tx.commit()?;
        Ok(draft)
    }
    /// Save even incomplete editable fields, rejecting stale/closed identities.
    ///
    /// # Errors
    /// Rejects unknown accounts, oversize JSON, revision regression and I/O errors.
    pub fn save_draft(&mut self, draft: &Draft) -> Result<(), Error> {
        validate_uuid(&draft.id)?;
        let revision = i64::try_from(draft.revision).map_err(|_| Error::InvalidInput)?;
        if revision == 0 {
            return Err(Error::InvalidInput);
        }
        let content = serde_json::to_vec(&draft.content)?;
        if content.len() > MAX_DRAFT_BYTES {
            return Err(Error::InvalidInput);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<(Option<String>, i64, bool, Vec<u8>)> = tx
            .query_row(
                "SELECT account_id,revision,closed,content FROM drafts WHERE id=?1",
                [&draft.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        if attachments::read_refs(&tx, &draft.id)? != draft.attachments {
            return Err(Error::OperationConflict);
        }
        if old
            .as_ref()
            .is_some_and(|(account, old_revision, closed, old_content)| {
                account.as_deref() != Some(&draft.account)
                    || *closed
                    || *old_revision > revision
                    || (*old_revision == revision && *old_content != content)
            })
        {
            return Err(Error::OperationConflict);
        }
        tx.execute("INSERT INTO drafts(id,account_id,revision,content,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,content=excluded.content,updated_at=excluded.updated_at",params![draft.id,draft.account,revision,content,now()])?;
        tx.commit()?;
        Ok(())
    }

    /// Load an editable draft for exactly one account. Closed drafts remain
    /// retained as tombstones so a late autosave cannot recreate sent content.
    ///
    /// # Errors
    /// Rejects missing/closed/oversized or invalid stored records.
    pub fn draft(&self, account: &str, id: &str) -> Result<Draft, Error> {
        let row: Option<(i64, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT revision,content FROM drafts WHERE id=?1 AND account_id=?2 AND closed=0",
                params![id, account],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (revision, content) = row.ok_or(Error::MissingMessage)?;
        if content.len() > MAX_DRAFT_BYTES {
            return Err(Error::InvalidInput);
        }
        Ok(Draft {
            id: id.into(),
            account: account.into(),
            revision: u64::try_from(revision).map_err(|_| Error::InvalidInput)?,
            content: serde_json::from_slice(&content)?,
            attachments: attachments::read_refs(&self.connection, id)?,
        })
    }

    /// At most 100 editable drafts for enabled accounts.
    ///
    /// # Errors
    /// Returns a request/metadata/storage failure without deleting old drafts.
    pub fn drafts(&self, accounts: &[String]) -> Result<Vec<DraftSummary>, Error> {
        if accounts.len() > 256 {
            return Err(Error::InvalidInput);
        }
        let mut statement=self.connection.prepare_cached("SELECT id,account_id,revision,content FROM drafts WHERE closed=0 AND account_id IN (SELECT value FROM json_each(?1)) ORDER BY updated_at DESC,id LIMIT 100")?;
        let rows = statement.query_map([serde_json::to_string(accounts)?], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (id, account, revision, bytes) = row?;
            if bytes.len() > MAX_DRAFT_BYTES {
                return Err(Error::InvalidInput);
            }
            let content: Content = serde_json::from_slice(&bytes)?;
            Ok(DraftSummary {
                id,
                account,
                revision: u64::try_from(revision).map_err(|_| Error::InvalidInput)?,
                subject: content.subject,
            })
        })
        .collect()
    }

    /// Freeze exactly one committed draft revision, before any network activity.
    /// Closed editing IDs prevent delayed autosave from rewriting that intent.
    ///
    /// # Errors
    /// Rejects stale/closed/invalid drafts, blob conflicts and storage failures.
    pub fn queue_draft(
        &mut self,
        account: &str,
        id: &str,
        revision: u64,
        server_saves_sent: bool,
    ) -> Result<Summary, Error> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let content:Option<Vec<u8>>=tx.query_row("SELECT content FROM drafts WHERE id=?1 AND account_id=?2 AND revision=?3 AND closed=0",params![id,account,i64::try_from(revision).map_err(|_|Error::InvalidInput)?],|row|row.get(0)).optional()?;
        let content = content.ok_or(Error::OperationConflict)?;
        if content.len() > MAX_DRAFT_BYTES {
            return Err(Error::InvalidInput);
        }
        let content: Content = serde_json::from_slice(&content)?;
        let files = attachments::load_bytes(&tx, &self.blobs, id)?;
        let frozen = content
            .freeze_with_attachments(&files)
            .map_err(|_| Error::InvalidInput)?;
        let hash = write_blob(&self.blobs, frozen.original())?;
        let envelope = Envelope {
            sender: frozen
                .envelope()
                .from()
                .ok_or(Error::InvalidInput)?
                .to_string(),
            recipients: frozen
                .envelope()
                .to()
                .iter()
                .map(ToString::to_string)
                .collect(),
        };
        let outbox_id = uuid::Uuid::new_v4().to_string();
        tx.execute("INSERT INTO outbox(id,account_id,draft_id,content,phase,envelope,message_id,blob_hash,updated_at,sent_copy,subject) VALUES(?1,?2,?3,X'','pending',?4,?5,?6,?7,?8,?9)",params![outbox_id,account,id,serde_json::to_string(&envelope)?,frozen.message_id(),hash,now(),if server_saves_sent{"not_needed"}else{"pending"},content.subject])?;
        tx.execute("UPDATE drafts SET closed=1 WHERE id=?1", [id])?;
        tx.commit()?;
        Ok(Summary {
            subject: content.subject,
            id: outbox_id,
            account: account.into(),
            phase: Phase::Pending,
            error: None,
            sent_copy_pending: !server_saves_sent,
            uncertain: false,
        })
    }

    /// Claim one Pending record. The attempt token and Submitting phase commit
    /// before the caller can send the verified frozen original.
    ///
    /// # Errors
    /// Rejects repeated/stale/wrong-account claims and damaged stored data.
    pub fn begin_submission(&mut self, account: &str, id: &str) -> Result<Submission, Error> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row:Option<(String,String,String)>=tx.query_row("SELECT envelope,message_id,blob_hash FROM outbox WHERE id=?1 AND account_id=?2 AND phase='pending'",params![id,account],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
        let (envelope, message_id, hash) = row.ok_or(Error::OperationConflict)?;
        if envelope.len() > 64 * 1024 {
            return Err(Error::InvalidInput);
        }
        let envelope: Envelope = serde_json::from_str(&envelope)?;
        let original = super::read_blob(&self.blobs, &hash)?;
        let message = Frozen::restore(message_id, &envelope.sender, envelope.recipients, original)
            .map_err(|_| Error::InvalidInput)?;
        let attempt = uuid::Uuid::new_v4().to_string();
        tx.execute("UPDATE outbox SET phase='submitting',attempt_id=?3,last_error=NULL,updated_at=?4 WHERE id=?1 AND account_id=?2",params![id,account,attempt,now()])?;
        tx.commit()?;
        Ok(Submission {
            id: id.into(),
            account: account.into(),
            attempt,
            message,
        })
    }

    /// Commit exactly one current attempt's result. Ambiguous delivery is never
    /// changed back to Pending here; only an explicit retry decision can do so.
    ///
    /// # Errors
    /// Rejects stale callbacks and storage failures, retaining durable intent.
    pub fn finish_submission(
        &mut self,
        submission: &Submission,
        outcome: Outcome,
    ) -> Result<(), Error> {
        let (mut phase, mut error) = match outcome {
            Outcome::Accepted => (Phase::Accepted, None),
            Outcome::Rejected { temporary: true } => (
                Phase::Failed,
                Some("server temporarily rejected submission"),
            ),
            Outcome::Rejected { temporary: false } => {
                (Phase::Failed, Some("server rejected submission"))
            }
            Outcome::AuthenticationRequired => (
                Phase::Failed,
                Some("authentication refused; foreground login required"),
            ),
            Outcome::CapabilityUnavailable => (
                Phase::Failed,
                Some("server lacks required security, authentication or encoding capability"),
            ),
            Outcome::TlsFailed => (
                Phase::Failed,
                Some("TLS validation failed before submission"),
            ),
            Outcome::CancelledBeforeSubmission => (Phase::Cancelled, None),
            Outcome::DeliveryUnknown => (
                Phase::DeliveryUnknown,
                Some("delivery unknown; check Sent before deciding whether to resend"),
            ),
        };
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let uncertain:Option<bool>=tx.query_row("SELECT uncertain FROM outbox WHERE id=?1 AND account_id=?2 AND attempt_id=?3 AND phase='submitting'",params![submission.id,submission.account,submission.attempt],|row|row.get(0)).optional()?;
        let uncertain = uncertain.ok_or(Error::OperationConflict)?;
        if uncertain && phase != Phase::Accepted {
            phase = Phase::DeliveryUnknown;
            error = Some(
                "current retry did not confirm delivery; an earlier attempt may have delivered; check Sent before resending",
            );
        }
        let changed=tx.execute("UPDATE outbox SET phase=?4,last_error=?5,updated_at=?6,uncertain=MAX(uncertain,?7) WHERE id=?1 AND account_id=?2 AND attempt_id=?3 AND phase='submitting'",params![submission.id,submission.account,submission.attempt,phase.as_str(),error,now(),phase==Phase::DeliveryUnknown])?;
        if changed != 1 {
            return Err(Error::OperationConflict);
        }
        if phase == Phase::Cancelled {
            tx.execute("UPDATE drafts SET closed=0 WHERE id=(SELECT draft_id FROM outbox WHERE id=?1 AND account_id=?2)",params![submission.id,submission.account])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Explicitly retry a definite failure, or an unknown delivery after the
    /// caller obtained a conscious user decision. Never invoke automatically.
    /// Frozen bytes/Message-ID are retained; a new attempt token is generated later.
    ///
    /// # Errors
    /// Rejects accepted/in-flight/cancelled records and unconfirmed unknown delivery.
    pub fn retry_submission(
        &mut self,
        account: &str,
        id: &str,
        confirm_unknown: bool,
    ) -> Result<(), Error> {
        let changed=self.connection.execute("UPDATE outbox SET phase='pending',last_error=NULL,updated_at=?4 WHERE id=?1 AND account_id=?2 AND (phase='failed' OR (phase='delivery_unknown' AND ?3=1)) AND (uncertain=0 OR ?3=1)",params![id,account,confirm_unknown,now()])?;
        if changed != 1 {
            return Err(Error::OperationConflict);
        }
        Ok(())
    }

    /// Cancel a frozen record only before an attempt begins.
    ///
    /// # Errors
    /// Rejects in-flight/accepted/unknown records; never labels ambiguous mail unsent.
    pub fn cancel_submission(&mut self, account: &str, id: &str) -> Result<(), Error> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed=tx.execute("UPDATE outbox SET phase=CASE WHEN uncertain=1 THEN 'delivery_unknown' ELSE 'cancelled' END,updated_at=?3 WHERE id=?1 AND account_id=?2 AND phase='pending'",params![id,account,now()])?;
        if changed != 1 {
            return Err(Error::OperationConflict);
        }
        tx.execute("UPDATE drafts SET closed=0 WHERE id=(SELECT draft_id FROM outbox WHERE id=?1 AND account_id=?2 AND uncertain=0)",params![id,account])?;
        tx.commit()?;
        Ok(())
    }

    /// At most 100 outgoing intents for enabled accounts. Accepted records keep
    /// their separately retriable Sent-copy status, without becoming sendable.
    ///
    /// # Errors
    /// Returns a request/metadata error; unsupported legacy content is preserved.
    pub fn outbox(&self, accounts: &[String]) -> Result<Vec<Summary>, Error> {
        if accounts.len() > 256 {
            return Err(Error::InvalidInput);
        }
        let mut statement=self.connection.prepare_cached("SELECT id,account_id,phase,last_error,sent_copy,subject,uncertain FROM outbox WHERE account_id IN (SELECT value FROM json_each(?1)) ORDER BY updated_at DESC,id LIMIT 100")?;
        let rows = statement.query_map([serde_json::to_string(accounts)?], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, bool>(6)?,
            ))
        })?;
        rows.map(|row| {
            let (id, account, phase, error, sent_copy, subject, uncertain) = row?;
            Ok(Summary {
                id,
                account,
                phase: Phase::parse(&phase)?,
                error,
                sent_copy_pending: sent_copy == "pending",
                subject,
                uncertain,
            })
        })
        .collect()
    }

    // Called only by the exclusive Worker owner, never by arbitrary Store::open.
    // A second integrity/GC handle must not declare a live network attempt lost.
    pub(crate) fn recover_submissions(&self) -> Result<(), Error> {
        self.connection.execute("UPDATE outbox SET phase='delivery_unknown',uncertain=1,last_error='submission interrupted; check Sent before deciding whether to resend',updated_at=?1 WHERE phase='submitting'",[now()])?;
        Ok(())
    }
}

fn validate_uuid(value: &str) -> Result<(), Error> {
    if uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value) {
        Ok(())
    } else {
        Err(Error::InvalidInput)
    }
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}
