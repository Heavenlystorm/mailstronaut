//! The queue that keeps the rebuildable search index in step with cached mail. It lives
//! in the same transactions as the mail itself, so that the index can only ever lag.

use super::{Error, Store};

mod excerpts;

// Both query paths use the same parsed From metadata. Do not include To/Cc here:
// a sender restriction must not match a recipient, nor lose named senders' addresses.
pub(super) const SENDER_TEXT: &str = "m.sender || ' ' || COALESCE((
 SELECT group_concat(COALESCE(json_extract(sender.value,'$.name'),'') || ' ' ||
 COALESCE(json_extract(sender.value,'$.address'),''),' ')
 FROM json_each(m.metadata,'$.headers.from') sender), '')";

pub(super) const SCHEMA: &str = "
CREATE TABLE search_pending (seq INTEGER PRIMARY KEY AUTOINCREMENT, message_id INTEGER NOT NULL UNIQUE);
INSERT INTO search_pending(message_id) SELECT id FROM messages;
CREATE TRIGGER search_message_removed AFTER DELETE ON messages BEGIN
 INSERT OR REPLACE INTO search_pending(message_id) VALUES (old.id);
END;
";

/// A message's searchable text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchText {
    /// Owning account.
    pub account: String,
    /// When the message arrived.
    pub received_at: i64,
    /// Subject line.
    pub subject: String,
    /// Parsed From names and addresses, with the displayed sender as a fallback.
    pub sender: String,
    /// Every address on the message.
    pub addresses: String,
    /// Readable body, already bounded by the MIME projection.
    pub body: String,
}

/// One queued message; without text, it must leave the index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchItem {
    /// Canonical message ID.
    pub id: i64,
    /// Current text, if the message still has any.
    pub text: Option<SearchText>,
}

/// The oldest queued changes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchBatch {
    /// Up to the requested number of messages.
    pub items: Vec<SearchItem>,
    /// Acknowledge through this position once the index has committed them.
    pub through: Option<i64>,
    /// Everything queued, including this batch.
    pub queued: u64,
}

impl Store {
    /// Canonical original IDs in the enabled account/folder/inbox scope.
    /// Supply these to the text collector before its hit limit; recheck scope
    /// when projecting results because memberships may change during a search.
    /// This reads identifiers only, including headers-only cached mail.
    /// # Errors
    /// Rejects invalid/disabled scopes and storage failures.
    pub fn search_scope(
        &self,
        accounts: &[String],
        scope: &super::browse::Scope,
    ) -> Result<std::collections::BTreeSet<i64>, Error> {
        if accounts.len() > 256 {
            return Err(Error::InvalidInput);
        }
        let (account, folder, inbox) = scope.parts(accounts)?;
        let mut query = self.connection.prepare_cached(
            "SELECT m.id FROM messages m
             WHERE m.account_id IN (SELECT value FROM json_each(?1))
             AND (?2 IS NULL OR m.account_id=?2)
             AND EXISTS(SELECT 1 FROM effective_memberships s JOIN folders f
              ON f.id=s.folder_id AND f.account_id=s.account_id AND f.uidvalidity=s.uidvalidity
              WHERE s.message_id=m.id AND s.account_id=m.account_id
              AND (?3 IS NULL OR f.id=?3) AND (?4=0 OR f.is_inbox=1))",
        )?;
        query
            .query_map(
                rusqlite::params![serde_json::to_string(accounts)?, account, folder, inbox],
                |row| row.get(0),
            )?
            .collect::<Result<_, _>>()
            .map_err(Error::from)
    }
    /// Read the oldest queued changes without removing them.
    /// # Errors
    /// Rejects an empty batch size and reports storage failures.
    pub fn search_pending(&self, limit: u32) -> Result<SearchBatch, Error> {
        if limit == 0 {
            return Err(Error::InvalidInput);
        }
        let queued =
            self.connection
                .query_row("SELECT COUNT(*) FROM search_pending", [], |row| {
                    row.get::<_, i64>(0)
                })?;
        let mut query = self.connection.prepare_cached(&format!(
            "SELECT p.seq,p.message_id,m.account_id,m.received_at,f.subject,{SENDER_TEXT},f.addresses,f.body
             FROM search_pending p
             LEFT JOIN messages m ON m.id=p.message_id
             LEFT JOIN message_search f ON f.rowid=p.message_id
             ORDER BY p.seq LIMIT ?1",
        ))?;
        let mut batch = SearchBatch {
            queued: u64::try_from(queued).unwrap_or_default(),
            ..SearchBatch::default()
        };
        let rows = query.query_map([limit], |row| {
            let seq: i64 = row.get(0)?;
            let id: i64 = row.get(1)?;
            let account: Option<String> = row.get(2)?;
            let subject: Option<String> = row.get(4)?;
            let text = match (account, subject) {
                (Some(account), Some(subject)) => Some(SearchText {
                    account,
                    received_at: row.get(3)?,
                    subject,
                    sender: row.get(5)?,
                    addresses: row.get(6)?,
                    body: row.get(7)?,
                }),
                _ => None,
            };
            Ok((seq, SearchItem { id, text }))
        })?;
        for row in rows {
            let (seq, item) = row?;
            batch.through = Some(seq);
            batch.items.push(item);
        }
        Ok(batch)
    }

