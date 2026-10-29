//! Revision-fenced account-local conversation identities and normalized ancestry.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{OptionalExtension as _, TransactionBehavior, params};

use super::{Error, Store};
use crate::{
    mime::Projection,
    threads::{self, Groups, Message},
};

pub(super) const SCHEMA:&str="
ALTER TABLE accounts ADD COLUMN thread_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE accounts ADD COLUMN projected_thread_revision INTEGER;
ALTER TABLE messages ADD COLUMN thread_references TEXT NOT NULL DEFAULT '[]';
ALTER TABLE messages ADD COLUMN thread_parent TEXT;
ALTER TABLE messages ADD COLUMN thread_routing_valid INTEGER NOT NULL DEFAULT 1 CHECK(thread_routing_valid IN (0,1));
UPDATE messages SET
 thread_references=CASE WHEN json_valid(metadata) THEN COALESCE(json_extract(metadata,'$.headers.references'),'[]') ELSE '[]' END,
 thread_parent=CASE WHEN json_valid(metadata) THEN CASE WHEN json_array_length(metadata,'$.headers.in_reply_to')=1 THEN json_extract(metadata,'$.headers.in_reply_to[0]') ELSE NULL END ELSE NULL END,
 thread_routing_valid=CASE WHEN json_valid(metadata) THEN NOT COALESCE(json_extract(metadata,'$.status.headers_truncated'),0) ELSE 1 END;
CREATE TRIGGER thread_message_insert AFTER INSERT ON messages BEGIN
 UPDATE accounts SET thread_revision=thread_revision+1 WHERE id=NEW.account_id;
END;
CREATE TRIGGER thread_message_change AFTER UPDATE OF message_id,thread_references,thread_parent,thread_routing_valid,received_at ON messages
WHEN OLD.message_id IS NOT NEW.message_id OR OLD.thread_references IS NOT NEW.thread_references OR OLD.thread_parent IS NOT NEW.thread_parent OR OLD.thread_routing_valid IS NOT NEW.thread_routing_valid OR OLD.received_at IS NOT NEW.received_at BEGIN
 UPDATE accounts SET thread_revision=thread_revision+1 WHERE id=NEW.account_id;
END;
CREATE TRIGGER thread_message_delete AFTER DELETE ON messages BEGIN
 UPDATE accounts SET thread_revision=thread_revision+1 WHERE id=OLD.account_id;
END;
CREATE VIEW effective_memberships AS
 SELECT s.account_id,s.folder_id,s.uidvalidity,s.uid,s.message_id,
  COALESCE(seen.desired,s.seen) AS seen,COALESCE(starred.desired,s.starred) AS starred
 FROM memberships s
 LEFT JOIN pending_operations seen ON seen.account_id=s.account_id AND seen.folder_id=s.folder_id AND seen.uidvalidity=s.uidvalidity AND seen.uid=s.uid AND seen.message_id=s.message_id AND seen.kind='seen' AND seen.state IN ('pending','submitted','failed')
 LEFT JOIN pending_operations starred ON starred.account_id=s.account_id AND starred.folder_id=s.folder_id AND starred.uidvalidity=s.uidvalidity AND starred.uid=s.uid AND starred.message_id=s.message_id AND starred.kind='starred' AND starred.state IN ('pending','submitted','failed');
CREATE TABLE conversations (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 account_id TEXT NOT NULL REFERENCES accounts(id),
 merged_into INTEGER,
 UNIQUE(account_id,id),
 CHECK(merged_into IS NULL OR merged_into<id),
 FOREIGN KEY(account_id,merged_into) REFERENCES conversations(account_id,id)
);
CREATE TABLE conversation_members (
 account_id TEXT NOT NULL,
 message_id INTEGER NOT NULL,
 conversation_id INTEGER NOT NULL,
 PRIMARY KEY(account_id,message_id),
 FOREIGN KEY(account_id,message_id) REFERENCES messages(account_id,id) ON DELETE CASCADE,
 FOREIGN KEY(account_id,conversation_id) REFERENCES conversations(account_id,id)
);
CREATE INDEX conversation_messages ON conversation_members(account_id,conversation_id,message_id);
";

/// One immutable metadata snapshot. It contains no bodies, recipients or subjects.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// Stable configuration account ID.
    pub account: String,
    /// Account-local metadata generation; commits reject stale helper output.
    pub revision: i64,
    /// The complete bounded account dataset, including cached Sent/Archive roots.
    pub messages: Vec<Message>,
}

