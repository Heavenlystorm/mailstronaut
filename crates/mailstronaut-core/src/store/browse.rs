//! Local account/folder navigation and literal full-text queries.

use super::{
    Error, Store,
    mailbox::{Cursor, Folder},
};
use rusqlite::params;

/// One explicit local scope. All reads remain restricted to enabled accounts.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Scope {
    /// Configured inbox memberships across enabled accounts.
    #[default]
    Inbox,
    /// All currently cached folder memberships in this account.
    Account(String),
    /// One account-bound opaque server mailbox identity.
    Folder {
        /// Enabled configuration account identity.
        account: String,
        /// Local folder identity belonging to that account.
        folder: i64,
    },
}
impl Scope {
    fn parts<'a>(
        &'a self,
        accounts: &[String],
    ) -> Result<(Option<&'a str>, Option<i64>, bool), Error> {
        let (account, folder, inbox) = match self {
            Self::Inbox => (None, None, true),
            Self::Account(account) => (Some(account.as_str()), None, false),
            Self::Folder { account, folder } if *folder > 0 => {
                (Some(account.as_str()), Some(*folder), false)
            }
            Self::Folder { .. } => return Err(Error::InvalidInput),
        };
        if account.is_some_and(|account| !accounts.iter().any(|enabled| enabled == account)) {
            return Err(Error::InvalidInput);
        }
        Ok((account, folder, inbox))
    }
}

/// Plain whitespace-separated terms, each quoted as an FTS5 phrase. Users do not
/// supply MATCH operators/column selectors. SQLite owns Unicode tokenization.
fn search_expression(input: &str) -> Result<Option<String>, Error> {
    if input.len() > 1024 || input.contains(['\0', '\r', '\n']) {
        return Err(Error::InvalidInput);
    }
    let terms = input.split_whitespace().collect::<Vec<_>>();
    if terms.len() > 32 {
        return Err(Error::InvalidInput);
    }
    Ok((!terms.is_empty()).then(|| {
        terms
            .iter()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ")
    }))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 4096 && !name.contains(['\0', '\r', '\n'])
}

