//! Account-scoped IMAP identities and paginated offline inbox snapshots.

use core::num::NonZeroU32;

use rusqlite::{OptionalExtension as _, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::{Error, Store, write_blob};
use crate::mime::{self, Headers, Projection, Status};

/// Forward migration: never recycle message IDs and retain decoded metadata.
pub(super) const SCHEMA: &str = "
CREATE TABLE messages_next (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  message_id TEXT,
  subject TEXT NOT NULL DEFAULT '',
  preview TEXT NOT NULL DEFAULT '',
  blob_hash TEXT NOT NULL,
  received_at INTEGER NOT NULL DEFAULT 0,
  sender TEXT NOT NULL DEFAULT '',
  metadata TEXT,
  read_cleared INTEGER NOT NULL DEFAULT 0 CHECK(read_cleared IN (0, 1)),
  UNIQUE(account_id, id)
);
INSERT INTO messages_next (id, account_id, message_id, subject, preview, blob_hash)
  SELECT id, account_id, message_id, subject, preview, blob_hash FROM messages;
DROP TABLE messages;
ALTER TABLE messages_next RENAME TO messages;
ALTER TABLE memberships ADD COLUMN starred INTEGER NOT NULL DEFAULT 0 CHECK(starred IN (0, 1));
CREATE INDEX exact_message_copy ON messages(account_id, message_id, blob_hash);
CREATE INDEX inbox_order ON messages(received_at DESC, id DESC);
CREATE INDEX message_memberships ON memberships(message_id);
CREATE INDEX inbox_folders ON folders(account_id, is_inbox);
";

/// A discovered mailbox and its current server generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Folder {
    /// Stable local folder identity.
    pub id: i64,
    /// Owning configuration account ID.
    pub account: String,
    /// Exact opaque server wire name, never a filesystem component.
    pub name: String,
    /// UID namespace, absent until SELECT succeeds.
    pub uid_validity: Option<NonZeroU32>,
    /// Whether this folder participates in the unified inbox.
    pub inbox: bool,
}

/// Observed server flags; fetching a body never changes them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Flags {
    /// Server reports Seen.
    pub seen: bool,
    /// Server reports Flagged.
    pub starred: bool,
}

/// Server identity for one message. Sequence numbers are deliberately absent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Membership {
    /// Local folder ID, already bound to this account.
    pub folder: i64,
    /// SELECT's nonzero UIDVALIDITY.
    pub uid_validity: NonZeroU32,
    /// Nonzero IMAP UID.
    pub uid: NonZeroU32,
    /// Observed flags.
    pub flags: Flags,
}

/// Trusted provider metadata from an authenticated Gmail protocol response.
/// This ID must never be populated from an email header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GmailId(pub core::num::NonZeroU64);

/// Complete unfiltered SEARCH result for one closed UID span.
#[derive(Clone, Debug)]
pub struct UidSnapshot {
    /// Account-local folder identity.
    pub folder: i64,
    /// Selected UID namespace.
    pub uid_validity: NonZeroU32,
    /// First searched UID, inclusive.
    pub first: NonZeroU32,
    /// Last searched UID, inclusive.
    pub last: NonZeroU32,
    /// Every UID returned by the successful unfiltered search.
    pub present: Vec<NonZeroU32>,
}

/// A stable keyset cursor, independent of offset shifts after new mail arrives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cursor {
    /// Last row's server internal date in UTC seconds.
    pub received_at: i64,
    /// Last row's immutable local message ID.
    pub message_id: i64,
}

/// An immutable inbox row; account badges use the stable account ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Summary {
    /// Immutable local message identity.
    pub id: i64,
    /// Owning account; identical mail in another account remains separate.
    pub account: String,
    /// Decoded subject.
    pub subject: String,
    /// Decoded display sender.
    pub sender: String,
    /// Extracted preview, never a generated summary.
    pub preview: String,
    /// Server internal date in UTC seconds.
    pub received_at: i64,
    /// True only when every participating inbox membership is Seen.
    pub seen: bool,
    /// True when any participating inbox membership is Flagged.
    pub starred: bool,
}
impl Summary {
    /// Continue after this row without using an unstable numeric offset.
    #[must_use]
    pub const fn cursor(&self) -> Cursor {
        Cursor {
            received_at: self.received_at,
            message_id: self.id,
        }
    }
}

/// Stored parse metadata; originals remain the authoritative body source.
#[derive(Serialize, Deserialize)]
struct Metadata {
    headers: Headers,
    status: Status,
}