/// Extract already-parsed routing metadata without building a second MIME parser.
/// Missing/ambiguous/oversized tokens are omitted, never guessed or sliced.
pub(super) fn routing(projection: &Projection) -> Result<(String, Option<&str>, bool), Error> {
    let valid = !projection.status.headers_truncated;
    let references = if valid {
        projection
            .headers
            .references
            .iter()
            .filter(|id| mailstronaut_mail::compose::valid_id(id))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let parent = valid
        .then_some(&projection.headers.in_reply_to)
        .filter(|ids| ids.len() == 1)
        .and_then(|ids| ids.first())
        .filter(|id| mailstronaut_mail::compose::valid_id(id))
        .map(String::as_str);
    Ok((serde_json::to_string(&references)?, parent, valid))
}

impl Store {
    /// Read a bounded complete account snapshot only when its projection is stale.
    /// All cached messages participate, allowing Sent/Archive and missing-inbox
    /// ancestors to connect replies without treating subjects as identity.
    ///
    /// # Errors
    /// Rejects excess metadata, malformed stored routing and storage failures.
    pub fn conversation_snapshot(&self, account: &str) -> Result<Option<Snapshot>, Error> {
        let versions: Option<(i64, Option<i64>)> = self
            .connection
            .query_row(
                "SELECT thread_revision,projected_thread_revision FROM accounts WHERE id=?1",
                [account],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((revision, projected)) = versions else {
            return Ok(None);
        };
        if projected == Some(revision) {
            return Ok(None);
        }
        let mut query=self.connection.prepare("SELECT id,message_id,thread_parent,thread_references,received_at,thread_routing_valid FROM messages WHERE account_id=?1 ORDER BY id LIMIT ?2")?;
        let mut rows = query.query(params![
            account,
            i64::try_from(threads::MAX_MESSAGES + 1).map_err(|_| Error::InvalidInput)?
        ])?;
        let mut messages = Vec::new();
        let mut bytes = 0_usize;
        while let Some(row) = rows.next()? {
            if messages.len() == threads::MAX_MESSAGES {
                return Err(Error::InvalidInput);
            }
            let raw: String = row.get(3)?;
            // Each identifier was limited before persistence. Also refuse corrupt
            // oversized index values before deserializing them into many strings.
            if raw.len() > 1024 * 1024 {
                return Err(Error::InvalidInput);
            }
            bytes = bytes
                .checked_add(raw.len() + 192)
                .ok_or(Error::InvalidInput)?;
            if bytes > threads::MAX_INPUT_BYTES {
                return Err(Error::InvalidInput);
            }
            let valid: bool = row.get(5)?;
            let mut message = Message {
                id: row.get(0)?,
                message_id: row.get(1)?,
                parent: row.get(2)?,
                references: serde_json::from_str(&raw)?,
                received_at: row.get(4)?,
            };
            for value in [&mut message.message_id, &mut message.parent] {
                if !valid
                    || value
                        .as_deref()
                        .is_some_and(|id| !mailstronaut_mail::compose::valid_id(id))
                {
                    *value = None;
                }
            }
            if valid {
                message
                    .references
                    .retain(|id| mailstronaut_mail::compose::valid_id(id));
            } else {
                message.references.clear();
            }
            messages.push(message);
        }
        threads::validate(&messages).map_err(|_| Error::InvalidInput)?;
        Ok(Some(Snapshot {
            account: account.into(),
            revision,
            messages,
        }))
    }

    /// Replace one account's derived grouping only for the unchanged snapshot.
    /// The oldest surviving conversation ID wins a merge; old IDs remain aliases.
    /// A split deterministically retains an ID for the oldest local member only.
    ///
    /// # Errors
    /// Rejects stale/incomplete/mixed-account output and storage failures.
    pub fn commit_conversations(
        &mut self,
        snapshot: &Snapshot,
        groups: &Groups,
    ) -> Result<(), Error> {
        groups
            .validate(&snapshot.messages)
            .map_err(|_| Error::InvalidInput)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision: Option<i64> = tx
            .query_row(
                "SELECT thread_revision FROM accounts WHERE id=?1",
                [&snapshot.account],
                |row| row.get(0),
            )
            .optional()?;
        if revision != Some(snapshot.revision) {
            return Err(Error::OperationConflict);
        }
        let expected = {
            let mut statement =
                tx.prepare("SELECT id FROM messages WHERE account_id=?1 ORDER BY id")?;
            statement
                .query_map([&snapshot.account], |row| row.get::<_, i64>(0))?
                .collect::<Result<BTreeSet<_>, _>>()?
        };
        if expected != snapshot.messages.iter().map(|message| message.id).collect() {
            return Err(Error::OperationConflict);
        }
        let old = {
            let mut statement = tx.prepare(
                "SELECT message_id,conversation_id FROM conversation_members WHERE account_id=?1",
            )?;
            statement
                .query_map([&snapshot.account], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                })?
                .collect::<Result<BTreeMap<_, _>, _>>()?
        };
        let mut ordered = groups.groups.iter().collect::<Vec<_>>();
        ordered.sort_unstable_by_key(|group| group.iter().min().copied());
        // Reserve each prior identity for the new group containing its oldest
        // surviving member before considering merges. A simultaneous split/merge
        // must not steal an identity merely because another group's ID sorts first.
        let mut anchors = BTreeMap::new();
        for (message, conversation) in &old {
            anchors.entry(*conversation).or_insert(*message);
        }
        let mut assignments = Vec::with_capacity(ordered.len());
        for group in ordered {
            let candidates = group
                .iter()
                .filter_map(|message| {
                    old.get(message)
                        .filter(|conversation| anchors.get(*conversation) == Some(message))
                        .copied()
                })
                .collect::<BTreeSet<_>>();
            let conversation = if let Some(oldest) = candidates.first() {
                *oldest
            } else {
                tx.execute(
                    "INSERT INTO conversations(account_id) VALUES(?1)",
                    [&snapshot.account],
                )?;
                tx.last_insert_rowid()
            };
            let losers = candidates
                .iter()
                .copied()
                .filter(|id| *id != conversation)
                .collect::<Vec<_>>();
            if !losers.is_empty() {
                let losers = serde_json::to_string(&losers)?;
                tx.execute("UPDATE conversations SET merged_into=?2 WHERE account_id=?1 AND (id IN (SELECT value FROM json_each(?3)) OR merged_into IN (SELECT value FROM json_each(?3)))",params![snapshot.account,conversation,losers])?;
            }
            assignments.push((conversation, group));
        }
        tx.execute(
            "DELETE FROM conversation_members WHERE account_id=?1",
            [&snapshot.account],
        )?;
        {
            let mut insert=tx.prepare_cached("INSERT INTO conversation_members(account_id,message_id,conversation_id) VALUES(?1,?2,?3)")?;
            for (conversation, messages) in assignments {
                for message in messages {
                    insert.execute(params![snapshot.account, message, conversation])?;
                }
            }
        }
        tx.execute(
            "UPDATE accounts SET projected_thread_revision=thread_revision WHERE id=?1",
            [&snapshot.account],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Resolve a preserved conversation identity after any number of merges.
    ///
    /// # Errors
    /// Rejects unknown/cross-account IDs or invalid stored alias chains.
    pub fn resolve_conversation(&self, account: &str, id: i64) -> Result<i64, Error> {
        self.connection
            .query_row(
                "SELECT COALESCE(merged_into,id) FROM conversations WHERE account_id=?1 AND id=?2",
                params![account, id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(Error::MissingMessage)
    }
}

/// One account-local inbox conversation with a concrete latest-message identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Summary {
    /// Latest inbox member; its own read/star flags are not replaced by aggregates.
    pub latest: super::mailbox::Summary,
    /// Stable projected conversation ID, None while grouping is unavailable/stale.
    pub conversation: Option<i64>,
    /// Cached members with current mailbox memberships, including Sent/Archive.
    pub cached_messages: u32,
    /// Unread inbox messages, never a command to mark every member read.
    pub unread: u32,
    /// Any inbox member is starred.
    pub starred: bool,
}

impl Store {
    /// Page account-local inbox conversations once, using latest inbox activity.
    /// Stale/unavailable projection falls back to distinct messages explicitly.
    ///
    /// # Errors
    /// Rejects oversized account/page selections and storage failures.
    pub fn conversation_inbox(
        &self,
        accounts: &[String],
        after: Option<super::mailbox::Cursor>,
        limit: u32,
    ) -> Result<Vec<Summary>, Error> {
        if accounts.len() > 256 || !(1..=100).contains(&limit) {
            return Err(Error::InvalidInput);
        }
        let selected = serde_json::to_string(accounts)?;
        let mut statement=self.connection.prepare_cached("
WITH inbox AS (
 SELECT m.id,m.account_id,m.subject,m.sender,m.preview,m.received_at,MIN(s.seen) AS seen,MAX(s.starred) AS starred
 FROM messages m JOIN effective_memberships s ON s.message_id=m.id AND s.account_id=m.account_id
 JOIN folders f ON f.id=s.folder_id AND f.account_id=s.account_id AND f.uidvalidity=s.uidvalidity
 WHERE f.is_inbox=1 AND m.account_id IN (SELECT value FROM json_each(?1)) GROUP BY m.id
), tagged AS (
 SELECT i.*,CASE WHEN a.projected_thread_revision=a.thread_revision THEN cm.conversation_id ELSE NULL END AS conversation,
 CASE WHEN a.projected_thread_revision=a.thread_revision AND cm.conversation_id IS NOT NULL THEN cm.conversation_id ELSE -i.id END AS group_key
 FROM inbox i JOIN accounts a ON a.id=i.account_id LEFT JOIN conversation_members cm ON cm.account_id=i.account_id AND cm.message_id=i.id
), ranked AS (
 SELECT *,SUM(NOT seen) OVER(PARTITION BY account_id,group_key) AS unread_count,
 MAX(starred) OVER(PARTITION BY account_id,group_key) AS thread_starred,
 ROW_NUMBER() OVER(PARTITION BY account_id,group_key ORDER BY received_at DESC,id DESC) AS position FROM tagged
)
SELECT id,account_id,subject,sender,preview,received_at,seen,starred,conversation,
 CASE WHEN conversation IS NULL THEN 1 ELSE (
 SELECT COUNT(*) FROM conversation_members cm WHERE cm.account_id=r.account_id AND cm.conversation_id=r.conversation
 AND EXISTS(SELECT 1 FROM memberships s JOIN folders f ON f.id=s.folder_id AND f.account_id=s.account_id AND f.uidvalidity=s.uidvalidity WHERE s.account_id=cm.account_id AND s.message_id=cm.message_id)) END,
 unread_count,thread_starred FROM ranked r
WHERE position=1 AND (?2 IS NULL OR received_at<?2 OR (received_at=?2 AND id<?3))
ORDER BY received_at DESC,id DESC LIMIT ?4")?;
        statement
            .query_map(
                params![
                    selected,
                    after.map(|cursor| cursor.received_at),
                    after.map(|cursor| cursor.message_id),
                    limit
                ],
                |row| {
                    Ok(Summary {
                        latest: super::mailbox::read_summary(row)?,
                        conversation: row.get(8)?,
                        cached_messages: row.get(9)?,
                        unread: row.get(10)?,
                        starred: row.get(11)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    /// Read up to 200 cached members for one selected message's current grouping.
    /// Includes Sent/Archive, never expands across configuration account IDs.
    ///
    /// # Errors
    /// Returns missing-message, invalid projection or storage failures.
    pub fn conversation_messages(
        &self,
        account: &str,
        message: i64,
    ) -> Result<Vec<super::mailbox::Summary>, Error> {
        let conversation:Option<Option<i64>>=self.connection.query_row("SELECT CASE WHEN a.projected_thread_revision=a.thread_revision THEN cm.conversation_id ELSE NULL END FROM messages m JOIN accounts a ON a.id=m.account_id LEFT JOIN conversation_members cm ON cm.account_id=m.account_id AND cm.message_id=m.id WHERE m.account_id=?1 AND m.id=?2",params![account,message],|row|row.get(0)).optional()?;
        let conversation = conversation.ok_or(Error::MissingMessage)?;
        let mut statement=self.connection.prepare_cached("SELECT m.id,m.account_id,m.subject,m.sender,m.preview,m.received_at,MIN(s.seen),MAX(s.starred),MAX(f.is_inbox) FROM messages m JOIN effective_memberships s ON s.account_id=m.account_id AND s.message_id=m.id JOIN folders f ON f.id=s.folder_id AND f.account_id=s.account_id AND f.uidvalidity=s.uidvalidity LEFT JOIN conversation_members cm ON cm.account_id=m.account_id AND cm.message_id=m.id WHERE m.account_id=?1 AND ((?3 IS NULL AND m.id=?2) OR (?3 IS NOT NULL AND cm.conversation_id=?3)) GROUP BY m.id ORDER BY CASE WHEN m.id=?2 THEN 0 ELSE 1 END,m.received_at DESC,m.id DESC LIMIT 200")?;
        let mut messages = statement
            .query_map(params![account, message, conversation], |row| {
                let mut message = super::mailbox::read_summary(row)?;
                message.inbox = row.get(8)?;
                Ok(message)
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        messages.sort_unstable_by_key(|message| (message.received_at, message.id));
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::mailbox::{Flags, Membership};
    use core::num::NonZeroU32;
    use mailstronaut_mail::imap::FlagChange;
    const CONTEXT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    fn setup() -> (tempfile::TempDir, Store, i64, i64, i64) {
        let root = tempfile::tempdir().expect("profile");
        let mut store = Store::open(root.path()).expect("store");
        for account in ["a", "b"] {
            store.ensure_account(account).expect("account");
            store
                .bind_mailbox_context(account, CONTEXT)
                .expect("context");
        }
        let inbox = store.ensure_folder("a", "INBOX", true).expect("inbox").id;
        let sent = store.ensure_folder("a", "Sent", false).expect("sent").id;
        let other = store
            .ensure_folder("b", "INBOX", true)
            .expect("other inbox")
            .id;
        for (account, folder) in [("a", inbox), ("a", sent), ("b", other)] {
            store
                .select_folder(account, folder, NonZeroU32::new(42).expect("namespace"))
                .expect("select");
        }
        (root, store, inbox, sent, other)
    }
    fn add(
        store: &mut Store,
        account: &str,
        folder: i64,
        uid: u32,
        id: &str,
        references: &[&str],
        date: i64,
    ) -> i64 {
        let references = references
            .iter()
            .map(|id| format!("<{id}>"))
            .collect::<Vec<_>>()
            .join(" ");
        let raw = format!(
            "From: sender@example.test\r\nTo: reader@example.test\r\nSubject: Same subject\r\nMessage-ID: <{id}>\r\nReferences: {references}\r\n\r\nSynthetic body {uid}.\r\n"
        );
        store
            .cache_message(
                account,
                Membership {
                    folder,
                    uid_validity: NonZeroU32::new(42).expect("generation"),
                    uid: NonZeroU32::new(uid).expect("UID"),
                    flags: Flags::default(),
                },
                date,
                None,
                raw.as_bytes(),
            )
            .expect("cache")
    }
    fn group(store: &mut Store, account: &str) {
        let snapshot = store
            .conversation_snapshot(account)
            .expect("snapshot")
            .expect("stale");
        let groups = crate::threads::project(&snapshot.messages).expect("synthetic RFC grouping");
        store
            .commit_conversations(&snapshot, &groups)
            .expect("projection commit");
    }
    fn identity(store: &Store, account: &str, message: i64) -> i64 {
        store.connection.query_row("SELECT conversation_id FROM conversation_members WHERE account_id=?1 AND message_id=?2",params![account,message],|row|row.get(0)).expect("conversation identity")
    }
    #[test]
    fn conversation_inbox_is_account_local_includes_sent_and_updates_flags_per_message() {
        let (_root, mut store, inbox, sent, other) = setup();
        let first = add(
            &mut store,
            "a",
            inbox,
            1,
            "first@example.test",
            &["parent@example.test"],
            10,
        );
        let second = add(
            &mut store,
            "a",
            inbox,
            2,
            "second@example.test",
            &["parent@example.test"],
            20,
        );
        let parent = add(&mut store, "a", sent, 3, "parent@example.test", &[], 30);
        add(
            &mut store,
            "b",
            other,
            1,
            "first@example.test",
            &["parent@example.test"],
            25,
        );
        group(&mut store, "a");
        group(&mut store, "b");
        let rows = store
            .conversation_inbox(&["a".into(), "b".into()], None, 100)
            .expect("inbox");
        assert_eq!(rows.len(), 2);
        let row = rows
            .iter()
            .find(|row| row.latest.account == "a")
            .expect("account a");
        assert_eq!(row.latest.id, second);
        assert_eq!(row.cached_messages, 3);
        assert_eq!(row.unread, 2);
        assert_eq!(
            store
                .conversation_messages("a", second)
                .expect("members")
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            [first, second, parent]
        );
        assert!(matches!(
            store.conversation_messages("b", second),
            Err(Error::MissingMessage)
        ));
        store
            .set_flag("a", second, FlagChange::Seen, true, CONTEXT)
            .expect("read selected member");
        let row = store
            .conversation_inbox(&["a".into()], None, 100)
            .expect("updated inbox")
            .remove(0);
        assert_eq!(row.unread, 1);
        assert!(row.latest.seen);
        assert!(
            !store.conversation_messages("a", second).expect("members")[0].seen,
            "reading one never reads its siblings"
        );
        assert!(
            store
                .conversation_snapshot("a")
                .expect("flags do not invalidate ancestry")
                .is_none()
        );
    }
    #[test]
    fn merges_keep_oldest_identity_and_aliases_survive_restart_and_later_merges() {
        let (root, mut store, inbox, _, _) = setup();
        let a = add(&mut store, "a", inbox, 1, "a@example.test", &[], 1);
        let b = add(&mut store, "a", inbox, 2, "b@example.test", &[], 2);
        let c = add(&mut store, "a", inbox, 3, "c@example.test", &[], 3);
        group(&mut store, "a");
        let ids = [
            identity(&store, "a", a),
            identity(&store, "a", b),
            identity(&store, "a", c),
        ];
        add(
            &mut store,
            "a",
            inbox,
            4,
            "bridge@example.test",
            &["a@example.test", "b@example.test"],
            4,
        );
        group(&mut store, "a");
        assert_eq!(
            store
                .resolve_conversation("a", ids[1])
                .expect("first alias"),
            ids[0]
        );
        add(
            &mut store,
            "a",
            inbox,
            5,
            "bridge-two@example.test",
            &["c@example.test", "a@example.test"],
            5,
        );
        group(&mut store, "a");
        drop(store);
        let store = Store::open(root.path()).expect("restart");
        for id in ids {
            assert_eq!(
                store.resolve_conversation("a", id).expect("stable alias"),
                ids[0]
            );
        }
        assert!(matches!(
            store.resolve_conversation("b", ids[0]),
            Err(Error::MissingMessage)
        ));
        assert_eq!(
            store
                .conversation_inbox(&["a".into()], None, 100)
                .expect("grouped")
                .len(),
            1
        );
    }
    #[test]
    fn late_results_and_incomplete_coverage_cannot_replace_newer_metadata() {
        let (_root, mut store, inbox, _, _) = setup();
        let first = add(&mut store, "a", inbox, 1, "first@example.test", &[], 1);
        let old = store
            .conversation_snapshot("a")
            .expect("snapshot")
            .expect("stale");
        let groups = crate::threads::project(&old.messages).expect("groups");
        add(
            &mut store,
            "a",
            inbox,
            2,
            "reply@example.test",
            &["first@example.test"],
            2,
        );
        assert!(matches!(
            store.commit_conversations(&old, &groups),
            Err(Error::OperationConflict)
        ));
        let current = store
            .conversation_snapshot("a")
            .expect("snapshot")
            .expect("stale");
        assert!(matches!(
            store.commit_conversations(
                &current,
                &Groups {
                    groups: vec![vec![first]]
                }
            ),
            Err(Error::InvalidInput)
        ));
        assert_eq!(
            store
                .conversation_inbox(&["a".into()], None, 100)
                .expect("honest flat fallback")
                .len(),
            2
        );
        group(&mut store, "a");
        assert_eq!(
            store
                .conversation_inbox(&["a".into()], None, 100)
                .expect("grouped")
                .len(),
            1
        );
    }
    #[test]
    fn a_bounded_member_page_always_contains_the_selected_message() {
        let (_root, mut store, inbox, _, _) = setup();
        let mut ids = Vec::new();
        for uid in 1..=205 {
            ids.push(add(
                &mut store,
                "a",
                inbox,
                uid,
                &format!("{uid}@large.test"),
                &["missing@large.test"],
                i64::from(uid),
            ));
        }
        group(&mut store, "a");
        let members = store.conversation_messages("a", ids[0]).expect("members");
        assert_eq!(members.len(), 200);
        assert_eq!(members.first().expect("selected member").id, ids[0]);
        assert_eq!(members.last().expect("latest member").id, ids[204]);
        assert!(!members.iter().any(|message| message.id == ids[1]));
        let row = store
            .conversation_inbox(&["a".into()], None, 100)
            .expect("inbox")
            .remove(0);
        assert_eq!(
            row.cached_messages, 205,
            "counts describe the complete cache"
        );
    }
    #[test]
    fn splitting_preserves_only_one_old_identity_and_deleted_roots_do_not_erase_surviving_thread_ids()
     {
        let (_root, mut store, inbox, _, _) = setup();
        let a = add(&mut store, "a", inbox, 1, "a@example.test", &[], 1);
        let b = add(
            &mut store,
            "a",
            inbox,
            2,
            "b@example.test",
            &["a@example.test"],
            2,
        );
        group(&mut store, "a");
        let old = identity(&store, "a", a);
        store
            .connection
            .execute(
                "DELETE FROM memberships WHERE account_id='a' AND message_id=?1",
                [a],
            )
            .expect("retire fixture memberships");
        store
            .connection
            .execute("DELETE FROM messages WHERE id=?1", [a])
            .expect("purge fixture root");
        group(&mut store, "a");
        assert_eq!(identity(&store, "a", b), old);
        let c = add(
            &mut store,
            "a",
            inbox,
            3,
            "c@example.test",
            &["b@example.test"],
            3,
        );
        group(&mut store, "a");
        store
            .connection
            .execute(
                "UPDATE messages SET thread_references='[]',thread_parent=NULL WHERE id=?1",
                [c],
            )
            .expect("changed ancestry fixture");
        group(&mut store, "a");
        assert_eq!(identity(&store, "a", b), old);
        assert_ne!(identity(&store, "a", c), old);
        assert_eq!(
            store
                .resolve_conversation("a", old)
                .expect("old identity stays valid"),
            old
        );
    }
}

#[cfg(test)]
mod identity_regression {
    use super::*;
    #[test]
    fn simultaneous_split_and_merge_reserve_old_ids_for_their_oldest_surviving_members() {
        let root = tempfile::tempdir().expect("profile");
        let mut store = Store::open(root.path()).expect("store");
        store.ensure_account("a").expect("account");
        let a = store.ingest_message("a", b"a").expect("a");
        let b = store.ingest_message("a", b"b").expect("b");
        let c = store.ingest_message("a", b"c").expect("c");
        let d = store.ingest_message("a", b"d").expect("d");
        let snapshot = store
            .conversation_snapshot("a")
            .expect("snapshot")
            .expect("stale");
        store
            .commit_conversations(
                &snapshot,
                &Groups {
                    groups: vec![vec![a, c, d], vec![b]],
                },
            )
            .expect("initial groups");
        let old_a: i64 = store
            .connection
            .query_row(
                "SELECT conversation_id FROM conversation_members WHERE message_id=?1",
                [a],
                |row| row.get(0),
            )
            .expect("a identity");
        let old_b: i64 = store
            .connection
            .query_row(
                "SELECT conversation_id FROM conversation_members WHERE message_id=?1",
                [b],
                |row| row.get(0),
            )
            .expect("b identity");
        store
            .connection
            .execute("DELETE FROM messages WHERE id=?1", [a])
            .expect("purge old anchor fixture");
        let snapshot = store
            .conversation_snapshot("a")
            .expect("snapshot")
            .expect("stale");
        store
            .commit_conversations(
                &snapshot,
                &Groups {
                    groups: vec![vec![b, d], vec![c]],
                },
            )
            .expect("simultaneous split/merge");
        let c_identity: i64 = store
            .connection
            .query_row(
                "SELECT conversation_id FROM conversation_members WHERE message_id=?1",
                [c],
                |row| row.get(0),
            )
            .expect("oldest surviving a member");
        let b_identity: i64 = store
            .connection
            .query_row(
                "SELECT conversation_id FROM conversation_members WHERE message_id=?1",
                [b],
                |row| row.get(0),
            )
            .expect("b member");
        assert_eq!(c_identity, old_a);
        assert_eq!(b_identity, old_b);
        assert_eq!(
            store
                .resolve_conversation("a", old_a)
                .expect("a still resolves"),
            old_a
        );
    }
}