impl Store {
    /// Remember selectable mailboxes from a successful authenticated LIST. Existing
    /// cached/removed folders are retained for offline use; only the configured
    /// inbox participates in the unified inbox after discovery.
    /// # Errors
    /// Rejects invalid/excessive names and storage failures. Caller binds the
    /// authenticated mailbox context first; this API never authenticates itself.
    pub fn remember_folders(
        &mut self,
        account: &str,
        names: &[String],
        inbox: &str,
    ) -> Result<(), Error> {
        if names.len() > 4096
            || names.iter().map(String::len).sum::<usize>() > 4 * 1024 * 1024
            || !valid_name(inbox)
            || names.iter().any(|name| !valid_name(name))
        {
            return Err(Error::InvalidInput);
        }
        let tx = self.connection.transaction()?;
        tx.execute(
            "UPDATE folders SET is_inbox=0 WHERE account_id=?1",
            [account],
        )?;
        for name in names {
            let is_inbox = name == inbox
                || (inbox.eq_ignore_ascii_case("INBOX") && name.eq_ignore_ascii_case("INBOX"));
            tx.execute("INSERT INTO folders(account_id,name,is_inbox) VALUES(?1,?2,?3) ON CONFLICT(account_id,name) DO UPDATE SET is_inbox=excluded.is_inbox",params![account,name,is_inbox])?;
        }
        tx.commit()?;
        Ok(())
    }
    /// Enumerate at most 4096 known folders, including discovered uncached ones.
    /// Wire names are display text only and never filesystem paths.
    /// # Errors
    /// Rejects excessive selection/results and storage failures.
    pub fn folders(&self, accounts: &[String]) -> Result<Vec<Folder>, Error> {
        if accounts.len() > 256 {
            return Err(Error::InvalidInput);
        }
        let mut query=self.connection.prepare_cached("SELECT id,account_id,name,uidvalidity,is_inbox,role FROM folders WHERE account_id IN (SELECT value FROM json_each(?1)) ORDER BY account_id,is_inbox DESC,CASE role WHEN 'sent' THEN 0 WHEN 'drafts' THEN 1 WHEN 'archive' THEN 2 WHEN 'junk' THEN 3 WHEN 'trash' THEN 4 ELSE 5 END,name LIMIT 4097")?;
        let rows = query
            .query_map(
                [serde_json::to_string(accounts)?],
                super::mailbox::read_folder,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        if rows.len() > 4096 {
            return Err(Error::InvalidInput);
        }
        Ok(rows)
    }

    /// Page local conversations in a configured inbox/account/folder scope.
    /// Search opens the newest matching member while counts describe the complete
    /// selected scope. Search never calls mail transports or accesses remote data.
    /// # Errors
    /// Rejects disabled accounts, foreign folders, query/page limits and storage errors.
    pub fn conversation_page(
        &self,
        accounts: &[String],
        scope: &Scope,
        query: &str,
        after: Option<Cursor>,
        limit: u32,
    ) -> Result<Vec<super::conversations::Summary>, Error> {
        if accounts.len() > 256 || !(1..=100).contains(&limit) {
            return Err(Error::InvalidInput);
        }
        let (account, folder, inbox_only) = scope.parts(accounts)?;
        if let Some(folder) = folder {
            let known: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM folders WHERE account_id=?1 AND id=?2)",
                params![account, folder],
                |row| row.get(0),
            )?;
            if !known {
                return Err(Error::InvalidInput);
            }
        }
        let search = search_expression(query)?;
        let selected = serde_json::to_string(accounts)?;
        let mut statement=self.connection.prepare_cached("
WITH inbox AS (
 SELECT m.id,m.account_id,m.subject,m.sender,m.preview,m.received_at,MIN(s.seen) AS seen,MAX(s.starred) AS starred,MAX(f.is_inbox) AS inbox,
 CASE WHEN ?7 IS NULL THEN 1 ELSE m.id IN (SELECT rowid FROM message_search WHERE message_search MATCH ?7) END AS matches
 FROM messages m JOIN effective_memberships s ON s.message_id=m.id AND s.account_id=m.account_id
 JOIN folders f ON f.id=s.folder_id AND f.account_id=s.account_id AND f.uidvalidity=s.uidvalidity
 WHERE m.account_id IN (SELECT value FROM json_each(?1))
 AND (?5 IS NULL OR m.account_id=?5) AND (?6 IS NULL OR f.id=?6)
 AND (?8=0 OR f.is_inbox=1) GROUP BY m.id
), tagged AS (
 SELECT i.*,CASE WHEN a.projected_thread_revision=a.thread_revision THEN cm.conversation_id ELSE NULL END AS conversation,
 CASE WHEN a.projected_thread_revision=a.thread_revision AND cm.conversation_id IS NOT NULL THEN cm.conversation_id ELSE -i.id END AS group_key
 FROM inbox i JOIN accounts a ON a.id=i.account_id LEFT JOIN conversation_members cm ON cm.account_id=i.account_id AND cm.message_id=i.id
), ranked AS (
 SELECT *,SUM(NOT seen) OVER(PARTITION BY account_id,group_key) AS unread_count,
 MAX(starred) OVER(PARTITION BY account_id,group_key) AS thread_starred,
 MAX(matches) OVER(PARTITION BY account_id,group_key) AS any_match,
 ROW_NUMBER() OVER(PARTITION BY account_id,group_key ORDER BY matches DESC,received_at DESC,id DESC) AS position FROM tagged
)
SELECT id,account_id,subject,sender,preview,received_at,seen,starred,conversation,
 CASE WHEN conversation IS NULL THEN 1 ELSE (
 SELECT COUNT(*) FROM conversation_members cm WHERE cm.account_id=r.account_id AND cm.conversation_id=r.conversation
 AND EXISTS(SELECT 1 FROM memberships s JOIN folders f ON f.id=s.folder_id AND f.account_id=s.account_id AND f.uidvalidity=s.uidvalidity WHERE s.account_id=cm.account_id AND s.message_id=cm.message_id)) END,
 unread_count,thread_starred,inbox FROM ranked r
WHERE position=1 AND any_match=1 AND (?2 IS NULL OR received_at<?2 OR (received_at=?2 AND id<?3))
ORDER BY received_at DESC,id DESC LIMIT ?4")?;
        statement
            .query_map(
                params![
                    selected,
                    after.map(|cursor| cursor.received_at),
                    after.map(|cursor| cursor.message_id),
                    limit,
                    account,
                    folder,
                    search,
                    inbox_only
                ],
                |row| {
                    Ok(super::conversations::Summary {
                        latest: {
                            let mut message = super::mailbox::read_summary(row)?;
                            message.inbox = row.get(12)?;
                            message
                        },
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        store::mailbox::{Flags, Membership},
        threads,
    };
    use core::num::NonZeroU32;
    fn cache(
        store: &mut Store,
        account: &str,
        folder: i64,
        uid: u32,
        parent: bool,
        body: &str,
    ) -> i64 {
        let raw = format!(
            "From: sender@example.test\r\nTo: reader@example.test\r\nSubject: Café dossier\r\nMessage-ID: <{uid}@{account}.test>\r\nReferences: <root@{account}.test>\r\n\r\n{body}"
        );
        store
            .cache_message(
                account,
                Membership {
                    folder,
                    uid_validity: NonZeroU32::new(42).expect("namespace"),
                    uid: NonZeroU32::new(uid).expect("UID"),
                    flags: Flags {
                        seen: parent,
                        starred: false,
                    },
                },
                i64::from(uid),
                None,
                raw.as_bytes(),
            )
            .expect("message")
    }
    fn project(store: &mut Store, account: &str) {
        let snapshot = store
            .conversation_snapshot(account)
            .expect("snapshot")
            .expect("stale");
        let groups = threads::project(&snapshot.messages).expect("synthetic grouping");
        store
            .commit_conversations(&snapshot, &groups)
            .expect("commit");
    }
    fn fixture() -> (tempfile::TempDir, Store, [i64; 4], [i64; 1]) {
        let root = tempfile::tempdir().expect("profile");
        let mut store = Store::open(root.path()).expect("store");
        for account in ["a", "b"] {
            store.ensure_account(account).expect("account");
        }
        let inbox = store.ensure_folder("a", "INBOX", true).expect("inbox").id;
        let sent = store.ensure_folder("a", "Sent", false).expect("sent").id;
        let other = store.ensure_folder("b", "INBOX", true).expect("other").id;
        for (account, folder) in [("a", inbox), ("a", sent), ("b", other)] {
            store
                .select_folder(account, folder, NonZeroU32::new(42).expect("namespace"))
                .expect("select");
        }
        let needle = cache(&mut store, "a", inbox, 1, false, "Needle questionnaire");
        let latest = cache(&mut store, "a", inbox, 2, false, "Later ordinary reply");
        let ancestor = cache(&mut store, "a", sent, 3, true, "Private sent marker");
        let foreign = cache(
            &mut store,
            "b",
            other,
            4,
            false,
            "Needle other-account marker",
        );
        project(&mut store, "a");
        project(&mut store, "b");
        (root, store, [needle, latest, ancestor, foreign], [sent])
    }
    #[test]
    fn local_search_opens_a_matching_member_and_respects_account_folder_and_namespace() {
        let (root, mut store, [needle, latest, ancestor, foreign], [sent]) = fixture();
        let accounts = vec!["a".into(), "b".into()];
        let results = store
            .conversation_page(&accounts, &Scope::Inbox, "needle", None, 100)
            .expect("search");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].latest.id, foreign);
        assert_eq!(results[1].latest.id, needle);
        assert_eq!(
            results[1].unread, 2,
            "counts include the nonmatching sibling"
        );
        assert_eq!(
            store
                .conversation_page(&accounts, &Scope::Inbox, "", None, 100)
                .expect("latest")[1]
                .latest
                .id,
            latest
        );
        let result = store
            .conversation_page(
                &accounts,
                &Scope::Account("a".into()),
                "sent marker",
                None,
                100,
            )
            .expect("account search");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].latest.id, ancestor);
        assert!(!result[0].latest.inbox);
        assert_eq!(
            store
                .conversation_page(&accounts, &Scope::Inbox, "sent marker", None, 100)
                .expect("inbox excludes Sent"),
            Vec::new()
        );
        let scope = Scope::Folder {
            account: "a".into(),
            folder: sent,
        };
        assert_eq!(
            store
                .conversation_page(&accounts, &scope, "cafe", None, 100)
                .expect("Unicode search")[0]
                .latest
                .id,
            ancestor
        );
        assert!(matches!(
            store.conversation_page(&["a".into()], &Scope::Account("b".into()), "", None, 100),
            Err(Error::InvalidInput)
        ));
        assert!(matches!(
            store.conversation_page(
                &accounts,
                &Scope::Folder {
                    account: "b".into(),
                    folder: sent
                },
                "",
                None,
                100
            ),
            Err(Error::InvalidInput)
        ));
        assert_eq!(
            store.folders(&["a".into()]).expect("known folders").len(),
            2
        );
        store
            .select_folder("a", sent, NonZeroU32::new(43).expect("changed namespace"))
            .expect("retire old Sent");
        assert_eq!(
            store
                .conversation_page(&accounts, &scope, "sent marker", None, 100)
                .expect("no stale membership"),
            Vec::new()
        );
        drop(store);
        let store = Store::open(root.path()).expect("offline restart");
        assert_eq!(
            store
                .conversation_page(&["a".into()], &Scope::Inbox, "questionnaire", None, 100)
                .expect("persistent index")[0]
                .latest
                .id,
            needle
        );
    }
    #[test]
    fn authenticated_catalog_keeps_wire_names_and_offline_history_but_replaces_the_inbox_role() {
        let (_root, mut store, _, [sent]) = fixture();
        let names = vec!["Custom inbox".into(), "&AMk-tudes".into(), "INBOX".into()];
        store
            .remember_folders("a", &names, "Custom inbox")
            .expect("catalog");
        let folders = store.folders(&["a".into(), "b".into()]).expect("folders");
        assert!(
            folders.iter().any(|folder| folder.account == "a"
                && folder.name == "Custom inbox"
                && folder.inbox)
        );
        assert!(
            folders
                .iter()
                .any(|folder| folder.account == "a" && folder.name == "&AMk-tudes")
        );
        assert!(folders.iter().any(|folder| folder.account == "a"
            && folder.id == sent
            && folder.uid_validity.is_some()));
        assert!(
            !folders
                .iter()
                .any(|folder| folder.account == "a" && folder.name == "INBOX" && folder.inbox)
        );
        assert!(
            folders
                .iter()
                .any(|folder| folder.account == "b" && folder.inbox)
        );
        assert!(
            store
                .remember_folders("a", &["new".into(), "bad\r\nSELECT".into()], "INBOX")
                .is_err()
        );
        assert_eq!(
            store
                .folders(&["a".into(), "b".into()])
                .expect("unchanged catalog"),
            folders
        );
    }
    #[test]
    fn query_text_cannot_supply_fts_operators_or_sql() {
        let (_root, store, _, _) = fixture();
        let accounts = vec!["a".into(), "b".into()];
        assert_eq!(
            store
                .conversation_page(&accounts, &Scope::Inbox, "needle OR missing", None, 100)
                .expect("literal OR"),
            Vec::new()
        );
        for query in [
            "\"",
            "*",
            "subject:needle",
            "needle) OR *",
            "' ; DELETE FROM messages; --",
        ] {
            let _literal = store
                .conversation_page(&accounts, &Scope::Inbox, query, None, 100)
                .expect("punctuation stays literal");
        }
    }
    #[test]
    fn local_search_is_bounded_and_pages_equal_dates_without_duplicates() {
        let root = tempfile::tempdir().expect("profile");
        let mut store = Store::open(root.path()).expect("store");
        store.ensure_account("a").expect("account");
        let folder = store.ensure_folder("a", "INBOX", true).expect("folder").id;
        store
            .select_folder("a", folder, NonZeroU32::new(42).expect("namespace"))
            .expect("select");
        for uid in 1..=12 {
            let id = cache(&mut store, "a", folder, uid, false, "Synthetic page");
            store
                .connection
                .execute(
                    "UPDATE messages SET thread_references='[]',received_at=1 WHERE id=?1",
                    [id],
                )
                .expect("independent messages with equal dates");
        }
        project(&mut store, "a");
        let mut after = None;
        let mut ids = Vec::new();
        loop {
            let page = store
                .conversation_page(&["a".into()], &Scope::Inbox, "synthetic", after, 3)
                .expect("page");
            if page.is_empty() {
                break;
            }
            after = page.last().map(|row| row.latest.cursor());
            ids.extend(page.iter().map(|row| row.latest.id));
        }
        assert_eq!(ids.len(), 12);
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 12);
        assert!(search_expression(&"x".repeat(1025)).is_err());
        assert!(search_expression(&"term ".repeat(33)).is_err());
        assert_eq!(search_expression(" \t ").expect("empty"), None);
    }
}