    /// Forget changes the index has committed. A message queued again since it was read
    /// has a later position and stays queued.
    /// # Errors
    /// Reports storage failures.
    pub fn search_indexed(&mut self, through: i64) -> Result<(), Error> {
        self.connection
            .execute("DELETE FROM search_pending WHERE seq<=?1", [through])?;
        Ok(())
    }

    /// Queue every message again, for an index that had to be recreated.
    /// # Errors
    /// Reports storage failures.
    pub fn requeue_search(&mut self) -> Result<(), Error> {
        self.connection.execute(
            "INSERT OR REPLACE INTO search_pending(message_id) SELECT id FROM messages",
            [],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use core::num::NonZeroU32;

    use super::super::mailbox::{Flags, Membership};
    use super::*;

    fn cache(store: &mut Store, folder: i64, uid: u32, body: &str) -> i64 {
        store
            .cache_message(
                "a",
                Membership {
                    folder,
                    uid_validity: NonZeroU32::new(42).expect("namespace"),
                    uid: NonZeroU32::new(uid).expect("UID"),
                    flags: Flags::default(),
                },
                i64::from(uid),
                None,
                format!(
                    "From: Sender <sender@example.test>\r\nTo: reader@example.test\r\nSubject: Subject {uid}\r\nMessage-ID: <{uid}@example.test>\r\n\r\n{body}"
                )
                .as_bytes(),
            )
            .expect("message")
    }

    fn fixture() -> (tempfile::TempDir, Store, i64) {
        let root = tempfile::tempdir().expect("fixture");
        let mut store = Store::open(root.path()).expect("open");
        store.ensure_account("a").expect("account");
        let folder = store.ensure_folder("a", "INBOX", true).expect("folder").id;
        store
            .select_folder("a", folder, NonZeroU32::new(42).expect("namespace"))
            .expect("generation");
        (root, store, folder)
    }

    #[test]
    fn scope_candidates_follow_current_memberships_and_enabled_accounts_without_reading() {
        use super::super::browse::Scope;
        let (_root, mut store, inbox) = fixture();
        let first = cache(&mut store, inbox, 1, "Inbox original.");
        let sent = store.ensure_folder("a", "Sent", false).expect("Sent").id;
        store
            .select_folder("a", sent, NonZeroU32::new(42).expect("namespace"))
            .expect("select Sent");
        let second = cache(&mut store, sent, 2, "Sent original.");
        let accounts = vec!["a".into()];
        assert_eq!(
            store.search_scope(&accounts, &Scope::Inbox).expect("inbox"),
            [first].into()
        );
        assert_eq!(
            store
                .search_scope(&accounts, &Scope::Account("a".into()))
                .expect("account"),
            [first, second].into()
        );
        assert_eq!(
            store
                .search_scope(
                    &accounts,
                    &Scope::Folder {
                        account: "a".into(),
                        folder: sent
                    }
                )
                .expect("Sent"),
            [second].into()
        );
        assert_eq!(
            store.search_scope(&[], &Scope::Inbox).expect("no accounts"),
            std::collections::BTreeSet::new()
        );
        assert!(
            store
                .search_scope(&[], &Scope::Account("a".into()))
                .is_err()
        );
        store.ensure_account("b").expect("other account");
        let foreign = store
            .ensure_folder("b", "Sent", false)
            .expect("foreign folder")
            .id;
        assert_eq!(
            store
                .search_scope(
                    &accounts,
                    &Scope::Folder {
                        account: "a".into(),
                        folder: foreign
                    }
                )
                .expect("foreign empty"),
            std::collections::BTreeSet::new()
        );
        assert_eq!(
            store
                .connection
                .query_row("SELECT count(*) FROM memberships WHERE seen=1", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("flags"),
            0
        );
        store
            .select_folder("a", inbox, NonZeroU32::new(43).expect("new namespace"))
            .expect("reset UIDVALIDITY");
        assert_eq!(
            store
                .search_scope(&accounts, &Scope::Inbox)
                .expect("retired memberships"),
            std::collections::BTreeSet::new()
        );
    }

    #[test]
    fn cached_mail_is_queued_with_its_text_and_stays_queued_until_acknowledged() {
        let (_root, mut store, folder) = fixture();
        let first = cache(&mut store, folder, 1, "First body.");
        let second = cache(&mut store, folder, 2, "Second body.");
        let batch = store.search_pending(1).expect("batch");
        assert_eq!(batch.queued, 2);
        assert_eq!(batch.items.len(), 1);
        assert_eq!(batch.items[0].id, first);
        let text = batch.items[0].text.as_ref().expect("text");
        assert_eq!(text.account, "a");
        assert_eq!(text.subject, "Subject 1");
        assert!(text.sender.contains("Sender"));
        assert!(text.sender.contains("sender@example.test"));
        assert!(!text.sender.contains("reader@example.test"));
        assert!(text.addresses.contains("reader@example.test"));
        assert!(text.body.contains("First body."));
        assert_eq!(store.search_pending(1).expect("again").items[0].id, first);
        store
            .search_indexed(batch.through.expect("position"))
            .expect("acknowledge");
        let batch = store.search_pending(10).expect("rest");
        assert_eq!(batch.queued, 1);
        assert_eq!(batch.items[0].id, second);
    }

    #[test]
    fn a_message_changed_while_being_indexed_stays_queued() {
        let (_root, mut store, folder) = fixture();
        let first = cache(&mut store, folder, 1, "Original.");
        let batch = store.search_pending(10).expect("batch");
        // The message is queued again before the index acknowledges the earlier read.
        store.requeue_search().expect("requeue");
        store
            .search_indexed(batch.through.expect("position"))
            .expect("acknowledge");
        let batch = store.search_pending(10).expect("still queued");
        assert_eq!(batch.items.len(), 1);
        assert_eq!(batch.items[0].id, first);
    }

    #[test]
    fn deleted_mail_and_cleared_indexes_are_queued_for_removal() {
        let (_root, mut store, folder) = fixture();
        let first = cache(&mut store, folder, 1, "One.");
        let second = cache(&mut store, folder, 2, "Two.");
        let through = store.search_pending(10).expect("batch").through;
        store
            .search_indexed(through.expect("position"))
            .expect("acknowledge");
        store
            .connection
            .execute_batch(&format!(
                "DELETE FROM memberships WHERE message_id={first}; DELETE FROM message_search WHERE rowid={first}; DELETE FROM messages WHERE id={first};"
            ))
            .expect("delete");
        let batch = store.search_pending(10).expect("removal");
        assert_eq!(
            batch.items,
            [SearchItem {
                id: first,
                text: None
            }]
        );
        store
            .search_indexed(batch.through.expect("position"))
            .expect("acknowledge");
        store.clear_rebuildable_indexes().expect("clear");
        assert_eq!(
            store.search_pending(10).expect("cleared").items,
            [SearchItem {
                id: second,
                text: None
            }]
        );
    }

    #[test]
    fn a_recreated_index_gets_everything_again() {
        let (_root, mut store, folder) = fixture();
        cache(&mut store, folder, 1, "One.");
        cache(&mut store, folder, 2, "Two.");
        let through = store.search_pending(10).expect("batch").through;
        store
            .search_indexed(through.expect("position"))
            .expect("acknowledge");
        assert_eq!(store.search_pending(10).expect("empty").queued, 0);
        store.requeue_search().expect("requeue");
        assert_eq!(store.search_pending(10).expect("batch").queued, 2);
    }

    #[test]
    fn nothing_queued_has_no_acknowledgment_and_a_zero_batch_is_refused() {
        let (_root, store, _folder) = fixture();
        let batch = store.search_pending(10).expect("batch");
        assert_eq!(batch.through, None);
        assert_eq!(batch.items, []);
        assert!(store.search_pending(0).is_err());
    }
}
