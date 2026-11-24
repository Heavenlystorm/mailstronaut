//! A separate durable embedding frontier, independent of lexical acknowledgments.
//!
//! This supplies bounded readable projections and canonical revision allowlists.
//! Inference and derived-index ownership stay outside the canonical store.

use super::{
    Error, Store,
    browse::{Scope, SearchFilter},
    search::SENDER_TEXT,
};
use rusqlite::params;
use std::collections::BTreeMap;

/// Change this when the readable source projection changes; combine with model policy.
pub const PROJECTION_REVISION: &str = "readable-subject-body-32k-v1";
const TEXT_BYTES: usize = 32 * 1024;

pub(super) const SCHEMA: &str = "
CREATE TABLE semantic_pending (seq INTEGER PRIMARY KEY AUTOINCREMENT, message_id INTEGER NOT NULL UNIQUE);
INSERT INTO semantic_pending(message_id) SELECT id FROM messages;
CREATE TRIGGER semantic_queued AFTER INSERT ON search_pending BEGIN
 INSERT OR REPLACE INTO semantic_pending(message_id) VALUES (new.message_id);
END;
";

/// Readable content for one bounded inference job, without opening/marking it Seen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Source {
    /// Owning account; the runtime must exclude disabled accounts.
    pub account: String,
    /// Exact immutable original SHA-256, combined with projection/model policy.
    pub revision: String,
    /// Bounded decoded subject.
    pub subject: String,
    /// Bounded decoded readable body.
    pub body: String,
    /// Some decoded subject/body was omitted by MIME or this projection's limits.
    pub truncated: bool,
}
/// A source replacement, or removal from the derived index when unavailable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    /// Canonical original ID.
    pub id: i64,
    /// Absent for deleted, encrypted, headers-only or unavailable projections.
    pub source: Option<Source>,
}
/// Bounded work retained until the separate semantic consumer commits it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Batch {
    /// Up to 32 queued changes.
    pub items: Vec<Item>,
    /// Acknowledge through this sequence only after durable derived storage.
    pub through: Option<i64>,
    /// All queued changes, including this batch.
    pub queued: u64,
}