impl Store {
    /// Observe flags for an already cached UID without fetching its original
    /// again. Read-cleared analysis history is monotonic across later Unseen.
    ///
    /// # Errors
    /// Rejects stale/wrong-account generations and storage failures.
    pub fn observe_membership(
        &mut self,
        account: &str,
        membership: Membership,
    ) -> Result<bool, Error> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_generation(&tx, account, membership)?;
        let changed = tx.execute("UPDATE memberships SET seen=?5,starred=?6 WHERE account_id=?1 AND folder_id=?2 AND uidvalidity=?3 AND uid=?4",params![account,membership.folder,membership.uid_validity.get(),membership.uid.get(),membership.flags.seen,membership.flags.starred])?;
        if changed != 0 && membership.flags.seen {
            tx.execute("UPDATE messages SET read_cleared=1 WHERE account_id=?1 AND id IN (SELECT message_id FROM memberships WHERE account_id=?1 AND folder_id=?2 AND uidvalidity=?3 AND uid=?4)",params![account,membership.folder,membership.uid_validity.get(),membership.uid.get()])?;
        }
        tx.commit()?;
        Ok(changed != 0)
    }

    /// Retire expunged memberships only inside a successfully searched closed
    /// UID span. The caller must pass an unfiltered UID SEARCH result (no SINCE,
    /// UNSEEN or other filter). Originals/user-owned state remain intact.
    ///
    /// # Errors
    /// Rejects spans over 1000, duplicate/out-of-range UIDs and stale generations.
    pub fn reconcile_span(&mut self, account: &str, snapshot: &UidSnapshot) -> Result<(), Error> {
        let UidSnapshot {
            folder,
            uid_validity: validity,
            first,
            last,
            present,
        } = snapshot;
        if last < first || last.get() - first.get() >= 1000 || present.len() > 1000 {
            return Err(Error::InvalidInput);
        }
        let unique = present
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        if unique.len() != present.len() || unique.iter().any(|uid| uid < first || uid > last) {
            return Err(Error::InvalidInput);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_generation(
            &tx,
            account,
            Membership {
                folder: *folder,
                uid_validity: *validity,
                uid: *first,
                flags: Flags::default(),
            },
        )?;
        let present =
            serde_json::to_string(&present.iter().map(|uid| uid.get()).collect::<Vec<_>>())?;
        tx.execute("DELETE FROM memberships WHERE account_id=?1 AND folder_id=?2 AND uidvalidity=?3 AND uid BETWEEN ?4 AND ?5 AND uid NOT IN (SELECT value FROM json_each(?6))",params![account,folder,validity.get(),first.get(),last.get(),present])?;
        tx.commit()?;
        Ok(())
    }

    /// Add a discovered mailbox without resetting a known UID generation.
    ///
    /// # Errors
    /// Rejects invalid names, unknown accounts and database failures.
    pub fn ensure_folder(
        &mut self,
        account: &str,
        name: &str,
        inbox: bool,
    ) -> Result<Folder, Error> {
        if name.is_empty() || name.len() > 4096 || name.contains(['\r', '\n', '\0']) {
            return Err(Error::InvalidInput);
        }
        self.connection.execute("INSERT INTO folders(account_id, name, is_inbox) VALUES (?1, ?2, ?3) ON CONFLICT(account_id, name) DO UPDATE SET is_inbox=excluded.is_inbox", params![account, name, inbox])?;
        self.connection.query_row("SELECT id, account_id, name, uidvalidity, is_inbox FROM folders WHERE account_id=?1 AND name=?2", params![account, name], read_folder).map_err(Error::from)
    }

    /// Commit SELECT's generation. A changed generation discards old server UID
    /// bindings and invalidates their pending operations, retaining original mail,
    /// reminder IDs, drafts, outbox and read/analysis history for reconciliation.
    ///
    /// # Errors
    /// Rejects a folder from another account or a database failure.
    pub fn select_folder(
        &mut self,
        account: &str,
        folder: i64,
        validity: NonZeroU32,
    ) -> Result<(), Error> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<Option<u32>> = tx
            .query_row(
                "SELECT uidvalidity FROM folders WHERE account_id=?1 AND id=?2",
                params![account, folder],
                |row| row.get(0),
            )
            .optional()?;
        let Some(previous) = previous else {
            return Err(Error::InvalidInput);
        };
        if previous != Some(validity.get()) {
            tx.execute(
                "DELETE FROM memberships WHERE account_id=?1 AND folder_id=?2",
                params![account, folder],
            )?;
            tx.execute("UPDATE pending_operations SET state='conflict' WHERE account_id=?1 AND folder_id=?2 AND state IN ('pending', 'submitted')", params![account, folder])?;
            tx.execute(
                "UPDATE folders SET uidvalidity=?3 WHERE account_id=?1 AND id=?2",
                params![account, folder, validity.get()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Cache one PEEK-fetched original, its canonical identity and server flags in
    /// one transaction. Manual exact copies require both Message-ID and raw hash;
    /// Gmail label copies use the account-scoped provider ID. No cross-account
    /// deduplication or subject-based identity is permitted.
    ///
    /// # Errors
    /// Rejects stale generations, conflicting UIDs/provider identities, oversized
    /// or unparseable MIME, corrupt blobs and storage failures.
    pub fn cache_message(
        &mut self,
        account: &str,
        membership: Membership,
        received_at: i64,
        gmail: Option<GmailId>,
        original: &[u8],
    ) -> Result<i64, Error> {
        let projection = mime::project(original).map_err(Error::Mime)?;
        let metadata = serde_json::to_string(&Metadata {
            headers: projection.headers.clone(),
            status: projection.status.clone(),
        })?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_generation(&tx, account, membership)?;
        let hash = write_blob(&self.blobs, original)?;
        let existing = canonical(
            &tx,
            account,
            membership,
            gmail,
            projection.headers.message_id.as_deref(),
            &hash,
        )?;
        let subject = projection.headers.subject.as_deref().unwrap_or_default();
        let preview = projection
            .preview
            .as_ref()
            .map_or("", |value| value.text.as_str());
        let sender = display_sender(&projection.headers);
        let id = if let Some(id) = existing {
            tx.execute("UPDATE messages SET message_id=?2, subject=?3, preview=?4, blob_hash=?5, sender=?6, metadata=?7, read_cleared=MAX(read_cleared, ?8) WHERE id=?1", params![id, projection.headers.message_id, subject, preview, hash, sender, metadata, membership.flags.seen])?;
            id
        } else {
            tx.execute("INSERT INTO messages(account_id, message_id, subject, preview, blob_hash, received_at, sender, metadata, read_cleared) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![account, projection.headers.message_id, subject, preview, hash, received_at, sender, metadata, membership.flags.seen])?;
            tx.last_insert_rowid()
        };
        if let Some(provider) = gmail {
            tx.execute("INSERT INTO provider_ids(account_id, provider_id, message_id) VALUES (?1,?2,?3) ON CONFLICT(account_id, provider_id) DO NOTHING", params![account, provider.0.get().to_string(), id])?;
        }
        tx.execute("INSERT INTO memberships(account_id, folder_id, uidvalidity, uid, message_id, seen, starred) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(account_id, folder_id, uidvalidity, uid) DO UPDATE SET seen=excluded.seen, starred=excluded.starred", params![account, membership.folder, membership.uid_validity.get(), membership.uid.get(), id, membership.flags.seen, membership.flags.starred])?;
        tx.execute("DELETE FROM message_search WHERE rowid=?1", params![id])?;
        let body = projection
            .text
            .clone()
            .or_else(|| projection.html.as_deref().map(mail_parser::html_to_text))
            .unwrap_or_default();
        tx.execute(
            "INSERT INTO message_search(rowid, subject, addresses, body) VALUES (?1,?2,?3,?4)",
            params![id, subject, search_addresses(&projection.headers), body],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Query at most 100 immutable rows from all designated inbox folders. Empty
    /// account selection returns no rows; disabled accounts are not displayed.
    ///
    /// # Errors
    /// Rejects invalid page sizes and database failures.
    pub fn inbox(
        &self,
        accounts: &[String],
        after: Option<Cursor>,
        limit: u32,
    ) -> Result<Vec<Summary>, Error> {
        if !(1..=100).contains(&limit) || accounts.len() > 256 {
            return Err(Error::InvalidInput);
        }
        // Account filtering is parameterized with SQLite's bundled JSON table
        // function; no user value becomes SQL text or an identifier.
        let selected = serde_json::to_string(accounts)?;
        let mut statement = self.connection.prepare_cached("SELECT m.id,m.account_id,m.subject,m.sender,m.preview,m.received_at,MIN(s.seen),MAX(s.starred) FROM messages m JOIN memberships s ON s.message_id=m.id JOIN folders f ON f.id=s.folder_id AND f.account_id=m.account_id WHERE f.is_inbox=1 AND m.account_id IN (SELECT value FROM json_each(?1)) AND (?2 IS NULL OR m.received_at < ?2 OR (m.received_at=?2 AND m.id<?3)) GROUP BY m.id ORDER BY m.received_at DESC,m.id DESC LIMIT ?4")?;
        let rows = statement.query_map(
            params![
                selected,
                after.map(|value| value.received_at),
                after.map(|value| value.message_id),
                limit
            ],
            read_summary,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }

    /// Read and verify the bounded original for the selected local message.
    /// No renderer, model, network request or attachment execution is involved.
    ///
    /// # Errors
    /// Returns a missing-message, integrity, MIME or storage error.
    pub fn message(&self, account: &str, id: i64) -> Result<Projection, Error> {
        let hash: String = self
            .connection
            .query_row(
                "SELECT blob_hash FROM messages WHERE account_id=?1 AND id=?2",
                params![account, id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(Error::MissingMessage)?;
        let original = super::read_blob(&self.blobs, &hash)?;
        mime::project(&original).map_err(Error::Mime)
    }
}

fn read_folder(row: &rusqlite::Row<'_>) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: row.get(0)?,
        account: row.get(1)?,
        name: row.get(2)?,
        uid_validity: row.get::<_, Option<u32>>(3)?.and_then(NonZeroU32::new),
        inbox: row.get(4)?,
    })
}
fn read_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<Summary> {
    Ok(Summary {
        id: row.get(0)?,
        account: row.get(1)?,
        subject: row.get(2)?,
        sender: row.get(3)?,
        preview: row.get(4)?,
        received_at: row.get(5)?,
        seen: row.get(6)?,
        starred: row.get(7)?,
    })
}
fn check_generation(
    tx: &Transaction<'_>,
    account: &str,
    membership: Membership,
) -> Result<(), Error> {
    let valid: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM folders WHERE account_id=?1 AND id=?2 AND uidvalidity=?3)",
        params![account, membership.folder, membership.uid_validity.get()],
        |row| row.get(0),
    )?;
    if valid {
        Ok(())
    } else {
        Err(Error::StaleMailbox)
    }
}
fn canonical(
    tx: &Transaction<'_>,
    account: &str,
    membership: Membership,
    gmail: Option<GmailId>,
    message_id: Option<&str>,
    hash: &str,
) -> Result<Option<i64>, Error> {
    let binding: Option<(i64, String)> = tx.query_row("SELECT m.id,m.blob_hash FROM memberships s JOIN messages m ON m.id=s.message_id WHERE s.account_id=?1 AND s.folder_id=?2 AND s.uidvalidity=?3 AND s.uid=?4", params![account, membership.folder, membership.uid_validity.get(), membership.uid.get()], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
    let provider: Option<i64> = if let Some(id) = gmail {
        tx.query_row(
            "SELECT message_id FROM provider_ids WHERE account_id=?1 AND provider_id=?2",
            params![account, id.0.get().to_string()],
            |row| row.get(0),
        )
        .optional()?
    } else {
        None
    };
    if let Some((id, old_hash)) = binding {
        if let Some(incoming) = gmail {
            let previous: Option<String> = tx.query_row(
                "SELECT provider_id FROM provider_ids WHERE account_id=?1 AND message_id=?2 LIMIT 1",
                params![account, id], |row| row.get(0),
            ).optional()?;
            if previous.is_some_and(|value| value != incoming.0.get().to_string()) {
                return Err(Error::IdentityConflict);
            }
        }
        if provider.is_some_and(|other| other != id) || (old_hash != hash && provider != Some(id)) {
            return Err(Error::IdentityConflict);
        }
        return Ok(Some(id));
    }
    // A known Gmail ID, including an as-yet uncached one, takes precedence over
    // Message-ID/raw-hash fallback. Different Gmail IDs remain distinct.
    if gmail.is_some() {
        return Ok(provider);
    }
    if let Some(message_id) = message_id.filter(|value| !value.is_empty()) {
        return tx.query_row("SELECT id FROM messages WHERE account_id=?1 AND message_id=?2 AND blob_hash=?3 ORDER BY id LIMIT 1", params![account, message_id, hash], |row| row.get(0)).optional().map_err(Error::from);
    }
    Ok(None)
}
fn display_sender(headers: &Headers) -> String {
    headers.from.first().map_or_else(String::new, |sender| {
        sender
            .name
            .as_deref()
            .or(sender.address.as_deref())
            .unwrap_or_default()
            .to_owned()
    })
}
fn search_addresses(headers: &Headers) -> String {
    headers
        .from
        .iter()
        .chain(&headers.to)
        .chain(&headers.cc)
        .filter_map(|mailbox| mailbox.address.as_deref())
        .collect::<Vec<_>>()
        .join(" ")
}
