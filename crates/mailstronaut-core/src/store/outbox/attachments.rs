//! Owned attachment snapshots. SQLite transactions coordinate blob GC and edits.

use core::fmt;

use mailstronaut_mail::compose::{self, AttachmentData};
use rusqlite::{Connection, TransactionBehavior, params};

use super::{Draft, now};
use crate::store::{Error, Store, read_blob, write_blob};

pub(crate) const SCHEMA: &str = "
CREATE TABLE draft_attachments (
 draft_id TEXT NOT NULL REFERENCES drafts(id),
 id TEXT NOT NULL,
 name TEXT NOT NULL,
 blob_hash TEXT NOT NULL,
 byte_len INTEGER NOT NULL CHECK(byte_len>=0),
 PRIMARY KEY(draft_id,id)
);
CREATE INDEX attachment_blob ON draft_attachments(blob_hash);
";

/// Metadata for a private owned snapshot; never a source filesystem path.
#[derive(Clone, Eq, PartialEq)]
pub struct StoredAttachment {
    /// Stable identifier scoped to the owning draft.
    pub id: String,
    /// Recipient-visible basename.
    pub name: String,
    /// Decoded file size.
    pub bytes: usize,
    hash: String,
}
impl fmt::Debug for StoredAttachment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StoredAttachment(<private file>)")
    }
}

pub(super) fn read_refs(db: &Connection, id: &str) -> Result<Vec<StoredAttachment>, Error> {
    let mut statement = db.prepare("SELECT id,name,blob_hash,byte_len FROM draft_attachments WHERE draft_id=?1 ORDER BY rowid LIMIT 17")?;
    let rows = statement.query_map([id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    let mut files = Vec::new();
    let mut total = 0usize;
    for row in rows {
        let (id, name, hash, size) = row?;
        let bytes = usize::try_from(size).map_err(|_| Error::InvalidInput)?;
        total = total.checked_add(bytes).ok_or(Error::InvalidInput)?;
        if !compose::valid_attachment_name(&name)
            || files.len() >= compose::MAX_ATTACHMENTS
            || total > compose::MAX_ATTACHMENT_BYTES
        {
            return Err(Error::InvalidInput);
        }
        files.push(StoredAttachment {
            id,
            name,
            bytes,
            hash,
        });
    }
    Ok(files)
}

pub(super) fn load_bytes(
    db: &Connection,
    blobs: &std::path::Path,
    id: &str,
) -> Result<Vec<AttachmentData>, Error> {
    read_refs(db, id)?
        .into_iter()
        .map(|file| {
            let bytes = read_blob(blobs, &file.hash)?;
            if bytes.len() != file.bytes {
                return Err(Error::CorruptBlob);
            }
            Ok(AttachmentData {
                name: file.name,
                bytes,
            })
        })
        .collect()
}

pub(super) fn copy_refs(db: &Connection, source: &str, target: &str) -> Result<(), Error> {
    db.execute("INSERT INTO draft_attachments(draft_id,id,name,blob_hash,byte_len) SELECT ?2,id,name,blob_hash,byte_len FROM draft_attachments WHERE draft_id=?1 ORDER BY rowid", params![source,target])?;
    Ok(())
}

fn current(db: &Connection, draft: &Draft) -> Result<(), Error> {
    let matches: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM drafts WHERE id=?1 AND account_id=?2 AND revision=?3 AND closed=0)",
        params![draft.id,draft.account,i64::try_from(draft.revision).map_err(|_|Error::InvalidInput)?], |r|r.get(0))?;
    if !matches || read_refs(db, &draft.id)? != draft.attachments {
        return Err(Error::OperationConflict);
    }
    Ok(())
}

fn advance(db: &Connection, draft: &Draft) -> Result<(), Error> {
    let revision = draft
        .revision
        .checked_add(1)
        .and_then(|r| i64::try_from(r).ok())
        .ok_or(Error::InvalidInput)?;
    db.execute(
        "UPDATE drafts SET revision=?2,updated_at=?3 WHERE id=?1",
        params![draft.id, revision, now()],
    )?;
    Ok(())
}

impl Store {
    /// Copy bounded attachment bytes into the private profile and advance the draft.
    /// Saving incomplete editor text is independent; attachment insertion is atomic.
    /// # Errors
    /// Rejects stale/closed drafts, excessive files and failed durable writes.
    pub fn add_draft_attachments(
        &mut self,
        draft: &Draft,
        files: Vec<AttachmentData>,
    ) -> Result<Draft, Error> {
        compose::validate_attachments(&files).map_err(|_| Error::InvalidInput)?;
        if files.is_empty() || draft.attachments.len() + files.len() > compose::MAX_ATTACHMENTS {
            return Err(Error::InvalidInput);
        }
        let total = draft
            .attachments
            .iter()
            .map(|f| f.bytes)
            .chain(files.iter().map(|f| f.bytes.len()))
            .try_fold(0usize, usize::checked_add)
            .ok_or(Error::InvalidInput)?;
        if total > compose::MAX_ATTACHMENT_BYTES {
            return Err(Error::InvalidInput);
        }
        self.save_draft(draft)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        current(&tx, draft)?;
        for file in files {
            let hash = write_blob(&self.blobs, &file.bytes)?;
            tx.execute("INSERT INTO draft_attachments(draft_id,id,name,blob_hash,byte_len) VALUES(?1,?2,?3,?4,?5)",
                params![draft.id,uuid::Uuid::new_v4().to_string(),file.name,hash,i64::try_from(file.bytes.len()).map_err(|_|Error::InvalidInput)?])?;
        }
        advance(&tx, draft)?;
        tx.commit()?;
        self.draft(&draft.account, &draft.id)
    }

    /// Remove one exact draft-local attachment; collection of its blob is separate.
    /// # Errors
    /// Rejects stale/closed/foreign drafts and unknown attachment identifiers.
    pub fn remove_draft_attachment(&mut self, draft: &Draft, id: &str) -> Result<Draft, Error> {
        self.save_draft(draft)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        current(&tx, draft)?;
        if tx.execute(
            "DELETE FROM draft_attachments WHERE draft_id=?1 AND id=?2",
            params![draft.id, id],
        )? != 1
        {
            return Err(Error::OperationConflict);
        }
        advance(&tx, draft)?;
        tx.commit()?;
        self.draft(&draft.account, &draft.id)
    }

    /// Create an independent draft under another configured sender, retaining files.
    /// # Errors
    /// Rejects stale source state, unknown destination accounts and storage failures.
    pub fn copy_draft(
        &mut self,
        draft: &Draft,
        account: &str,
        from: String,
    ) -> Result<Draft, Error> {
        if from.len() > 4096 {
            return Err(Error::InvalidInput);
        }
        self.save_draft(draft)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        current(&tx, draft)?;
        let mut copy = Draft::new(account.into(), from.clone());
        copy.content = draft.content.clone();
        copy.content.from = from;
        let content = serde_json::to_vec(&copy.content)?;
        if content.len() > super::MAX_DRAFT_BYTES {
            return Err(Error::InvalidInput);
        }
        tx.execute(
            "INSERT INTO drafts(id,account_id,revision,content,updated_at) VALUES(?1,?2,1,?3,?4)",
            params![copy.id, account, content, now()],
        )?;
        copy_refs(&tx, &draft.id, &copy.id)?;
        tx.commit()?;
        self.draft(account, &copy.id)
    }
}