impl Store {
    /// Read a small embedding batch without consuming lexical or semantic work.
    /// # Errors
    /// Rejects a size outside 1..=32 and reports storage failures.
    pub fn semantic_pending(&self, limit: u32) -> Result<Batch, Error> {
        if !(1..=32).contains(&limit) {
            return Err(Error::InvalidInput);
        }
        let queued: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM semantic_pending", [], |row| {
                    row.get(0)
                })?;
        let mut query = self.connection.prepare_cached(
            "SELECT p.seq,p.message_id,m.account_id,m.blob_hash,
         substr(f.subject,1,32768),substr(f.body,1,32768),
         length(CAST(f.subject AS BLOB))>32768 OR length(CAST(f.body AS BLOB))>32768
         OR COALESCE(json_extract(m.metadata,'$.status.body_truncated'),0)
         OR COALESCE(json_extract(m.metadata,'$.status.headers_truncated'),0)
         FROM semantic_pending p LEFT JOIN messages m ON m.id=p.message_id
         LEFT JOIN message_search f ON f.rowid=p.message_id AND m.omitted IS NULL
         AND COALESCE(json_extract(m.metadata,'$.status.encrypted_unsupported'),0)=0
         ORDER BY p.seq LIMIT ?1",
        )?;
        let mut rows = query.query([limit])?;
        let mut batch = Batch {
            queued: u64::try_from(queued).unwrap_or_default(),
            ..Batch::default()
        };
        while let Some(row) = rows.next()? {
            let subject: Option<String> = row.get(4)?;
            let source = match subject {
                Some(mut subject) => {
                    let mut body: String = row.get(5)?;
                    subject.truncate(subject.floor_char_boundary(TEXT_BYTES.min(subject.len())));
                    body.truncate(body.floor_char_boundary(TEXT_BYTES.min(body.len())));
                    Some(Source {
                        account: row.get(2)?,
                        revision: row.get(3)?,
                        subject,
                        body,
                        truncated: row.get(6)?,
                    })
                }
                None => None,
            };
            batch.through = Some(row.get(0)?);
            batch.items.push(Item {
                id: row.get(1)?,
                source,
            });
        }
        Ok(batch)
    }

    /// Acknowledge only this consumer. A concurrent replacement has a later sequence.
    /// # Errors
    /// Rejects an invalid sequence and reports storage failures.
    pub fn semantic_indexed(&mut self, through: i64) -> Result<(), Error> {
        if through <= 0 {
            return Err(Error::InvalidInput);
        }
        self.connection
            .execute("DELETE FROM semantic_pending WHERE seq<=?1", [through])?;
        Ok(())
    }

    /// Queue every original again for a new/invalidated semantic index.
    /// # Errors
    /// Reports storage failures; does not consume or recreate the lexical frontier.
    pub fn requeue_semantic(&mut self) -> Result<(), Error> {
        self.connection.execute(
            "INSERT OR REPLACE INTO semantic_pending(message_id) SELECT id FROM messages",
            [],
        )?;
        Ok(())
    }

    /// Current enabled/scope/filter revisions, before any vector hit limit.
    /// This metadata-only query neither loads source bodies nor changes flags.
    /// # Errors
    /// Rejects invalid scope/filters and reports storage failures.
    pub fn semantic_candidates(
        &self,
        accounts: &[String],
        scope: &Scope,
        filter: &SearchFilter,
    ) -> Result<BTreeMap<i64, String>, Error> {
        if accounts.len() > 256 {
            return Err(Error::InvalidInput);
        }
        filter.validate()?;
        let (account, folder, inbox) = scope.parts(accounts)?;
        let mut query = self.connection.prepare_cached(&format!("SELECT m.id,m.blob_hash FROM messages m
         WHERE m.account_id IN (SELECT value FROM json_each(?1)) AND (?2 IS NULL OR m.account_id=?2)
         AND m.omitted IS NULL AND COALESCE(json_extract(m.metadata,'$.status.encrypted_unsupported'),0)=0
         AND EXISTS(SELECT 1 FROM message_search t WHERE t.rowid=m.id)
         AND (?5 IS NULL OR m.received_at>=?5) AND (?6 IS NULL OR m.received_at<?6)
         AND CASE WHEN ?7='' THEN 1 ELSE instr(lower({SENDER_TEXT}),lower(?7))>0 END
         AND EXISTS(SELECT 1 FROM effective_memberships s JOIN folders f ON f.id=s.folder_id
          AND f.account_id=s.account_id AND f.uidvalidity=s.uidvalidity
          WHERE s.message_id=m.id AND s.account_id=m.account_id AND (?3 IS NULL OR f.id=?3) AND (?4=0 OR f.is_inbox=1))"))?;
        let rows = query.query_map(
            params![
                serde_json::to_string(accounts)?,
                account,
                folder,
                inbox,
                filter.received_from,
                filter.received_before,
                filter.sender
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::super::mailbox::{Flags, Membership};
    use super::*;
    use core::num::NonZeroU32;

    fn folder(store: &mut Store, account: &str, name: &str, inbox: bool) -> i64 {
        store.ensure_account(account).expect("account");
        let folder = store
            .ensure_folder(account, name, inbox)
            .expect("folder")
            .id;
        store
            .select_folder(account, folder, NonZeroU32::new(42).expect("namespace"))
            .expect("selected");
        folder
    }
    fn cache(store: &mut Store, account: &str, folder: i64, uid: u32, mime: &str) -> i64 {
        store.cache_message(account, Membership { folder, uid_validity: NonZeroU32::new(42).expect("namespace"), uid: NonZeroU32::new(uid).expect("UID"), flags: Flags::default() }, i64::from(uid)*100, None,
        format!("From: Alpha <alpha@example.test>\r\nTo: reader@example.test\r\nSubject: Notice {uid}\r\nMessage-ID: <{account}-{uid}@example.test>\r\n{mime}").as_bytes()).expect("cached")
    }
    fn fixture() -> (tempfile::TempDir, Store, i64) {
        let root = tempfile::tempdir().expect("private store");
        let mut store = Store::open(root.path()).expect("store");
        let inbox = folder(&mut store, "a", "INBOX", true);
        (root, store, inbox)
    }
    #[test]
    fn independent_frontiers_survive_lexical_acknowledgment_replay_and_deletion() {
        let (_root, mut store, inbox) = fixture();
        let id = cache(&mut store, "a", inbox, 1, "\r\nReadable body.");
        let batch = store.semantic_pending(1).expect("semantic change");
        assert_eq!(batch.queued, 1);
        assert_eq!(batch.items[0].id, id);
        let source = batch.items[0].source.as_ref().expect("readable");
        assert_eq!(source.account, "a");
        assert_eq!(source.body, "Readable body.");
        assert_eq!(source.revision.len(), 64);
        let lexical = store.search_pending(1).expect("lexical");
        store
            .search_indexed(lexical.through.expect("lexical cursor"))
            .expect("lexical acknowledgment");
        assert_eq!(store.semantic_pending(1).expect("independent"), batch);
        store.requeue_semantic().expect("model revision changed");
        store
            .semantic_indexed(batch.through.expect("old semantic cursor"))
            .expect("old acknowledgment");
        assert_eq!(
            store.semantic_pending(1).expect("replay retained").queued,
            1
        );
        assert_eq!(
            store.search_pending(1).expect("lexical unchanged").queued,
            0
        );
        let through = store
            .semantic_pending(1)
            .expect("batch")
            .through
            .expect("cursor");
        store
            .semantic_indexed(through)
            .expect("durable vector acknowledged");
        store.clear_cache("a").expect("remove unowned mail");
        assert_eq!(
            store
                .semantic_pending(1)
                .expect("deletion retained after lexical cleanup")
                .items,
            [Item { id, source: None }]
        );
        assert!(store.semantic_pending(0).is_err());
        assert!(store.semantic_pending(33).is_err());
    }
    #[test]
    fn upgrading_schema_fourteen_seeds_semantic_work_without_requeueing_lexical_mail() {
        let (root, mut store, inbox) = fixture();
        let id = cache(&mut store, "a", inbox, 1, "\r\nExisting readable mail.");
        let lexical = store.search_pending(1).expect("lexical batch");
        store
            .search_indexed(lexical.through.expect("cursor"))
            .expect("lexical already indexed");
        let unread = store.unread_counts(&["a".into()]).expect("unread");
        store.connection.execute_batch("ALTER TABLE accounts DROP COLUMN move_unavailable; DROP TABLE deadline_corrections; DROP TRIGGER semantic_queued; DROP TABLE semantic_pending; PRAGMA user_version=14;").expect("previous schema");
        drop(store);
        let upgraded = Store::open(root.path()).expect("upgrade with backup");
        assert_eq!(
            upgraded
                .search_pending(1)
                .expect("lexical unchanged")
                .queued,
            0
        );
        let batch = upgraded.semantic_pending(1).expect("new frontier");
        assert_eq!(batch.queued, 1);
        assert_eq!(batch.items[0].id, id);
        assert_eq!(
            batch.items[0].source.as_ref().expect("readable").body,
            "Existing readable mail."
        );
        assert_eq!(
            upgraded.unread_counts(&["a".into()]).expect("still unread"),
            unread
        );
    }

    #[test]
    fn scope_and_sender_dates_are_canonical_and_do_not_change_seen() {
        let (_root, mut store, inbox) = fixture();
        let sent = folder(&mut store, "a", "Sent", false);
        let foreign = folder(&mut store, "b", "INBOX", true);
        let first = cache(&mut store, "a", inbox, 1, "\r\nDoctoral registration.");
        let second = cache(&mut store, "a", sent, 2, "\r\nSent ancestry.");
        cache(&mut store, "b", foreign, 3, "\r\nForeign copy.");
        let accounts = vec!["a".to_owned()];
        let before = store.unread_counts(&accounts).expect("unread");
        let keys = store
            .semantic_candidates(&accounts, &Scope::Inbox, &SearchFilter::default())
            .expect("inbox");
        assert_eq!(keys.keys().copied().collect::<Vec<_>>(), [first]);
        let mut filter = SearchFilter {
            received_from: Some(150),
            received_before: Some(250),
            sender: "ALPHA@EXAMPLE.TEST".into(),
        };
        let scope = Scope::Account("a".into());
        assert_eq!(
            store
                .semantic_candidates(&accounts, &scope, &filter)
                .expect("metadata restrictions")
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [second]
        );
        filter.sender = "reader@example.test".into();
        assert_eq!(
            store
                .semantic_candidates(&accounts, &scope, &filter)
                .expect("recipient excluded"),
            BTreeMap::new()
        );
        assert_eq!(
            store
                .semantic_candidates(&[], &Scope::Inbox, &SearchFilter::default())
                .expect("disabled"),
            BTreeMap::new()
        );
        assert!(
            store
                .semantic_candidates(
                    &accounts,
                    &Scope::Account("b".into()),
                    &SearchFilter::default()
                )
                .is_err()
        );
        assert_eq!(
            store.unread_counts(&accounts).expect("still unread"),
            before
        );
        store
            .select_folder("a", inbox, NonZeroU32::new(99).expect("new namespace"))
            .expect("UID reset");
        assert_eq!(
            store
                .semantic_candidates(&accounts, &Scope::Inbox, &SearchFilter::default())
                .expect("stale membership excluded"),
            BTreeMap::new()
        );
    }
    #[test]
    fn a_source_changed_after_retrieval_is_fenced_without_losing_its_cursor_rank() {
        let (_root, mut store, inbox) = fixture();
        let first = cache(&mut store, "a", inbox, 1, "\r\nFirst meaning match.");
        let second = cache(&mut store, "a", inbox, 2, "\r\nSecond meaning match.");
        let accounts = vec!["a".into()];
        let unread = store.unread_counts(&accounts).expect("unread");
        let mut sources = store
            .semantic_candidates(&accounts, &Scope::Inbox, &SearchFilter::default())
            .expect("retrieval snapshot");
        let mut page = super::super::browse::FilteredPage {
            accounts,
            scope: Scope::Inbox,
            query: "cross-language phrase".into(),
            hits: Some(vec![second, first]),
            filter: SearchFilter::default(),
            after: None,
            limit: 1,
        };
        let before = store
            .revisioned_conversation_page(&page, &sources)
            .expect("ranked original");
        assert_eq!(before[0].latest.id, second);
        let cursor = before[0].latest.cursor();
        // A replacement arrives between vector retrieval and canonical presentation.
        let replacement = "b".repeat(64);
        store
            .connection
            .execute(
                "UPDATE messages SET blob_hash=?2 WHERE id=?1",
                params![second, replacement],
            )
            .expect("source changed");
        assert_eq!(
            store
                .revisioned_conversation_page(&page, &sources)
                .expect("stale source excluded")[0]
                .latest
                .id,
            first
        );
        page.after = Some(cursor);
        assert_eq!(
            store
                .revisioned_conversation_page(&page, &sources)
                .expect("old rank cursor retained")[0]
                .latest
                .id,
            first
        );
        page.after = None;
        sources.insert(second, replacement);
        assert_eq!(
            store
                .revisioned_conversation_page(&page, &sources)
                .expect("fresh source positive control")[0]
                .latest
                .id,
            second
        );
        sources.remove(&second);
        assert!(store.revisioned_conversation_page(&page, &sources).is_err());
        assert_eq!(
            store
                .unread_counts(&page.accounts)
                .expect("no read side effects"),
            unread
        );
    }

    #[test]
    fn bounded_unicode_sources_report_coverage_and_exclude_encrypted_or_headers_only_mail() {
        let (_root, mut store, inbox) = fixture();
        let long = cache(
            &mut store,
            "a",
            inbox,
            1,
            &format!(
                "Content-Type: text/plain; charset=utf-8\r\n\r\n{}",
                "é".repeat(40000)
            ),
        );
        let encrypted = cache(
            &mut store,
            "a",
            inbox,
            2,
            "Content-Type: application/pkcs7-mime\r\n\r\nopaque encrypted bytes",
        );
        let outline = cache(
            &mut store,
            "a",
            inbox,
            3,
            "\r\nBody not retained in this outline.",
        );
        store
            .connection
            .execute("UPDATE messages SET omitted='{}' WHERE id=?1", [outline])
            .expect("headers-only state");
        let batch = store.semantic_pending(32).expect("batch");
        let source = batch
            .items
            .iter()
            .find(|item| item.id == long)
            .expect("long message")
            .source
            .as_ref()
            .expect("readable");
        assert_eq!(source.body.len(), TEXT_BYTES);
        assert!(source.truncated);
        assert!(
            batch
                .items
                .iter()
                .filter(|item| [encrypted, outline].contains(&item.id))
                .all(|item| item.source.is_none())
        );
        assert_eq!(
            store
                .semantic_candidates(&["a".into()], &Scope::Inbox, &SearchFilter::default())
                .expect("readable candidates")
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [long]
        );
    }
}
